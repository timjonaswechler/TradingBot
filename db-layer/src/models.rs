/// Conversion helpers between generated `module_bindings` types and `shared::` types.
///
/// The generated `module_bindings::Candle`, `LivePosition`, `LiveTrade` structs
/// are the canonical DB types.  `shared::Candle` / `shared::Position` are the
/// lightweight in-memory types used by the engine and backtester.
use shared::{Candle, Position, PositionSide, Timeframe};

use crate::module_bindings::{Candle as DbCandle, LivePosition};

// ── Candle ────────────────────────────────────────────────────────────────────

/// Build the deterministic dedup key: `"{symbol}_{timeframe}_{timestamp_ms}"`.
pub fn canonical_id(symbol: &str, timeframe: &str, timestamp_ms: i64) -> String {
    format!("{symbol}_{timeframe}_{timestamp_ms}")
}

/// Convert a `shared::Candle` + provider into the args needed by the `insert_candle` reducer.
/// Returns `(canonical_id, timestamp, symbol, open, high, low, close, volume, timeframe, provider)`.
pub fn candle_to_reducer_args(
    c: &Candle,
    provider: &str,
) -> (String, i64, String, f64, f64, f64, f64, f64, String, String) {
    (
        canonical_id(&c.symbol, &c.timeframe.to_string(), c.timestamp),
        c.timestamp,
        c.symbol.clone(),
        c.open,
        c.high,
        c.low,
        c.close,
        c.volume,
        c.timeframe.to_string(),
        provider.to_string(),
    )
}

/// Convert a DB `Candle` (from generated bindings) to a `shared::Candle`.
pub fn db_candle_to_shared(c: DbCandle) -> Candle {
    Candle {
        timestamp: c.timestamp,
        symbol: c.symbol,
        open: c.open,
        high: c.high,
        low: c.low,
        close: c.close,
        volume: c.volume,
        timeframe: c
            .timeframe
            .parse::<Timeframe>()
            .expect("DB candle timeframe should be canonical"),
    }
}

// ── LivePosition ──────────────────────────────────────────────────────────────

/// Convert a DB `LivePosition` to `(id, strategy, shared::Position)`.
pub fn db_position_to_shared(p: LivePosition) -> (u64, String, Position) {
    let side = if p.side == "long" {
        PositionSide::Long
    } else {
        PositionSide::Short
    };
    let pos = Position {
        symbol: p.symbol,
        side,
        entry_price: p.entry_price,
        size: p.size,
        entry_time: p.entry_time,
        stop_loss: (p.stop_loss != 0.0).then_some(p.stop_loss),
        take_profit: (p.take_profit != 0.0).then_some(p.take_profit),
    };
    (p.id, p.strategy, pos)
}

// ── LiveTrade (re-exported for convenience) ────────────────────────────────────
pub use crate::module_bindings::LiveTrade as DbTrade;
