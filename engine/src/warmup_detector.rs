/// Automatic warmup period detection from a compiled Rhai strategy.
///
/// Scans the AST for all `indicators::*` function calls and extracts the
/// period argument (the second argument after `candles`).  Returns
/// `max(all_periods) + 1` so the engine has one extra candle of history.
///
/// Handles:
/// - Integer literals:  `indicators::sma(candles, 30)`          → 30
/// - Constants:         `indicators::sma(candles, SLOW)`         → looks up SLOW in scope
/// - Nested constants:  `indicators::macd(candles, FAST, SLOW, 9)` → max(FAST, SLOW, 9)
///
/// Falls back to `DEFAULT_WARMUP` (200) when no indicators are found or
/// the period cannot be resolved (e.g. dynamic expressions).
use rhai::{AST, Expr, Scope};

/// Safe default: covers EMA(200), Ichimoku(52), MACD(26), etc.
pub const DEFAULT_WARMUP: usize = 200;

/// Detect the minimum number of historical candles required before this
/// strategy can produce meaningful signals.
pub fn detect_warmup_period(ast: &AST, scope: &Scope) -> usize {
    let mut max_period: usize = 0;
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
                && call.namespace.path.first()
                    .map(|seg| seg.name.as_str() == "indicators")
                    .unwrap_or(false);

            if !is_indicators {
                return true;
            }

            // Period is the second argument (index 1), after `candles`.
            // Some indicators take multiple period args (e.g. MACD: fast, slow, signal).
            // We scan all integer arguments starting from index 1.
            for arg in call.args.iter().skip(1) {
                if let Some(period) = resolve_period(arg, scope) {
                    found_any = true;
                    if period > max_period {
                        max_period = period;
                    }
                }
            }
        }

        true // continue walking
    });

    if found_any && max_period > 0 {
        max_period + 1
    } else {
        DEFAULT_WARMUP
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
            scope.get_value::<i64>(var_name.as_str())
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
    use rhai::Engine as RhaiEngine;
    use crate::{bindings::register_all, candle_wrapper::register_types};

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
    fn sma_cross_strategy_detects_slow_period() {
        let src = std::fs::read_to_string("../strategies/sma_cross.rhai").unwrap();
        let (ast, scope) = compile(&src);
        // Strategy currently uses FAST=5, SLOW=20 → max+1 = 21
        assert_eq!(detect_warmup_period(&ast, &scope), 21);
    }
}
