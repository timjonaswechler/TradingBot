use std::str::FromStr;
use std::sync::{Arc, RwLock};

use rhai::{Dynamic, Engine as RhaiEngine, Scope, AST};
use shared::{Candle, Context, Signal, TradeDecision};

use crate::{
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
    rhai:    RhaiEngine,
    ast:     AST,
    scope:   Scope<'static>,
    candles: Arc<RwLock<Vec<Candle>>>,
    cache:   Arc<RwLock<IndicatorCache>>,
}

impl Engine {
    /// Create a new engine and compile the strategy source.
    pub fn new(strategy_src: &str) -> Result<Self, EngineError> {
        let mut rhai = RhaiEngine::new();

        // Register all custom types and the indicators:: module.
        register_types(&mut rhai);
        register_all(&mut rhai);

        // Compile the strategy — catches syntax errors early.
        let ast = rhai.compile(strategy_src).map_err(|e| {
            EngineError::Strategy(format!("strategy compile error: {e}"))
        })?;

        // Execute top-level code (constants, etc.) into a persistent scope.
        let mut scope = Scope::new();
        rhai.run_ast_with_scope(&mut scope, &ast).map_err(|e| {
            EngineError::Strategy(format!("strategy init error: {e}"))
        })?;

        // Verify on_tick is defined.
        if !rhai.call_fn::<Dynamic>(&mut scope, &ast, "on_tick", ()).is_ok() {
            // A real call with no args will fail — we just need the function to exist.
            // Check via AST iteration instead.
        }
        // Better check: try to find "on_tick" in the AST
        let has_on_tick = ast.iter_functions().any(|f| f.name == "on_tick");
        if !has_on_tick {
            return Err(EngineError::Strategy(
                "strategy must define `fn on_tick(candles, context)`".into(),
            ));
        }

        let candles = Arc::new(RwLock::new(Vec::new()));
        let cache   = Arc::new(RwLock::new(IndicatorCache::new()));

        Ok(Self { rhai, ast, scope, candles, cache })
    }

    /// Feed one candle into the engine and get a trading decision back.
    pub fn tick(&mut self, candle: Candle, ctx: Context) -> Result<TradeDecision, EngineError> {
        // Append the new candle.
        self.candles.write().unwrap().push(candle);

        // Build Rhai arguments — cheap Arc clones, no candle data copied.
        let candle_list = CandleList::new(
            Arc::clone(&self.candles),
            Arc::clone(&self.cache),
        );
        let ctx_wrapper = ContextWrapper(ctx);

        // Call on_tick(candles, context).
        let result: Dynamic = self.rhai
            .call_fn(&mut self.scope, &self.ast, "on_tick", (candle_list, ctx_wrapper))
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

    let signal = Signal::from_str(&signal_str)
        .map_err(|e| EngineError::InvalidSignal(e))?;

    // size — optional; defaults to 1.0 for directional signals, 0.0 for HOLD
    let default_size = match signal {
        Signal::Hold => 0.0,
        _            => 1.0,
    };
    let size = map
        .get("size")
        .and_then(|v| v.clone().try_cast::<f64>())
        .unwrap_or(default_size);

    // Optional fields
    let stop_loss   = map.get("stop_loss").and_then(|v| v.clone().try_cast::<f64>());
    let take_profit = map.get("take_profit").and_then(|v| v.clone().try_cast::<f64>());
    let reason      = map.get("reason").and_then(|v| v.clone().try_cast::<String>());

    Ok(TradeDecision { signal, size, stop_loss, take_profit, reason })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PositionSide;

    fn make_candle(close: f64, ts: i64) -> Candle {
        Candle {
            timestamp: ts,
            symbol:    "TEST".into(),
            open:      close - 0.5,
            high:      close + 1.0,
            low:       close - 1.0,
            close,
            volume:    1000.0,
            timeframe: "1d".into(),
        }
    }

    fn flat_ctx() -> Context { Context::new(10_000.0) }

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
                symbol:      "TEST".into(),
                side:        PositionSide::Long,
                entry_price: 100.0,
                size:        10.0,
                entry_time:  1,
                stop_loss:   None,
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
            let d = e.tick(make_candle(100.0 + i as f64, i), flat_ctx()).unwrap();
            assert_eq!(d.signal, Signal::Hold);
        }
    }

    #[test]
    fn rsi_sells_in_strong_uptrend() {
        let mut e = Engine::new(RSI_STRAT).unwrap();
        for i in 0..31i64 {
            e.tick(make_candle(100.0 + i as f64 * 5.0, i), flat_ctx()).unwrap();
        }
        let d = e.tick(make_candle(100.0 + 31.0 * 5.0, 31), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }
}
