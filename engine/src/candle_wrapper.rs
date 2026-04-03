use rhai::{Dynamic, Engine, INT};
use shared::{Candle, Context, Position};
use std::sync::{Arc, RwLock};
use crate::indicator_cache::IndicatorCache;

// ── CandleWrapper ─────────────────────────────────────────────────────────────

/// A single OHLCV candle exposed to Rhai strategies.
#[derive(Debug, Clone)]
pub struct CandleWrapper(pub Candle);

// ── CandleList ────────────────────────────────────────────────────────────────

/// Passed to `on_tick` as the `candles` argument.
///
/// Holds cheap Arc clones — no candle data is copied each tick.
///
/// Rhai-side contract:
/// - `candles[1]`           → current (newest) candle
/// - `candles[n]`           → nth candle back  (unit `()` if out of range)
/// - `candles.len()`        → number of candles visible
/// - `candles.closes()`     → array of closes, index 1 = newest
/// - `candles.opens()`      → same for opens
/// - `candles.highs()`      → same for highs
/// - `candles.lows()`       → same for lows
/// - `candles.volumes()`    → same for volumes
#[derive(Clone)]
pub struct CandleList {
    pub candles: Arc<RwLock<Vec<Candle>>>,
    pub cache:   Arc<RwLock<IndicatorCache>>,
}

impl CandleList {
    pub fn new(candles: Arc<RwLock<Vec<Candle>>>, cache: Arc<RwLock<IndicatorCache>>) -> Self {
        Self { candles, cache }
    }
}

// ── ContextWrapper ────────────────────────────────────────────────────────────

/// Portfolio context passed to `on_tick` every candle.
#[derive(Debug, Clone)]
pub struct ContextWrapper(pub Context);

/// An open position exposed to Rhai.
#[derive(Debug, Clone)]
pub struct PositionWrapper(pub Position);

// ── Registration helpers ──────────────────────────────────────────────────────

fn price_array(cl: &mut CandleList, field: &str) -> rhai::Array {
    cl.candles
        .read()
        .unwrap()
        .iter()
        .rev()
        .map(|c| Dynamic::from(match field {
            "close"  => c.close,
            "open"   => c.open,
            "high"   => c.high,
            "low"    => c.low,
            "volume" => c.volume,
            _        => unreachable!(),
        }))
        .collect()
}

/// Register all custom types and their fields/methods on the Rhai `Engine`.
pub fn register_types(engine: &mut Engine) {
    // ── CandleWrapper ─────────────────────────────────────────────────────
    engine.register_type_with_name::<CandleWrapper>("Candle");

    engine.register_get("open",      |c: &mut CandleWrapper| c.0.open);
    engine.register_get("high",      |c: &mut CandleWrapper| c.0.high);
    engine.register_get("low",       |c: &mut CandleWrapper| c.0.low);
    engine.register_get("close",     |c: &mut CandleWrapper| c.0.close);
    engine.register_get("volume",    |c: &mut CandleWrapper| c.0.volume);
    engine.register_get("timestamp", |c: &mut CandleWrapper| c.0.timestamp);
    engine.register_get("symbol",    |c: &mut CandleWrapper| c.0.symbol.clone());

    engine.register_fn("body",  |c: &mut CandleWrapper| c.0.body());
    engine.register_fn("range", |c: &mut CandleWrapper| c.0.range());

    // ── CandleList ────────────────────────────────────────────────────────
    engine.register_type_with_name::<CandleList>("CandleList");

    // candles[n]  — 1-indexed, newest first
    engine.register_indexer_get(|cl: &mut CandleList, idx: INT| -> Dynamic {
        if idx < 1 { return Dynamic::UNIT; }
        let candles = cl.candles.read().unwrap();
        let n = candles.len();
        match n.checked_sub(idx as usize) {
            Some(i) => Dynamic::from(CandleWrapper(candles[i].clone())),
            None    => Dynamic::UNIT,
        }
    });

    // candles.len()
    engine.register_fn("len", |cl: &mut CandleList| -> INT {
        cl.candles.read().unwrap().len() as INT
    });

    // Price-series helpers — newest first, consistent with candles[1]
    engine.register_fn("closes",  |cl: &mut CandleList| price_array(cl, "close"));
    engine.register_fn("opens",   |cl: &mut CandleList| price_array(cl, "open"));
    engine.register_fn("highs",   |cl: &mut CandleList| price_array(cl, "high"));
    engine.register_fn("lows",    |cl: &mut CandleList| price_array(cl, "low"));
    engine.register_fn("volumes", |cl: &mut CandleList| price_array(cl, "volume"));

    // ── PositionWrapper ───────────────────────────────────────────────────
    engine.register_type_with_name::<PositionWrapper>("Position");

    engine.register_get("side",        |p: &mut PositionWrapper| p.0.side.to_string());
    engine.register_get("entry_price", |p: &mut PositionWrapper| p.0.entry_price);
    engine.register_get("size",        |p: &mut PositionWrapper| p.0.size);
    engine.register_get("entry_time",  |p: &mut PositionWrapper| p.0.entry_time);
    engine.register_get("stop_loss",   |p: &mut PositionWrapper| -> Dynamic {
        p.0.stop_loss.map(Dynamic::from).unwrap_or(Dynamic::UNIT)
    });
    engine.register_get("take_profit", |p: &mut PositionWrapper| -> Dynamic {
        p.0.take_profit.map(Dynamic::from).unwrap_or(Dynamic::UNIT)
    });

    // ── ContextWrapper ────────────────────────────────────────────────────
    engine.register_type_with_name::<ContextWrapper>("Context");

    engine.register_get("balance",      |c: &mut ContextWrapper| c.0.balance);
    engine.register_get("equity",       |c: &mut ContextWrapper| c.0.equity);
    engine.register_get("trades_count", |c: &mut ContextWrapper| c.0.trades_count as INT);
    engine.register_get("position",     |c: &mut ContextWrapper| -> Dynamic {
        match &c.0.position {
            None      => Dynamic::UNIT,
            Some(pos) => Dynamic::from(PositionWrapper(pos.clone())),
        }
    });
    engine.register_fn("has_position", |c: &mut ContextWrapper| c.0.has_position());
}
