//! Warmup detection and planning for Trading Runtime strategy handling.

use crate::{RuntimeConfig, StrategyConfiguration};
use domain::Timeframe;
use rhai::{Expr, FnCallExpr, Scope, AST};
use std::collections::HashMap;

/// Detect the automatic warmup requirement implied by typed Rhai indicator calls.
///
/// This scans the compiled strategy AST for canonical `ta::*` calls and
/// transitional `indicators::*` calls whose first argument is the Market View
/// candle-history API: `market.candles()` or `market.candles(tf)`. The returned
/// value includes one extra candle of history
/// for indicators with a detected period, matching the donor detector's rule.
/// When no relevant indicator call is found, this returns `0` so the runtime
/// minimum/default policy remains authoritative.
pub fn detect_auto_warmup(ast: &AST, scope: &Scope<'_>) -> usize {
    let mut max_hint: usize = 0;
    let mut found_any = false;

    ast.walk(&mut |nodes| {
        let Some(rhai::ASTNode::Expr(Expr::FnCall(call, _))) = nodes.last() else {
            return true;
        };

        if !is_indicator_namespace_call(call) || !first_arg_is_market_candles(call) {
            return true;
        }

        if let Some(hint) = warmup_hint_for_call(call, scope) {
            found_any = true;
            max_hint = max_hint.max(hint);
        }

        true
    });

    if found_any {
        max_hint + 1
    } else {
        0
    }
}

/// Resolve the effective v1 warmup count.
pub fn resolve_effective_warmup(
    auto_detected_warmup: usize,
    strategy_config_minimum_warmup: usize,
    runtime_minimum_warmup: usize,
) -> usize {
    auto_detected_warmup
        .max(strategy_config_minimum_warmup)
        .max(runtime_minimum_warmup)
}

/// Per-timeframe-capable warmup plan used by the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WarmupPlan {
    requirements_by_timeframe: HashMap<Timeframe, usize>,
}

impl WarmupPlan {
    pub fn new(requirements_by_timeframe: HashMap<Timeframe, usize>) -> Self {
        Self {
            requirements_by_timeframe,
        }
    }

    /// Build the v1 plan shape: the same resolved requirement assigned to every
    /// configured timeframe.
    pub fn same_requirement(config: &RuntimeConfig, requirement: usize) -> Self {
        Self::new(
            config
                .configured_timeframes()
                .into_iter()
                .map(|timeframe| (timeframe, requirement))
                .collect(),
        )
    }

    pub fn requirement_for(&self, timeframe: Timeframe) -> Option<usize> {
        self.requirements_by_timeframe.get(&timeframe).copied()
    }

    pub fn effective_requirement(&self) -> usize {
        self.requirements_by_timeframe
            .values()
            .copied()
            .max()
            .unwrap_or(0)
    }

    pub fn requirements_by_timeframe(&self) -> &HashMap<Timeframe, usize> {
        &self.requirements_by_timeframe
    }
}

/// Resolve a v1 warmup plan for a strategy/run combination.
///
/// V1 assigns the same global effective count to every configured timeframe,
/// while keeping the result keyed by timeframe for future per-timeframe rules.
pub fn resolve_warmup_plan(
    config: &RuntimeConfig,
    strategy_config: &StrategyConfiguration,
    ast: &AST,
    scope: &Scope<'_>,
    runtime_minimum_warmup: usize,
) -> WarmupPlan {
    let effective_warmup = resolve_effective_warmup(
        detect_auto_warmup(ast, scope),
        strategy_config.minimum_warmup(),
        runtime_minimum_warmup,
    );

    WarmupPlan::same_requirement(config, effective_warmup)
}

fn is_indicator_namespace_call(call: &FnCallExpr) -> bool {
    !call.namespace.is_empty()
        && call
            .namespace
            .path
            .first()
            .map(|segment| matches!(segment.name.as_str(), "ta" | "indicators"))
            .unwrap_or(false)
}

fn first_arg_is_market_candles(call: &FnCallExpr) -> bool {
    call.args
        .first()
        .map(is_market_candles_expression)
        .unwrap_or(false)
}

fn is_market_candles_expression(expr: &Expr) -> bool {
    match expr {
        Expr::Dot(binary, _, _) => {
            matches!(&binary.lhs, Expr::Variable(info, _, _) if info.1.as_str() == "market")
                && matches!(&binary.rhs, Expr::MethodCall(method, _) if method.name.as_str() == "candles" && method.args.len() <= 1)
        }
        _ => false,
    }
}

fn warmup_hint_for_call(call: &FnCallExpr, scope: &Scope<'_>) -> Option<usize> {
    fn arg(call: &FnCallExpr, index: usize, scope: &Scope<'_>) -> Option<usize> {
        call.args
            .get(index)
            .and_then(|expr| resolve_period(expr, scope))
    }

    match call.name.as_str() {
        // Single declared-period indicators. The detector's final +1 supplies
        // the donor warmup margin and the extra candle needed by change/TR
        // indicators such as RSI, ROC, ATR, and MFI.
        "sma" | "ema" | "adx" | "rsi" | "cci" | "williams_r" | "roc" | "atr" | "mfi" | "slope"
        | "bollinger" | "keltner" => arg(call, 1, scope),

        // DEMA/TEMA apply EMA repeatedly, so their minimum history is larger
        // than one simple period before the detector's extra candle is added.
        "dema" => arg(call, 1, scope).map(dema_history_requirement),
        "tema" => arg(call, 1, scope).map(tema_history_requirement),

        // Stochastic variants.
        "stochastic_fast" => arg(call, 1, scope),
        "stochastic_slow" => arg(call, 1, scope).map(|period| period + 3),
        "stochastic_full" => [1usize, 2, 3]
            .into_iter()
            .filter_map(|index| arg(call, index, scope))
            .max(),

        // Multi-period indicators.
        "macd" => [1usize, 2, 3]
            .into_iter()
            .filter_map(|index| arg(call, index, scope))
            .max(),

        // Indicator-specific donor rules where numeric arguments are not
        // warmup periods.
        "ichimoku" => Some(52),
        "sar" | "obv" => Some(1),
        "vwap" | "volume_profile" | "pivot_points" | "fibonacci" => Some(0),

        // Conservative fallback for future period-based indicators.
        _ => call
            .args
            .iter()
            .skip(1)
            .filter_map(|expr| resolve_period(expr, scope))
            .max(),
    }
}

fn dema_history_requirement(period: usize) -> usize {
    period.saturating_mul(2).saturating_sub(1)
}

fn tema_history_requirement(period: usize) -> usize {
    period.saturating_mul(3).saturating_sub(2)
}

fn resolve_period(expr: &Expr, scope: &Scope<'_>) -> Option<usize> {
    match expr {
        Expr::IntegerConstant(value, _) => (*value > 0).then_some(*value as usize),
        Expr::Variable(info, _, _) => scope
            .get_value::<i64>(info.1.as_str())
            .filter(|value| *value > 0)
            .map(|value| value as usize),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        MarketInput, PortfolioState, PredeterminedStrategyHandler, RhaiStrategy, RuntimeConfig,
        RuntimeEvent, SecondaryTimeframeConfig, StrategyDecision, Timeframe, TradingRuntime,
    };
    use domain::Candle;

    fn load(source: &str) -> RhaiStrategy {
        RhaiStrategy::load(source).expect("strategy should load")
    }

    fn source_with_on_tick(body: &str) -> String {
        format!(
            r#"
fn strategy_config() {{
    strategy_config::new().with_primary(timeframe("1m"))
}}

fn on_tick(market, context) {{
    {body}
}}
"#
        )
    }

    fn candle(timeframe: Timeframe, close: f64, timestamp: i64) -> Candle {
        Candle {
            timestamp,
            symbol: "BTC-USD".to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            timeframe,
        }
    }

    #[test]
    fn detects_sma_period_from_primary_market_candles() {
        let source = source_with_on_tick(
            r#"
let slow = indicators::sma(market.candles(), 50);
decision::hold()
"#,
        );
        let strategy = load(&source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 51);
    }

    #[test]
    fn detects_ta_periods_from_primary_market_candles_for_literals_and_constants() {
        let source = r#"
const SLOW = 50;
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    let fast = ta::sma(market.candles(), 20);
    let slow = ta::sma(market.candles(), SLOW);
    decision::hold()
}
"#;
        let strategy = load(source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 51);
    }

    #[test]
    fn resolves_top_level_constants_used_as_periods() {
        let source = r#"
const SLOW = 50;
const FAST = 10;
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    let slow = indicators::sma(market.candles(), SLOW);
    let fast = indicators::sma(market.candles(), FAST);
    decision::hold()
}
"#;
        let strategy = load(source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 51);
    }

    #[test]
    fn detects_multiple_indicators_by_max_requirement() {
        let source = r#"
const MACD_SLOW = 26;
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    let macd = indicators::macd(market.candles(), 12, MACD_SLOW, 9);
    let rsi = indicators::rsi(market.candles(), 14);
    decision::hold()
}
"#;
        let strategy = load(source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 27);
    }

    #[test]
    fn detects_indicator_call_nested_inside_method_call_argument() {
        let source = source_with_on_tick(
            r#"
context.state.set("last_rsi", indicators::rsi(market.candles(), 14));
decision::hold()
"#,
        );
        let strategy = load(&source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 15);
    }

    #[test]
    fn detects_v1_scalar_indicator_pack_names() {
        for (call, expected) in [
            ("indicators::sma(market.candles(), 7)", 8),
            ("indicators::ema(market.candles(), 7)", 8),
            ("indicators::dema(market.candles(), 7)", 14),
            ("indicators::tema(market.candles(), 7)", 20),
            ("indicators::slope(market.candles(), 7)", 8),
            ("indicators::rsi(market.candles(), 7)", 8),
            ("indicators::roc(market.candles(), 7)", 8),
            ("indicators::cci(market.candles(), 7)", 8),
            ("indicators::williams_r(market.candles(), 7)", 8),
            ("indicators::atr(market.candles(), 7)", 8),
            ("indicators::mfi(market.candles(), 7)", 8),
            ("indicators::obv(market.candles())", 2),
        ] {
            let source = source_with_on_tick(&format!(
                r#"
let value = {call};
decision::hold()
"#
            ));
            let strategy = load(&source);

            assert_eq!(
                detect_auto_warmup(strategy.ast(), strategy.scope()),
                expected,
                "{call}"
            );
        }
    }

    #[test]
    fn detects_dema_and_tema_expanded_history_requirements() {
        let source = source_with_on_tick(
            r#"
let dema_value = indicators::dema(market.candles(), 20);
let tema_value = indicators::tema(market.candles(), 20);
decision::hold()
"#,
        );
        let strategy = load(&source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 59);
    }

    #[test]
    fn detects_secondary_market_candles_indicator_period() {
        let source = r#"
const H1 = timeframe("1h");
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1))
}

fn on_tick(market, context) {
    let h1_slow = indicators::sma(market.candles(H1), 80);
    decision::hold()
}
"#;
        let strategy = load(source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 81);
    }

    #[test]
    fn no_relevant_indicators_returns_zero_so_runtime_minimum_can_win() {
        let source = source_with_on_tick("decision::hold()");
        let strategy = load(&source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 0);
    }

    #[test]
    fn old_candles_argument_shape_is_not_detected() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    let old_shape = indicators::sma(candles, 200);
    decision::hold()
}
"#;
        let strategy = load(source);

        assert_eq!(detect_auto_warmup(strategy.ast(), strategy.scope()), 0);
    }

    #[test]
    fn strategy_config_minimum_warmup_wins_over_detected_and_runtime_minimum() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(100)
}

fn on_tick(market, context) {
    let slow = indicators::sma(market.candles(), 30);
    decision::hold()
}
"#;
        let strategy = load(source);
        let config = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1));
        let plan = resolve_warmup_plan(
            &config,
            strategy.strategy_config(),
            strategy.ast(),
            strategy.scope(),
            10,
        );

        assert_eq!(plan.requirement_for(Timeframe::minutes(1)), Some(100));
    }

    #[test]
    fn runtime_minimum_warmup_wins_over_detected_and_strategy_minimum() {
        let source = source_with_on_tick(
            r#"
let slow = indicators::sma(market.candles(), 30);
decision::hold()
"#,
        );
        let strategy = load(&source);
        let config = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1));
        let plan = resolve_warmup_plan(
            &config,
            strategy.strategy_config(),
            strategy.ast(),
            strategy.scope(),
            120,
        );

        assert_eq!(plan.requirement_for(Timeframe::minutes(1)), Some(120));
    }

    #[test]
    fn strategy_config_cannot_lower_detected_warmup() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(10)
}

fn on_tick(market, context) {
    let slow = indicators::sma(market.candles(), 80);
    decision::hold()
}
"#;
        let strategy = load(source);
        let config = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1));
        let plan = resolve_warmup_plan(
            &config,
            strategy.strategy_config(),
            strategy.ast(),
            strategy.scope(),
            0,
        );

        assert_eq!(plan.requirement_for(Timeframe::minutes(1)), Some(81));
    }

    #[test]
    fn v1_plan_assigns_effective_requirement_to_every_configured_timeframe() {
        let source = r#"
const H1 = timeframe("1h");
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1))
}

fn on_tick(market, context) {
    let slow = indicators::sma(market.candles(H1), 80);
    decision::hold()
}
"#;
        let strategy = load(source);
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let config = RuntimeConfig::with_secondary_configs(
            "BTC-USD",
            primary,
            [SecondaryTimeframeConfig::required(secondary, 0)],
        );
        let plan = resolve_warmup_plan(
            &config,
            strategy.strategy_config(),
            strategy.ast(),
            strategy.scope(),
            10,
        );

        assert_eq!(plan.requirement_for(primary), Some(81));
        assert_eq!(plan.requirement_for(secondary), Some(81));
    }

    #[test]
    fn runtime_uses_warmup_plan_requirements_by_timeframe() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let config = RuntimeConfig::with_secondary_configs(
            "BTC-USD",
            primary,
            [SecondaryTimeframeConfig::required(secondary, 0)],
        );
        let plan = WarmupPlan::new(HashMap::from([(primary, 1), (secondary, 2)]));
        let strategy = PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]);
        let mut runtime =
            TradingRuntime::with_warmup_plan(config, PortfolioState::new(1_000.0), plan, strategy);

        let primary_step = runtime
            .on_market_input(MarketInput::WarmupCandle(candle(primary, 100.0, 60_000)))
            .expect("primary warmup should be accepted");
        assert!(!primary_step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));

        let first_secondary_step = runtime
            .on_market_input(MarketInput::WarmupCandle(candle(
                secondary, 100.0, 3_600_000,
            )))
            .expect("secondary warmup should be accepted");
        assert!(!first_secondary_step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));

        let second_secondary_step = runtime
            .on_market_input(MarketInput::WarmupCandle(candle(
                secondary, 101.0, 7_200_000,
            )))
            .expect("secondary warmup should be accepted");
        assert!(second_secondary_step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
    }
}
