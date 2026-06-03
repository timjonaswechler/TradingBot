use crate::anchored::{AnchoredOutput, AnchoredOutputs};
use crate::indicator_cache::IndicatorCache;
use domain::{Candle, Context, Position, PositionSide};
use rhai::{Dynamic, Engine, INT};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// ── CandleWrapper ─────────────────────────────────────────────────────────────

/// A single OHLCV candle exposed to Rhai strategies, along with its absolute
/// bar index in the engine's candle history (0-based, monotonic).
#[derive(Debug, Clone)]
pub struct CandleWrapper {
    pub candle: Candle,
    pub bar: u64,
}

impl CandleWrapper {
    pub fn new(candle: Candle, bar: u64) -> Self {
        Self { candle, bar }
    }
}

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
    pub cache: Arc<RwLock<IndicatorCache>>,
}

impl CandleList {
    pub fn new(candles: Arc<RwLock<Vec<Candle>>>, cache: Arc<RwLock<IndicatorCache>>) -> Self {
        Self { candles, cache }
    }
}

// ── ContextWrapper ────────────────────────────────────────────────────────────

/// Portfolio context + per-tick anchored outputs, passed to `on_tick`.
#[derive(Debug, Clone)]
pub struct ContextWrapper {
    pub ctx: Context,
    pub anchored: Arc<AnchoredOutputs>,
    pub current_price: f64,
    pub state: Arc<RwLock<HashMap<String, Dynamic>>>,
}

impl ContextWrapper {
    pub fn new(
        ctx: Context,
        anchored: Arc<AnchoredOutputs>,
        current_price: f64,
        state: Arc<RwLock<HashMap<String, Dynamic>>>,
    ) -> Self {
        Self {
            ctx,
            anchored,
            current_price,
            state,
        }
    }
    pub fn plain(
        ctx: Context,
        current_price: f64,
        state: Arc<RwLock<HashMap<String, Dynamic>>>,
    ) -> Self {
        Self {
            ctx,
            anchored: Arc::new(AnchoredOutputs::default()),
            current_price,
            state,
        }
    }
}

// ── TrendLineRhai / PivotEventRhai ────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct TrendLineRhai(pub indicators::anchored::evaluators::TrendLine);

#[derive(Debug, Clone, Copy)]
pub struct PivotEventRhai {
    pub bar: u64,
    pub price: f64,
    pub volume: f64,
    pub is_high: bool,
}

/// An open position exposed to Rhai.
#[derive(Debug, Clone)]
pub struct PositionWrapper {
    pub position: Position,
    pub current_price: f64,
}

impl PositionWrapper {
    pub fn new(position: Position, current_price: f64) -> Self {
        Self {
            position,
            current_price,
        }
    }
}

// ── Registration helpers ──────────────────────────────────────────────────────

fn price_array(cl: &mut CandleList, field: &str) -> rhai::Array {
    cl.candles
        .read()
        .unwrap()
        .iter()
        .rev()
        .map(|c| {
            Dynamic::from(match field {
                "close" => c.close,
                "open" => c.open,
                "high" => c.high,
                "low" => c.low,
                "volume" => c.volume,
                _ => unreachable!(),
            })
        })
        .collect()
}

/// Register all custom types and their fields/methods on the Rhai `Engine`.
pub fn register_types(engine: &mut Engine) {
    // ── CandleWrapper ─────────────────────────────────────────────────────
    engine.register_type_with_name::<CandleWrapper>("Candle");

    engine.register_get("open", |c: &mut CandleWrapper| c.candle.open);
    engine.register_get("high", |c: &mut CandleWrapper| c.candle.high);
    engine.register_get("low", |c: &mut CandleWrapper| c.candle.low);
    engine.register_get("close", |c: &mut CandleWrapper| c.candle.close);
    engine.register_get("volume", |c: &mut CandleWrapper| c.candle.volume);
    engine.register_get("timestamp", |c: &mut CandleWrapper| c.candle.timestamp);
    engine.register_get("symbol", |c: &mut CandleWrapper| c.candle.symbol.clone());
    engine.register_get("bar", |c: &mut CandleWrapper| c.bar as INT);

    engine.register_fn("body", |c: &mut CandleWrapper| c.candle.body());
    engine.register_fn("range", |c: &mut CandleWrapper| c.candle.range());

    // ── CandleList ────────────────────────────────────────────────────────
    engine.register_type_with_name::<CandleList>("CandleList");

    // candles[n]  — 1-indexed, newest first
    engine.register_indexer_get(|cl: &mut CandleList, idx: INT| -> Dynamic {
        if idx < 1 {
            return Dynamic::UNIT;
        }
        let candles = cl.candles.read().unwrap();
        let n = candles.len();
        match n.checked_sub(idx as usize) {
            Some(i) => Dynamic::from(CandleWrapper::new(candles[i].clone(), i as u64)),
            None => Dynamic::UNIT,
        }
    });

    // candles.len()
    engine.register_fn("len", |cl: &mut CandleList| -> INT {
        cl.candles.read().unwrap().len() as INT
    });

    // Price-series helpers — newest first, consistent with candles[1]
    engine.register_fn("closes", |cl: &mut CandleList| price_array(cl, "close"));
    engine.register_fn("opens", |cl: &mut CandleList| price_array(cl, "open"));
    engine.register_fn("highs", |cl: &mut CandleList| price_array(cl, "high"));
    engine.register_fn("lows", |cl: &mut CandleList| price_array(cl, "low"));
    engine.register_fn("volumes", |cl: &mut CandleList| price_array(cl, "volume"));

    // ── PositionWrapper ───────────────────────────────────────────────────
    engine.register_type_with_name::<PositionWrapper>("Position");

    engine.register_get("side", |p: &mut PositionWrapper| {
        p.position.side.to_string()
    });
    engine.register_get("entry_price", |p: &mut PositionWrapper| {
        p.position.entry_price
    });
    engine.register_get("size", |p: &mut PositionWrapper| p.position.size);
    engine.register_get("entry_time", |p: &mut PositionWrapper| {
        p.position.entry_time
    });
    engine.register_get("stop_loss", |p: &mut PositionWrapper| -> Dynamic {
        p.position
            .stop_loss
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });
    engine.register_get("take_profit", |p: &mut PositionWrapper| -> Dynamic {
        p.position
            .take_profit
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });

    // PnL and Value calculated from current_price
    engine.register_fn("pnl", |p: &mut PositionWrapper| -> Dynamic {
        let pnl = match p.position.side {
            PositionSide::Long => (p.current_price - p.position.entry_price) * p.position.size,
            PositionSide::Short => (p.position.entry_price - p.current_price) * p.position.size,
        };
        Dynamic::from(pnl)
    });
    engine.register_fn("value", |p: &mut PositionWrapper| {
        Dynamic::from(p.position.size * p.current_price)
    });

    // ── ContextWrapper ────────────────────────────────────────────────────
    engine.register_type_with_name::<ContextWrapper>("Context");

    engine.register_get("balance", |c: &mut ContextWrapper| c.ctx.balance);
    engine.register_get("equity", |c: &mut ContextWrapper| c.ctx.equity);
    engine.register_get("trades_count", |c: &mut ContextWrapper| {
        c.ctx.trades_count as INT
    });
    engine.register_get("position", |c: &mut ContextWrapper| -> Dynamic {
        match &c.ctx.position {
            None => Dynamic::UNIT,
            Some(pos) => Dynamic::from(PositionWrapper::new(pos.clone(), c.current_price)),
        }
    });
    engine.register_fn("has_position", |c: &mut ContextWrapper| {
        c.ctx.has_position()
    });

    // State API — persistent key-value store between ticks
    // Read state as integer
    engine.register_fn(
        "state",
        |c: &mut ContextWrapper, name: &str, default_val: INT| -> Dynamic {
            let state = c.state.read().unwrap();
            match state.get(name) {
                Some(v) => v.clone(),
                None => Dynamic::from(default_val),
            }
        },
    );

    // Read state as float
    engine.register_fn(
        "state_f",
        |c: &mut ContextWrapper, name: &str, default_val: f64| -> Dynamic {
            let state = c.state.read().unwrap();
            match state.get(name) {
                Some(v) => v.clone(),
                None => Dynamic::from(default_val),
            }
        },
    );

    // Set state with integer value
    engine.register_fn(
        "set_state",
        |c: &mut ContextWrapper, name: &str, value: INT| {
            let mut state = c.state.write().unwrap();
            state.insert(name.to_string(), Dynamic::from(value));
        },
    );

    // Set state with float value
    engine.register_fn(
        "set_state_f",
        |c: &mut ContextWrapper, name: &str, value: f64| {
            let mut state = c.state.write().unwrap();
            state.insert(name.to_string(), Dynamic::from(value));
        },
    );

    // Anchored outputs — strategies access via `ctx.anchored("name")` etc.
    engine.register_fn(
        "anchored",
        |c: &mut ContextWrapper, name: &str| -> Dynamic {
            match c.anchored.values.get(name) {
                None => Dynamic::UNIT,
                Some(AnchoredOutput::Slope(Some(v))) => Dynamic::from(*v),
                Some(AnchoredOutput::Slope(None)) => Dynamic::UNIT,
                Some(AnchoredOutput::Trendlines(lines)) => {
                    let arr: rhai::Array = lines
                        .iter()
                        .map(|l| Dynamic::from(TrendLineRhai(*l)))
                        .collect();
                    Dynamic::from(arr)
                }
            }
        },
    );
    engine.register_fn(
        "last_pivot",
        |c: &mut ContextWrapper, detector_id: &str, side: &str| -> Dynamic {
            let (map, is_high) = match side.to_ascii_lowercase().as_str() {
                "high" => (&c.anchored.last_pivot_high, true),
                "low" => (&c.anchored.last_pivot_low, false),
                _ => return Dynamic::UNIT,
            };
            match map.get(detector_id) {
                None => Dynamic::UNIT,
                Some(&(bar, price, volume)) => Dynamic::from(PivotEventRhai {
                    bar,
                    price,
                    volume,
                    is_high,
                }),
            }
        },
    );

    // ── TrendLineRhai ────────────────────────────────────────────────────
    engine.register_type_with_name::<TrendLineRhai>("TrendLine");
    engine.register_get("slope", |t: &mut TrendLineRhai| t.0.slope);
    engine.register_get("intercept", |t: &mut TrendLineRhai| t.0.intercept);
    engine.register_get("touches", |t: &mut TrendLineRhai| t.0.touches as INT);
    engine.register_get("anchor_start_bar", |t: &mut TrendLineRhai| {
        t.0.anchor_start_bar as INT
    });
    engine.register_get("anchor_end_bar", |t: &mut TrendLineRhai| {
        t.0.anchor_end_bar as INT
    });
    engine.register_get("side", |t: &mut TrendLineRhai| match t.0.side {
        indicators::anchored::evaluators::TrendlineSide::Resistance => "resistance".to_string(),
        indicators::anchored::evaluators::TrendlineSide::Support => "support".to_string(),
    });
    engine.register_fn("y_at", |t: &mut TrendLineRhai, bar: INT| {
        t.0.y_at(bar.max(0) as u64)
    });

    // ── PivotEventRhai ───────────────────────────────────────────────────
    engine.register_type_with_name::<PivotEventRhai>("PivotEvent");
    engine.register_get("bar", |p: &mut PivotEventRhai| p.bar as INT);
    engine.register_get("price", |p: &mut PivotEventRhai| p.price);
    engine.register_get("volume", |p: &mut PivotEventRhai| p.volume);
    engine.register_get("side", |p: &mut PivotEventRhai| {
        if p.is_high {
            "high".to_string()
        } else {
            "low".to_string()
        }
    });
}
