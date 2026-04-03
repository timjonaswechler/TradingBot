use std::cell::RefCell;
use std::str::FromStr;

use shared::{Candle, Context, Signal, TradeDecision};

use crate::{
    bindings::register_indicators,
    candle_wrapper::{LuaCandles, LuaContext},
    error::EngineError,
    indicator_cache::IndicatorCache,
};

// ── EngineData ───────────────────────────────────────────────────────────────

/// All mutable runtime state kept as Lua app-data so indicator bindings can
/// access it during `on_tick` without holding a borrow on `LuaEngine`.
pub struct EngineData {
    pub candles: Vec<Candle>,
    pub cache:   IndicatorCache,
}

// ── LuaEngine ────────────────────────────────────────────────────────────────

/// A long-lived Lua VM that runs a single strategy file.
///
/// - For **live trading** (daemon): one `LuaEngine` per strategy, kept running
///   for the entire session. The indicator cache warms up over time.
/// - For **backtesting** (UI): one fresh `LuaEngine` per backtest run;
///   call `tick` for every historical candle in chronological order.
pub struct LuaEngine {
    lua: mlua::Lua,
}

impl LuaEngine {
    /// Create a new engine and load the strategy source.
    ///
    /// The Lua VM is initialised, all indicator bindings are registered, and the
    /// strategy script is executed (which defines `on_tick` as a global).
    pub fn new(strategy_src: &str) -> Result<Self, EngineError> {
        let lua = mlua::Lua::new();

        // Initialise engine data as app-data so bindings can access it.
        lua.set_app_data(RefCell::new(EngineData {
            candles: Vec::new(),
            cache:   IndicatorCache::new(),
        }));

        // Register all indicators as Lua globals.
        register_indicators(&lua)?;

        // Load and execute the strategy (defines on_tick).
        lua.load(strategy_src).exec().map_err(|e| {
            EngineError::Strategy(format!("failed to load strategy: {e}"))
        })?;

        // Verify on_tick exists.
        let on_tick: mlua::Value = lua.globals().get("on_tick")?;
        if !matches!(on_tick, mlua::Value::Function(_)) {
            return Err(EngineError::Strategy(
                "strategy must define `on_tick(candles, context)`".into(),
            ));
        }

        Ok(Self { lua })
    }

    /// Feed one candle into the engine and get a trading decision back.
    ///
    /// The candle is appended to the internal history before `on_tick` is called,
    /// so `candles[1]` inside Lua always refers to this new candle.
    pub fn tick(&mut self, candle: Candle, ctx: Context) -> Result<TradeDecision, EngineError> {
        // Append the new candle — borrow released before we call Lua.
        {
            let data = self.lua
                .app_data_ref::<RefCell<EngineData>>()
                .expect("EngineData must be set");
            data.borrow_mut().candles.push(candle);
        }

        // Build Lua arguments.
        let lua_candles = self.lua.create_userdata(LuaCandles)?;
        let lua_ctx     = self.lua.create_userdata(LuaContext(ctx))?;

        // Call on_tick.
        let on_tick: mlua::Function = self.lua.globals().get("on_tick")?;
        let result: mlua::Value = on_tick
            .call((lua_candles, lua_ctx))
            .map_err(|e| EngineError::Strategy(format!("on_tick error: {e}")))?;

        // Parse the returned table into a TradeDecision.
        match result {
            mlua::Value::Table(t) => parse_trade_decision(&t),
            mlua::Value::Nil      => Err(EngineError::Strategy(
                "on_tick returned nil; it must return a table with a `signal` key".into(),
            )),
            other => Err(EngineError::Strategy(format!(
                "on_tick must return a table, got: {:?}",
                other.type_name()
            ))),
        }
    }

    /// Number of candles currently in the engine's history.
    pub fn candle_count(&self) -> usize {
        self.lua
            .app_data_ref::<RefCell<EngineData>>()
            .map(|d| d.borrow().candles.len())
            .unwrap_or(0)
    }

    /// Pre-load historical candles to warm up the indicator cache without
    /// triggering any trading logic.  Used by the warmup module.
    pub fn push_candle(&mut self, candle: Candle) {
        let data = self.lua
            .app_data_ref::<RefCell<EngineData>>()
            .expect("EngineData must be set");
        data.borrow_mut().candles.push(candle);
    }
}

// ── Parse helper ─────────────────────────────────────────────────────────────

fn parse_trade_decision(t: &mlua::Table) -> Result<TradeDecision, EngineError> {
    // signal — required
    let signal_str: String = t
        .get::<String>("signal")
        .map_err(|_| EngineError::Strategy("on_tick return table missing `signal` key".into()))?;

    let signal = Signal::from_str(&signal_str)
        .map_err(|e| EngineError::InvalidSignal(e))?;

    // size — optional; default 1.0 for directional signals, 0.0 for HOLD
    let default_size = match signal {
        Signal::Hold => 0.0,
        _            => 1.0,
    };
    let size: f64 = t.get::<Option<f64>>("size")
        .unwrap_or(None)
        .unwrap_or(default_size);

    // Optional fields
    let stop_loss:   Option<f64>    = t.get::<Option<f64>>("stop_loss").unwrap_or(None);
    let take_profit: Option<f64>    = t.get::<Option<f64>>("take_profit").unwrap_or(None);
    let reason:      Option<String> = t.get::<Option<String>>("reason").unwrap_or(None);

    Ok(TradeDecision { signal, size, stop_loss, take_profit, reason })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PositionSide;

    fn make_candle(close: f64, n: i64) -> Candle {
        Candle {
            timestamp: n,
            symbol:    "TEST".into(),
            open:      close - 0.5,
            high:      close + 1.0,
            low:       close - 1.0,
            close,
            volume:    1000.0,
            timeframe: "1d".into(),
        }
    }

    fn flat_ctx() -> Context {
        Context::new(10_000.0)
    }

    // ── Strategy: always HOLD ─────────────────────────────────────────────

    const HOLD_STRATEGY: &str = r#"
function on_tick(candles, context)
    return { signal = "HOLD" }
end
"#;

    #[test]
    fn engine_loads_and_ticks() {
        let mut engine = LuaEngine::new(HOLD_STRATEGY).unwrap();
        let decision = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(decision.signal, Signal::Hold);
    }

    #[test]
    fn candle_count_grows() {
        let mut engine = LuaEngine::new(HOLD_STRATEGY).unwrap();
        engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        engine.tick(make_candle(101.0, 2), flat_ctx()).unwrap();
        assert_eq!(engine.candle_count(), 2);
    }

    // ── Candle access from Lua ────────────────────────────────────────────

    const ACCESS_STRATEGY: &str = r#"
function on_tick(candles, context)
    local c = candles[1]
    if c == nil then return { signal = "HOLD" } end
    if c.close > 100.0 then
        return { signal = "BUY", size = 0.5, reason = "above 100" }
    end
    return { signal = "HOLD" }
end
"#;

    #[test]
    fn strategy_reads_candle_fields() {
        let mut engine = LuaEngine::new(ACCESS_STRATEGY).unwrap();
        let d = engine.tick(make_candle(105.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
        assert_eq!(d.size, 0.5);
        assert_eq!(d.reason.as_deref(), Some("above 100"));
    }

    #[test]
    fn strategy_hold_when_below_threshold() {
        let mut engine = LuaEngine::new(ACCESS_STRATEGY).unwrap();
        let d = engine.tick(make_candle(99.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    // ── Context access from Lua ───────────────────────────────────────────

    const CTX_STRATEGY: &str = r#"
function on_tick(candles, context)
    if context.balance > 5000 then
        return { signal = "BUY" }
    end
    return { signal = "HOLD" }
end
"#;

    #[test]
    fn strategy_reads_context() {
        let mut engine = LuaEngine::new(CTX_STRATEGY).unwrap();
        let d = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy); // balance = 10_000 > 5_000
    }

    // ── Indicator access from Lua ─────────────────────────────────────────

    const SMA_STRATEGY: &str = r#"
function on_tick(candles, context)
    local s = indicators.sma(candles, 3)
    if s == nil then return { signal = "HOLD", reason = "warming up" } end
    if candles[1].close > s then
        return { signal = "BUY" }
    end
    return { signal = "SELL" }
end
"#;

    #[test]
    fn sma_returns_nil_during_warmup() {
        let mut engine = LuaEngine::new(SMA_STRATEGY).unwrap();
        // Only 1 candle — SMA(3) needs 3
        let d = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Hold);
    }

    #[test]
    fn sma_works_after_warmup() {
        let mut engine = LuaEngine::new(SMA_STRATEGY).unwrap();
        engine.tick(make_candle(10.0, 1), flat_ctx()).unwrap();
        engine.tick(make_candle(10.0, 2), flat_ctx()).unwrap();
        // Third candle: SMA(3) = (10+10+20)/3 = 13.33, close=20 > SMA → BUY
        let d = engine.tick(make_candle(20.0, 3), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── Candle helpers (closes/opens etc.) ───────────────────────────────

    const CLOSES_STRATEGY: &str = r#"
function on_tick(candles, context)
    local closes = candles:closes()
    if closes == nil or #closes == 0 then return { signal = "HOLD" } end
    -- closes[1] should be the newest close
    if closes[1] > 50 then return { signal = "BUY" } end
    return { signal = "HOLD" }
end
"#;

    #[test]
    fn candle_helper_closes() {
        let mut engine = LuaEngine::new(CLOSES_STRATEGY).unwrap();
        let d = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);
    }

    // ── Error handling ────────────────────────────────────────────────────

    const BAD_SIGNAL_STRATEGY: &str = r#"
function on_tick(candles, context)
    return { signal = "MOON" }
end
"#;

    #[test]
    fn unknown_signal_returns_err() {
        let mut engine = LuaEngine::new(BAD_SIGNAL_STRATEGY).unwrap();
        let err = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap_err();
        assert!(matches!(err, EngineError::InvalidSignal(_)));
    }

    const MISSING_SIGNAL_STRATEGY: &str = r#"
function on_tick(candles, context)
    return { size = 0.1 }
end
"#;

    #[test]
    fn missing_signal_key_returns_err() {
        let mut engine = LuaEngine::new(MISSING_SIGNAL_STRATEGY).unwrap();
        let err = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap_err();
        assert!(matches!(err, EngineError::Strategy(_)));
    }

    // ── Position in context ───────────────────────────────────────────────

    const POS_STRATEGY: &str = r#"
function on_tick(candles, context)
    if context.position ~= nil then
        return { signal = "SELL", reason = "close position" }
    end
    return { signal = "BUY" }
end
"#;

    #[test]
    fn strategy_sees_open_position() {
        let mut engine = LuaEngine::new(POS_STRATEGY).unwrap();

        // First tick — no position → BUY
        let d = engine.tick(make_candle(100.0, 1), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Buy);

        // Second tick — with position → SELL
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
        let d = engine.tick(make_candle(110.0, 2), ctx_with_pos).unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }

    // ── RSI indicator end-to-end ──────────────────────────────────────────

    const RSI_STRATEGY: &str = r#"
function on_tick(candles, context)
    local r = indicators.rsi(candles, 14)
    if r == nil then return { signal = "HOLD" } end
    if r < 30 then return { signal = "BUY" } end
    if r > 70 then return { signal = "SELL" } end
    return { signal = "HOLD" }
end
"#;

    #[test]
    fn rsi_strategy_holds_during_warmup() {
        let mut engine = LuaEngine::new(RSI_STRATEGY).unwrap();
        // Only 10 candles — RSI(14) needs >14
        for i in 0..10 {
            let d = engine.tick(make_candle(100.0 + i as f64, i), flat_ctx()).unwrap();
            assert_eq!(d.signal, Signal::Hold);
        }
    }

    #[test]
    fn rsi_sells_in_strong_uptrend() {
        let mut engine = LuaEngine::new(RSI_STRATEGY).unwrap();
        // Feed 30 strongly rising candles — RSI should go > 70
        for i in 0..30 {
            engine.tick(make_candle(100.0 + i as f64 * 5.0, i), flat_ctx()).unwrap();
        }
        let d = engine.tick(make_candle(100.0 + 30.0 * 5.0, 30), flat_ctx()).unwrap();
        assert_eq!(d.signal, Signal::Sell);
    }
}
