use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, RwLock};

use rhai::{Dynamic, Engine as RhaiEngine, Scope, AST};
use shared::{Candle, Context, Signal, TradeDecision};

use crate::{
    anchored::{AnchoredOutputs, AnchoredRuntime, AnchoredSpec},
    bindings::register_all,
    candle_wrapper::{register_types, CandleList, ContextWrapper},
    error::EngineError,
    indicator_cache::IndicatorCache,
};

// ── Engine ───────────────────────────────────────────────────────────────────

/// A long-lived Rhai scripting engine that runs a single strategy.
///
/// - **Live trading** (daemon): one `Engine` per strategy, kept running for
///   the entire session. The O(1) indicator cache warms up over time.
/// - **Backtesting** (UI): one fresh `Engine` per backtest run; call `tick`
///   for every historical candle in chronological order.
pub struct Engine {
    rhai: RhaiEngine,
    ast: AST,
    scope: Scope<'static>,
    candles: Arc<RwLock<Vec<Candle>>>,
    cache: Arc<RwLock<IndicatorCache>>,
    anchored: Option<AnchoredRuntime>,
    state: Arc<RwLock<HashMap<String, Dynamic>>>,
}

impl Engine {
    /// Create a new engine and compile the strategy source.
    pub fn new(strategy_src: &str) -> Result<Self, EngineError> {
        let mut rhai = RhaiEngine::new();

        // Register all custom types and the indicators:: module.
        register_types(&mut rhai);
        register_all(&mut rhai);

        // Compile the strategy — catches syntax errors early.
        let ast = rhai
            .compile(strategy_src)
            .map_err(|e| EngineError::Strategy(format!("strategy compile error: {e}")))?;

        // Execute top-level code (constants, etc.) into a persistent scope.
        let mut scope = Scope::new();
        rhai.run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| EngineError::Strategy(format!("strategy init error: {e}")))?;

        // Verify on_tick is defined by scanning the AST.
        let has_on_tick = ast.iter_functions().any(|f| f.name == "on_tick");
        if !has_on_tick {
            return Err(EngineError::Strategy(
                "strategy must define `fn on_tick(candles, context)`".into(),
            ));
        }

        // Optional: fn anchored_config() — build the AnchoredRuntime.
        let anchored = if ast
            .iter_functions()
            .any(|f| f.name == "anchored_config" && f.params.is_empty())
        {
            let result: Dynamic = rhai
                .call_fn(&mut scope, &ast, "anchored_config", ())
                .map_err(|e| EngineError::Strategy(format!("anchored_config error: {e}")))?;
            let map = result
                .try_cast::<rhai::Map>()
                .ok_or_else(|| EngineError::Strategy("anchored_config must return a map".into()))?;
            let spec = AnchoredSpec::from_rhai_map(map)?;
            if spec.is_empty() {
                None
            } else {
                Some(AnchoredRuntime::from_spec(&spec)?)
            }
        } else {
            None
        };

        let candles = Arc::new(RwLock::new(Vec::new()));
        let cache = Arc::new(RwLock::new(IndicatorCache::new()));
        let state = Arc::new(RwLock::new(HashMap::new()));

        Ok(Self {
            rhai,
            ast,
            scope,
            candles,
            cache,
            anchored,
            state,
        })
    }

    /// Feed one candle into the engine and get a trading decision back.
    pub fn tick(&mut self, candle: Candle, ctx: Context) -> Result<TradeDecision, EngineError> {
        // Append the new candle.
        self.candles.write().unwrap().push(candle.clone());

        // Run the anchored pipeline (if any) on the new bar.
        let outputs = if let Some(rt) = self.anchored.as_mut() {
            let buf = self.candles.read().unwrap();
            let bar = buf.len() as u64 - 1;
            rt.tick(&candle, bar, &buf);
            Arc::new(rt.outputs().clone())
        } else {
            Arc::new(AnchoredOutputs::default())
        };

        // Build Rhai arguments — cheap Arc clones, no candle data copied.
        let candle_list = CandleList::new(Arc::clone(&self.candles), Arc::clone(&self.cache));
        let ctx_wrapper = ContextWrapper::new(ctx, outputs, candle.close, Arc::clone(&self.state));

        // Call on_tick(candles, context).
        let result: Dynamic = self
            .rhai
            .call_fn(
                &mut self.scope,
                &self.ast,
                "on_tick",
                (candle_list, ctx_wrapper),
            )
            .map_err(|e| EngineError::Strategy(format!("on_tick error: {e}")))?;

        parse_trade_decision(result)
    }

    /// Number of candles currently in the engine's history.
    pub fn candle_count(&self) -> usize {
        self.candles.read().unwrap().len()
    }

    /// Push a historical candle without triggering `on_tick`.
    /// Used by the warmup module to pre-load history.
    pub fn push_candle(&mut self, candle: Candle) {
        self.candles.write().unwrap().push(candle);
    }

    /// Compiled AST of the strategy — used by `detect_warmup_period`.
    pub fn ast(&self) -> &AST {
        &self.ast
    }

    /// Scope populated by top-level `const` / `let` declarations — used by
    /// `detect_warmup_period` to resolve period constants.
    pub fn scope(&self) -> &Scope<'static> {
        &self.scope
    }
}

// ── Parse helper ─────────────────────────────────────────────────────────────

fn parse_trade_decision(result: Dynamic) -> Result<TradeDecision, EngineError> {
    // on_tick must return an object map: #{ signal: "BUY", size: 0.5, ... }
    let map = result.try_cast::<rhai::Map>().ok_or_else(|| {
        EngineError::Strategy(
            "on_tick must return an object map, e.g. #{ signal: \"HOLD\" }".into(),
        )
    })?;

    // signal — required
    let signal_str = map
        .get("signal")
        .and_then(|v| v.clone().try_cast::<String>())
        .ok_or_else(|| EngineError::Strategy("missing or invalid `signal` key".into()))?;

    let signal = Signal::from_str(&signal_str).map_err(|e| EngineError::InvalidSignal(e))?;

    // size — optional; defaults to 1.0 for directional signals, 0.0 for HOLD
    let default_size = match signal {
        Signal::Hold => 0.0,
        _ => 1.0,
    };
    let size = map
        .get("size")
        .and_then(|v| v.clone().try_cast::<f64>())
        .unwrap_or(default_size);

    // Optional fields
    let stop_loss = map
        .get("stop_loss")
        .and_then(|v| v.clone().try_cast::<f64>());
    let take_profit = map
        .get("take_profit")
        .and_then(|v| v.clone().try_cast::<f64>());
    let reason = map
        .get("reason")
        .and_then(|v| v.clone().try_cast::<String>());

    Ok(TradeDecision {
        signal,
        size,
        stop_loss,
        take_profit,
        reason,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PositionSide;

    fn make_candle(close: f64, ts: i64) -> Candle {
        Candle {
            timestamp: ts,
            symbol: "TEST".into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            timeframe: "1d".into(),
        }
    }

    fn flat_ctx() -> Context {
        Context::new(10_000.0)
    }

    // ── Strategy: always HOLD ─────────────────────────────────────────────

    const HOLD: &str = r#"
fn on_tick(candles, context) {
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn engine_loads_and_ticks() {
        let mut e = Engine::new(HOLD).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    #[test]
    fn candle_count_grows() {
        let mut e = Engine::new(HOLD).unwrap();
        e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        e.tick(make_candle(101.0, 2), flat_ctx()).unwrap();
        assert_eq!(e.candle_count(), 2);
    }

    // ── Candle field access ───────────────────────────────────────────────

    const ACCESS: &str = r#"
fn on_tick(candles, context) {
    let c = candles[1];
    if c == () { return #{ signal: "HOLD" }; }
    if c.close > 100.0 {
        return #{ signal: "BUY", size: 0.5, reason: "above 100" };
    }
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn strategy_reads_candle_fields() {
        let mut e = Engine::new(ACCESS).unwrap();
        let d = e.tick(make_candle(105.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
        assert_eq!(d.size, 0.5);
        assert_eq!(d.reason.as_deref(), Some("above 100"));
    }

    #[test]
    fn strategy_hold_below_threshold() {
        let mut e = Engine::new(ACCESS).unwrap();
        let d = e.tick(make_candle(99.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    // ── Context access ────────────────────────────────────────────────────

    const CTX: &str = r#"
fn on_tick(candles, context) {
    if context.balance > 5000.0 { return #{ signal: "BUY" }; }
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn strategy_reads_context() {
        let mut e = Engine::new(CTX).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── SMA indicator ─────────────────────────────────────────────────────

    const SMA: &str = r#"
fn on_tick(candles, context) {
    let s = indicators::sma(candles, 3);
    if s == () { return #{ signal: "HOLD", reason: "warming up" }; }
    if candles[1].close > s { return #{ signal: "BUY" }; }
    #{ signal: "SELL" }
}
"#;

    #[test]
    fn sma_nil_during_warmup() {
        let mut e = Engine::new(SMA).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    #[test]
    fn sma_works_after_warmup() {
        let mut e = Engine::new(SMA).unwrap();
        e.tick(make_candle(10.0, 1), flat_ctx()).unwrap();
        e.tick(make_candle(10.0, 2), flat_ctx()).unwrap();
        // SMA(3)=(10+10+20)/3=13.33, close=20 > SMA → BUY
        let d = e.tick(make_candle(20.0, 3), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── candles.closes() helper ───────────────────────────────────────────

    const CLOSES: &str = r#"
fn on_tick(candles, context) {
    let cls = candles.closes();
    if cls.len() == 0 { return #{ signal: "HOLD" }; }
    if cls[0] > 50.0 { return #{ signal: "BUY" }; }
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn candle_helper_closes() {
        let mut e = Engine::new(CLOSES).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── SMA offset (crossover detection) ─────────────────────────────────

    const SMA_CROSS: &str = r#"
fn on_tick(candles, context) {
    let fast      = indicators::sma(candles, 2);
    let fast_prev = indicators::sma(candles, 2, 1);
    let slow      = indicators::sma(candles, 3);
    let slow_prev = indicators::sma(candles, 3, 1);
    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return #{ signal: "HOLD" };
    }
    if fast_prev <= slow_prev && fast > slow { return #{ signal: "BUY" }; }
    if fast_prev >= slow_prev && fast < slow { return #{ signal: "SELL" }; }
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn sma_crossover_detected() {
        let mut e = Engine::new(SMA_CROSS).unwrap();
        // Feed flat then spike to trigger cross
        e.tick(make_candle(10.0, 1), flat_ctx()).unwrap();
        e.tick(make_candle(10.0, 2), flat_ctx()).unwrap();
        e.tick(make_candle(10.0, 3), flat_ctx()).unwrap();
        // Big jump: fast SMA will cross above slow
        let d = e.tick(make_candle(100.0, 4), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── Error handling ────────────────────────────────────────────────────

    const BAD_SIGNAL: &str = r#"
fn on_tick(candles, context) { #{ signal: "MOON" } }
"#;

    #[test]
    fn unknown_signal_returns_err() {
        let mut e = Engine::new(BAD_SIGNAL).unwrap();
        let err = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap_err();
        assert!(matches!(err, EngineError::InvalidSignal(_)));
    }

    const NO_SIGNAL: &str = r#"
fn on_tick(candles, context) { #{ size: 0.1 } }
"#;

    #[test]
    fn missing_signal_key_returns_err() {
        let mut e = Engine::new(NO_SIGNAL).unwrap();
        let err = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap_err();
        assert!(matches!(err, EngineError::Strategy(_)));
    }

    const NO_FN: &str = r#"let x = 1;"#;

    #[test]
    fn missing_on_tick_returns_err() {
        assert!(Engine::new(NO_FN).is_err());
    }

    // ── Position in context ───────────────────────────────────────────────

    const POS: &str = r#"
fn on_tick(candles, context) {
    if context.position != () { return #{ signal: "SELL", reason: "close" }; }
    #{ signal: "BUY" }
}
"#;

    #[test]
    fn strategy_sees_open_position() {
        let mut e = Engine::new(POS).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);

        let ctx_with_pos = Context {
            position: Some(shared::Position {
                symbol: "TEST".into(),
                side: PositionSide::Long,
                entry_price: 100.0,
                size: 10.0,
                entry_time: 1,
                stop_loss: None,
                take_profit: None,
            }),
            ..flat_ctx()
        };
        let d = e.tick(make_candle(110.0, 2), ctx_with_pos).unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }

    // ── RSI end-to-end ────────────────────────────────────────────────────

    const RSI_STRAT: &str = r#"
fn on_tick(candles, context) {
    let r = indicators::rsi(candles, 14);
    if r == () { return #{ signal: "HOLD" }; }
    if r < 30.0 { return #{ signal: "BUY" }; }
    if r > 70.0 { return #{ signal: "SELL" }; }
    #{ signal: "HOLD" }
}
"#;

    #[test]
    fn rsi_holds_during_warmup() {
        let mut e = Engine::new(RSI_STRAT).unwrap();
        for i in 0..10i64 {
            let d = e
                .tick(make_candle(100.0 + i as f64, i), flat_ctx())
                .unwrap();
            assert_eq!(d.signal, Signal::Hold);
        }
    }

    fn assert_indicator_binding_smoke(expr: &str, ticks: usize) {
        let src = format!(
            r#"
fn on_tick(candles, context) {{
    let value = {expr};
    if value == () {{ return #{{ signal: "HOLD" }}; }}
    #{{ signal: "BUY" }}
}}
"#
        );
        let mut e = Engine::new(&src).unwrap();
        let mut last = Signal::Hold;
        for i in 0..ticks {
            let d = e
                .tick(make_candle(100.0 + i as f64, i as i64), flat_ctx())
                .unwrap();
            last = d.signal;
        }
        assert_eq!(
            last,
            Signal::Buy,
            "binding should expose non-unit result for `{expr}` after warmup"
        );
    }

    macro_rules! indicator_smoke_test {
        ($name:ident, $expr:expr, $ticks:expr) => {
            #[test]
            fn $name() {
                assert_indicator_binding_smoke($expr, $ticks);
            }
        };
    }

    indicator_smoke_test!(ema_binding_smoke, "indicators::ema(candles, 3)", 6);
    indicator_smoke_test!(dema_binding_smoke, "indicators::dema(candles, 3)", 10);
    indicator_smoke_test!(tema_binding_smoke, "indicators::tema(candles, 3)", 15);
    indicator_smoke_test!(macd_binding_smoke, "indicators::macd(candles, 3, 6, 2)", 20);
    indicator_smoke_test!(sar_binding_smoke, "indicators::sar(candles, 0.02, 0.2)", 6);
    indicator_smoke_test!(adx_binding_smoke, "indicators::adx(candles, 3)", 20);
    indicator_smoke_test!(ichimoku_binding_smoke, "indicators::ichimoku(candles)", 60);
    indicator_smoke_test!(cci_binding_smoke, "indicators::cci(candles, 3)", 10);
    indicator_smoke_test!(
        stochastic_binding_smoke,
        "indicators::stochastic(candles, 3)",
        10
    );
    indicator_smoke_test!(
        williams_r_binding_smoke,
        "indicators::williams_r(candles, 3)",
        10
    );
    indicator_smoke_test!(roc_binding_smoke, "indicators::roc(candles, 3)", 10);
    indicator_smoke_test!(
        bollinger_binding_smoke,
        "indicators::bollinger(candles, 3, 2.0)",
        10
    );
    indicator_smoke_test!(atr_binding_smoke, "indicators::atr(candles, 3)", 10);
    indicator_smoke_test!(
        keltner_binding_smoke,
        "indicators::keltner(candles, 3, 2.0)",
        10
    );
    indicator_smoke_test!(obv_binding_smoke, "indicators::obv(candles)", 3);
    indicator_smoke_test!(vwap_binding_smoke, "indicators::vwap(candles)", 3);
    indicator_smoke_test!(mfi_binding_smoke, "indicators::mfi(candles, 3)", 10);
    indicator_smoke_test!(
        volume_profile_binding_smoke,
        "indicators::volume_profile(candles, 4)",
        5
    );
    indicator_smoke_test!(
        pivot_points_binding_smoke,
        "indicators::pivot_points(candles)",
        2
    );
    indicator_smoke_test!(
        fibonacci_binding_smoke,
        "indicators::fibonacci(candles, 90.0, 110.0)",
        1
    );
    indicator_smoke_test!(slope_binding_smoke, "indicators::slope(candles, 3)", 10);

    fn assert_indicator_offset_binding_smoke(expr: &str, ticks: usize) {
        let src = format!(
            r#"
fn on_tick(candles, context) {{
    let value = {expr};
    if value == () {{ return #{{ signal: "HOLD" }}; }}
    #{{ signal: "BUY" }}
}}
"#
        );
        let mut e = Engine::new(&src).unwrap();
        let mut last = Signal::Hold;
        for i in 0..ticks {
            let d = e
                .tick(make_candle(100.0 + i as f64, i as i64), flat_ctx())
                .unwrap();
            last = d.signal;
        }
        assert_eq!(
            last,
            Signal::Buy,
            "offset binding should expose non-unit result for `{expr}` after warmup"
        );
    }

    macro_rules! indicator_offset_smoke_test {
        ($name:ident, $expr:expr, $ticks:expr) => {
            #[test]
            fn $name() {
                assert_indicator_offset_binding_smoke($expr, $ticks);
            }
        };
    }

    indicator_offset_smoke_test!(
        adx_offset_binding_smoke,
        "indicators::adx(candles, 3, 1)",
        20
    );
    indicator_offset_smoke_test!(
        ichimoku_offset_binding_smoke,
        "indicators::ichimoku(candles, 1)",
        60
    );
    indicator_offset_smoke_test!(
        stochastic_offset_binding_smoke,
        "indicators::stochastic(candles, 3, 1)",
        10
    );
    indicator_offset_smoke_test!(
        bollinger_offset_binding_smoke,
        "indicators::bollinger(candles, 3, 2.0, 1)",
        10
    );
    indicator_offset_smoke_test!(
        keltner_offset_binding_smoke,
        "indicators::keltner(candles, 3, 2.0, 1)",
        10
    );
    indicator_offset_smoke_test!(obv_offset_binding_smoke, "indicators::obv(candles, 1)", 3);
    indicator_offset_smoke_test!(vwap_offset_binding_smoke, "indicators::vwap(candles, 1)", 3);
    indicator_offset_smoke_test!(
        mfi_offset_binding_smoke,
        "indicators::mfi(candles, 3, 1)",
        10
    );
    indicator_offset_smoke_test!(
        volume_profile_offset_binding_smoke,
        "indicators::volume_profile(candles, 4, 1)",
        5
    );
    indicator_offset_smoke_test!(
        pivot_points_offset_binding_smoke,
        "indicators::pivot_points(candles, 1)",
        3
    );

    const OBV_OFFSET_SEMANTICS: &str = r#"
fn on_tick(candles, context) {
    let current = indicators::obv(candles);
    let previous = indicators::obv(candles, 1);

    let seen = context.state("seen", 0);
    let last = context.state_f("last", 0.0);

    let result = if seen == 1 && previous != () {
        let matches = previous >= last - 0.000001 && previous <= last + 0.000001;
        if matches { "BUY" } else { "SELL" }
    } else {
        "HOLD"
    };

    if current != () {
        context.set_state_f("last", current);
        context.set_state("seen", 1);
    }

    #{ signal: result }
}
"#;

    #[test]
    fn obv_offset_matches_previous_tick_value() {
        let mut e = Engine::new(OBV_OFFSET_SEMANTICS).unwrap();
        let mut last = Signal::Hold;
        for (i, close) in [10.0, 12.0, 11.0, 15.0].into_iter().enumerate() {
            let d = e.tick(make_candle(close, i as i64), flat_ctx()).unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy);
    }

    const PIVOT_POINTS_OFFSET_SEMANTICS: &str = r#"
fn on_tick(candles, context) {
    let current = indicators::pivot_points(candles);
    let previous = indicators::pivot_points(candles, 1);

    let seen = context.state("seen", 0);
    let last_pp = context.state_f("pp", 0.0);
    let last_r1 = context.state_f("r1", 0.0);
    let last_r2 = context.state_f("r2", 0.0);
    let last_r3 = context.state_f("r3", 0.0);
    let last_s1 = context.state_f("s1", 0.0);
    let last_s2 = context.state_f("s2", 0.0);
    let last_s3 = context.state_f("s3", 0.0);

    let result = if seen == 1 && previous != () {
        let pp_ok = previous.pp >= last_pp - 0.000001 && previous.pp <= last_pp + 0.000001;
        let r1_ok = previous.r1 >= last_r1 - 0.000001 && previous.r1 <= last_r1 + 0.000001;
        let r2_ok = previous.r2 >= last_r2 - 0.000001 && previous.r2 <= last_r2 + 0.000001;
        let r3_ok = previous.r3 >= last_r3 - 0.000001 && previous.r3 <= last_r3 + 0.000001;
        let s1_ok = previous.s1 >= last_s1 - 0.000001 && previous.s1 <= last_s1 + 0.000001;
        let s2_ok = previous.s2 >= last_s2 - 0.000001 && previous.s2 <= last_s2 + 0.000001;
        let s3_ok = previous.s3 >= last_s3 - 0.000001 && previous.s3 <= last_s3 + 0.000001;
        let all_resistance_ok = pp_ok && r1_ok && r2_ok && r3_ok;
        let all_support_ok = s1_ok && s2_ok && s3_ok;
        if all_resistance_ok && all_support_ok { "BUY" } else { "SELL" }
    } else {
        "HOLD"
    };

    if current != () {
        context.set_state_f("pp", current.pp);
        context.set_state_f("r1", current.r1);
        context.set_state_f("r2", current.r2);
        context.set_state_f("r3", current.r3);
        context.set_state_f("s1", current.s1);
        context.set_state_f("s2", current.s2);
        context.set_state_f("s3", current.s3);
        context.set_state("seen", 1);
    }

    #{ signal: result }
}
"#;

    #[test]
    fn pivot_points_offset_matches_previous_tick_levels() {
        let mut e = Engine::new(PIVOT_POINTS_OFFSET_SEMANTICS).unwrap();
        let mut last = Signal::Hold;
        for (i, close) in [100.0, 101.0, 102.0].into_iter().enumerate() {
            let d = e.tick(make_candle(close, i as i64), flat_ctx()).unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy);
    }

    const BOLLINGER_OFFSET_SEMANTICS: &str = r#"
fn on_tick(candles, context) {
    let current = indicators::bollinger(candles, 3, 2.0);
    let previous = indicators::bollinger(candles, 3, 2.0, 1);

    let seen = context.state("seen", 0);
    let last_upper = context.state_f("upper", 0.0);
    let last_middle = context.state_f("middle", 0.0);
    let last_lower = context.state_f("lower", 0.0);

    let result = if seen == 1 && previous != () {
        let upper_ok = previous.upper >= last_upper - 0.000001 && previous.upper <= last_upper + 0.000001;
        let middle_ok = previous.middle >= last_middle - 0.000001 && previous.middle <= last_middle + 0.000001;
        let lower_ok = previous.lower >= last_lower - 0.000001 && previous.lower <= last_lower + 0.000001;
        if upper_ok && middle_ok && lower_ok { "BUY" } else { "SELL" }
    } else {
        "HOLD"
    };

    if current != () {
        context.set_state_f("upper", current.upper);
        context.set_state_f("middle", current.middle);
        context.set_state_f("lower", current.lower);
        context.set_state("seen", 1);
    }

    #{ signal: result }
}
"#;

    #[test]
    fn bollinger_offset_matches_previous_tick_bands() {
        let mut e = Engine::new(BOLLINGER_OFFSET_SEMANTICS).unwrap();
        let mut last = Signal::Hold;
        for (i, close) in [1.0, 2.0, 3.0, 4.0, 5.0].into_iter().enumerate() {
            let d = e.tick(make_candle(close, i as i64), flat_ctx()).unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy);
    }

    const STOCHASTIC_OFFSET_SEMANTICS: &str = r#"
fn on_tick(candles, context) {
    let current = indicators::stochastic(candles, 3);
    let previous = indicators::stochastic(candles, 3, 1);

    let seen = context.state("seen", 0);
    let last_k = context.state_f("k", 0.0);
    let last_d = context.state_f("d", 0.0);

    let result = if seen == 1 && previous != () {
        let k_ok = previous.k >= last_k - 0.000001 && previous.k <= last_k + 0.000001;
        let d_ok = previous.d >= last_d - 0.000001 && previous.d <= last_d + 0.000001;
        if k_ok && d_ok { "BUY" } else { "SELL" }
    } else {
        "HOLD"
    };

    if current != () {
        context.set_state_f("k", current.k);
        context.set_state_f("d", current.d);
        context.set_state("seen", 1);
    }

    #{ signal: result }
}
"#;

    #[test]
    fn stochastic_offset_matches_previous_tick_values() {
        let mut e = Engine::new(STOCHASTIC_OFFSET_SEMANTICS).unwrap();
        let mut last = Signal::Hold;
        for (i, close) in [10.0, 11.0, 12.0, 13.0, 12.5, 14.0].into_iter().enumerate() {
            let d = e.tick(make_candle(close, i as i64), flat_ctx()).unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy);
    }

    const ADX_OFFSET_SEMANTICS: &str = r#"
fn on_tick(candles, context) {
    let current = indicators::adx(candles, 3);
    let previous = indicators::adx(candles, 3, 1);

    let seen = context.state("seen", 0);
    let last_adx = context.state_f("adx", 0.0);
    let last_plus_di = context.state_f("plus_di", 0.0);
    let last_minus_di = context.state_f("minus_di", 0.0);

    let result = if seen == 1 && previous != () {
        let adx_ok = previous.adx >= last_adx - 0.000001 && previous.adx <= last_adx + 0.000001;
        let plus_ok = previous.plus_di >= last_plus_di - 0.000001 && previous.plus_di <= last_plus_di + 0.000001;
        let minus_ok = previous.minus_di >= last_minus_di - 0.000001 && previous.minus_di <= last_minus_di + 0.000001;
        if adx_ok && plus_ok && minus_ok { "BUY" } else { "SELL" }
    } else {
        "HOLD"
    };

    if current != () {
        context.set_state_f("adx", current.adx);
        context.set_state_f("plus_di", current.plus_di);
        context.set_state_f("minus_di", current.minus_di);
        context.set_state("seen", 1);
    }

    #{ signal: result }
}
"#;

    #[test]
    fn adx_offset_matches_previous_tick_values() {
        let mut e = Engine::new(ADX_OFFSET_SEMANTICS).unwrap();
        let mut last = Signal::Hold;
        for (i, close) in [10.0, 11.0, 12.0, 13.0, 12.0, 14.0, 15.0, 14.5].into_iter().enumerate() {
            let d = e.tick(make_candle(close, i as i64), flat_ctx()).unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy);
    }

    // ── Anchored: strategy declares pivot+trendline config ────────────────

    const ANCHORED: &str = r#"
fn anchored_config() {
    #{
        detectors: [ #{ id: "p", kind: "pivot", left: 2, right: 2 } ],
        evaluators: [ #{
            expose_as: "res", kind: "trendline", side: "resistance",
            pivot_source: "p", pivot_buffer: 6,
            tolerance: 0.01, min_touches: 3, max_lines: 1
        } ],
    }
}
fn on_tick(candles, context) {
    let res = context.anchored("res");
    if type_of(res) == "array" && res.len() > 0 {
        let line = res[0];
        let bar = candles[1].bar;
        if candles[1].close > line.y_at(bar) {
            return #{ signal: "BUY", reason: "broke resistance" };
        }
    }
    #{ signal: "HOLD" }
}
"#;

    fn hlc_candle(h: f64, l: f64, c: f64, ts: i64) -> Candle {
        Candle {
            timestamp: ts,
            symbol: "T".into(),
            open: c,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            timeframe: "1m".into(),
        }
    }

    #[test]
    fn anchored_pipeline_exposes_trendline_and_fires_on_break() {
        let mut e = Engine::new(ANCHORED).unwrap();
        // Build three pivot highs at 100 (bars 2, 7, 12 with left=right=2) then break up at bar 15.
        // Pattern for each high: low, low, HIGH, low, low.
        let seq: &[(f64, f64)] = &[
            (98.0, 95.0),   // 0
            (98.0, 95.0),   // 1
            (100.0, 95.0),  // 2  pivot-high candidate
            (98.0, 95.0),   // 3
            (98.0, 95.0),   // 4  confirms pivot@2
            (98.0, 95.0),   // 5
            (98.0, 95.0),   // 6
            (100.0, 95.0),  // 7  pivot-high candidate
            (98.0, 95.0),   // 8
            (98.0, 95.0),   // 9  confirms pivot@7
            (98.0, 95.0),   // 10
            (98.0, 95.0),   // 11
            (100.0, 95.0),  // 12 pivot-high candidate
            (98.0, 95.0),   // 13
            (98.0, 95.0),   // 14 confirms pivot@12 → trendline becomes active
            (110.0, 108.0), // 15 close=108 > 100 → BUY
        ];
        let mut last = Signal::Hold;
        for (i, &(h, l)) in seq.iter().enumerate() {
            // close = l on pivot bars (to keep them non-breaking), close = 108 on break bar
            let close = if i == 15 { 108.0 } else { l };
            let d = e
                .tick(hlc_candle(h, l, close, i as i64), flat_ctx())
                .unwrap();
            last = d.signal;
        }
        assert_eq!(last, Signal::Buy, "expected breakout BUY on final bar");
    }

    #[test]
    fn trendline_break_strategy_loads() {
        // Loads the actual strategy file from disk — exercises the full spec parse.
        let src = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../strategies/trendline_break.rhai"
        ))
        .expect("read trendline_break.rhai");
        let mut e = Engine::new(&src).expect("engine should load");
        // A single HOLD tick must not crash (no anchored output yet).
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    #[test]
    fn strategy_without_anchored_config_works() {
        // HOLD strategy has no anchored_config — must not break.
        let mut e = Engine::new(HOLD).unwrap();
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    #[test]
    fn rsi_sells_in_strong_uptrend() {
        let mut e = Engine::new(RSI_STRAT).unwrap();
        for i in 0..31i64 {
            e.tick(make_candle(100.0 + i as f64 * 5.0, i), flat_ctx())
                .unwrap();
        }
        let d = e
            .tick(make_candle(100.0 + 31.0 * 5.0, 31), flat_ctx())
            .unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }

    // ── State API ─────────────────────────────────────────────────────────

    #[test]
    fn state_persists_between_ticks() {
        const STRAT: &str = r#"
fn on_tick(candles, context) {
    let current = context.state("counter", 0);
    context.set_state("counter", current + 1);
    if current >= 3 { return #{ signal: "SELL" }; }
    #{ signal: "BUY" }
}
"#;
        let mut e = Engine::new(STRAT).unwrap();
        // Tick 1: counter=0, current=0, set counter=1, signal=BUY
        let d = e.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
        // Tick 2: counter=1, current=1, set counter=2, signal=BUY
        let d = e.tick(make_candle(100.0, 2), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
        // Tick 3: counter=2, current=2, set counter=3, signal=BUY
        let d = e.tick(make_candle(100.0, 3), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
        // Tick 4: counter=3, current=3, set counter=4, signal=SELL
        let d = e.tick(make_candle(100.0, 4), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }
}
