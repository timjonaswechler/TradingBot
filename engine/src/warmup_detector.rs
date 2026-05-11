/// Automatic warmup period detection from a compiled Rhai strategy.
///
/// Scans the AST for all `indicators::*` function calls and derives a warmup
/// hint per call. For classic period-based indicators, this usually means the
/// period argument after `candles`. For builtins like `ichimoku`, `obv`,
/// `vwap`, `volume_profile`, `pivot_points`, `fibonacci`, and `sar`, the warmup
/// comes from indicator-specific rules instead of blindly scanning numeric
/// arguments.
///
/// Returns `max(all_hints) + 1` so the engine has one extra candle of history.
///
/// Falls back to `DEFAULT_WARMUP` (200) when no indicators are found or
/// the warmup cannot be resolved.
use rhai::{Expr, FnCallExpr, Scope, AST};

/// Safe default: covers EMA(200), Ichimoku(52), MACD(26), etc.
pub const DEFAULT_WARMUP: usize = 200;

/// Detect the minimum number of historical candles required before this
/// strategy can produce meaningful signals.
pub fn detect_warmup_period(ast: &AST, scope: &Scope) -> usize {
    let mut max_hint: usize = 0;
    let mut found_any = false;

    ast.walk(&mut |nodes| {
        // We only care about the leaf expression nodes.
        let node = match nodes.last() {
            Some(n) => n,
            None => return true,
        };

        let expr = match node {
            rhai::ASTNode::Expr(e) => e,
            _ => return true,
        };

        if let Expr::FnCall(call, _) = expr {
            // Only care about qualified calls: indicators::xxx(...)
            let is_indicators = !call.namespace.is_empty()
                && call
                    .namespace
                    .path
                    .first()
                    .map(|seg| seg.name.as_str() == "indicators")
                    .unwrap_or(false);

            if !is_indicators {
                return true;
            }

            if let Some(hint) = warmup_hint_for_call(call, scope) {
                found_any = true;
                if hint > max_hint {
                    max_hint = hint;
                }
            }
        }

        true // continue walking
    });

    if found_any {
        max_hint + 1
    } else {
        DEFAULT_WARMUP
    }
}

fn warmup_hint_for_call(call: &FnCallExpr, scope: &Scope) -> Option<usize> {
    fn arg(call: &FnCallExpr, index: usize, scope: &Scope) -> Option<usize> {
        call.args
            .get(index)
            .and_then(|expr| resolve_period(expr, scope))
    }

    match call.name.as_str() {
        // Single-period indicators
        "sma" | "ema" | "dema" | "tema" | "adx" | "rsi" | "cci" | "stochastic" | "williams_r"
        | "roc" | "atr" | "mfi" | "slope" | "bollinger" | "keltner" => arg(call, 1, scope),

        // Multi-period indicator
        "macd" => [1usize, 2, 3]
            .into_iter()
            .filter_map(|i| arg(call, i, scope))
            .max(),

        // Built-in warmup hints for indicators without meaningful integer period args
        "ichimoku" => Some(52),
        "sar" | "obv" => Some(1),
        "vwap" | "volume_profile" | "pivot_points" | "fibonacci" => Some(0),

        // Conservative fallback for future indicators: scan integer args after `candles`.
        _ => call
            .args
            .iter()
            .skip(1)
            .filter_map(|expr| resolve_period(expr, scope))
            .max(),
    }
}

/// Try to resolve a Rhai expression to a `usize` period value.
///
/// Handles:
/// - Integer constants: `14`, `26`
/// - Variables resolved from the scope: `SLOW`, `FAST`
fn resolve_period(expr: &Expr, scope: &Scope) -> Option<usize> {
    match expr {
        // Direct integer literal: indicators::sma(candles, 30)
        Expr::IntegerConstant(n, _) => {
            if *n > 0 {
                Some(*n as usize)
            } else {
                None
            }
        }

        // Variable reference: indicators::sma(candles, SLOW)
        // Look up the value in the scope (populated by top-level const declarations).
        // Variable tuple: (optional_index, name, namespace, hash)
        Expr::Variable(info, _, _) => {
            let var_name = &info.1;
            scope
                .get_value::<i64>(var_name.as_str())
                .filter(|&v| v > 0)
                .map(|v| v as usize)
        }

        _ => None,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bindings::register_all, candle_wrapper::register_types};
    use rhai::Engine as RhaiEngine;

    fn compile(src: &str) -> (AST, Scope<'static>) {
        let mut engine = RhaiEngine::new();
        register_types(&mut engine);
        register_all(&mut engine);
        let ast = engine.compile(src).unwrap();
        let mut scope = Scope::new();
        engine.run_ast_with_scope(&mut scope, &ast).unwrap();
        (ast, scope)
    }

    #[test]
    fn detects_literal_period() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::sma(candles, 30);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 31);
    }

    #[test]
    fn detects_constant_period() {
        let src = r#"
            const SLOW = 30;
            const FAST = 10;
            fn on_tick(candles, context) {
                let s = indicators::sma(candles, SLOW);
                let f = indicators::sma(candles, FAST);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        // max(30, 10) + 1 = 31
        assert_eq!(detect_warmup_period(&ast, &scope), 31);
    }

    #[test]
    fn detects_multiple_indicators() {
        let src = r#"
            const MACD_SLOW = 26;
            fn on_tick(candles, context) {
                let m = indicators::macd(candles, 12, MACD_SLOW, 9);
                let r = indicators::rsi(candles, 14);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        // max(12, 26, 9, 14) + 1 = 27
        assert_eq!(detect_warmup_period(&ast, &scope), 27);
    }

    #[test]
    fn falls_back_to_default_when_no_indicators() {
        let src = r#"
            fn on_tick(candles, context) {
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), DEFAULT_WARMUP);
    }

    #[test]
    fn detects_ichimoku_builtin_warmup() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::ichimoku(candles);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 53);
    }

    #[test]
    fn detects_sar_builtin_warmup_without_numeric_period_args() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::sar(candles, 0.02, 0.2);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 2);
    }

    #[test]
    fn detects_obv_builtin_warmup() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::obv(candles);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 2);
    }

    #[test]
    fn ignores_non_warmup_numeric_params_for_volume_profile() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::volume_profile(candles, 50);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 1);
    }

    #[test]
    fn detects_stateless_fibonacci_without_falling_back_to_default() {
        let src = r#"
            fn on_tick(candles, context) {
                let x = indicators::fibonacci(candles, 90.0, 110.0);
                #{ signal: "HOLD" }
            }
        "#;
        let (ast, scope) = compile(src);
        assert_eq!(detect_warmup_period(&ast, &scope), 1);
    }

    #[test]
    fn sma_cross_strategy_detects_slow_period() {
        let src = std::fs::read_to_string("../strategies/sma_cross.rhai").unwrap();
        let (ast, scope) = compile(&src);
        // Strategy currently uses FAST=50, SLOW=200 → max+1 = 201
        assert_eq!(detect_warmup_period(&ast, &scope), 201);
    }
}
