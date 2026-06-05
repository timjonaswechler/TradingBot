/// Conversion helpers between generated `module_bindings` types and `domain::` types.
///
/// The generated `module_bindings::Candle`, `LivePosition`, `LiveTrade`,
/// `PaperOpenPosition`, and `PaperTrade` structs are the canonical DB types.
/// `domain::Candle` / `domain::OpenPosition` are the lightweight in-memory
/// values used by runners/adapters.
use domain::{Candle, EntryRiskParameters, OpenPosition, PositionSide, Timeframe};

use crate::{
    error::DbError,
    module_bindings::{Candle as DbCandle, LivePosition},
};

// ── Candle ────────────────────────────────────────────────────────────────────

/// Build the deterministic dedup key: `"{symbol}_{timeframe}_{timestamp_ms}"`.
pub fn canonical_id(symbol: &str, timeframe: &str, timestamp_ms: i64) -> String {
    format!("{symbol}_{timeframe}_{timestamp_ms}")
}

/// Convert a `domain::Candle` + provider into the args needed by the `insert_candle` reducer.
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

impl TryFrom<DbCandle> for Candle {
    type Error = DbError;

    fn try_from(c: DbCandle) -> Result<Self, Self::Error> {
        let timeframe =
            c.timeframe
                .parse::<Timeframe>()
                .map_err(|source| DbError::InvalidCandleTimeframe {
                    timeframe: c.timeframe.clone(),
                    canonical_id: c.canonical_id.clone(),
                    symbol: c.symbol.clone(),
                    timestamp: c.timestamp,
                    source,
                })?;

        Ok(Candle {
            timestamp: c.timestamp,
            symbol: c.symbol,
            open: c.open,
            high: c.high,
            low: c.low,
            close: c.close,
            volume: c.volume,
            timeframe,
        })
    }
}

/// Fallibly convert a DB `Candle` (from generated bindings) to a `domain::Candle`.
pub fn db_candle_to_domain_candle(c: DbCandle) -> Result<Candle, DbError> {
    c.try_into()
}

// ── LivePosition ──────────────────────────────────────────────────────────────

/// Convert a DB `LivePosition` to `(id, strategy, domain::OpenPosition)`.
pub fn db_position_to_shared(p: LivePosition) -> (u64, String, OpenPosition) {
    let side = if p.side == "long" {
        PositionSide::Long
    } else {
        PositionSide::Short
    };
    let pos = OpenPosition {
        symbol: p.symbol,
        side,
        entry_price: p.entry_price,
        quantity: p.size,
        entry_time: p.entry_time,
        entry_risk: EntryRiskParameters {
            stop_loss: (p.stop_loss != 0.0).then_some(p.stop_loss),
            take_profit: (p.take_profit != 0.0).then_some(p.take_profit),
        },
    };
    (p.id, p.strategy, pos)
}

// ── Generated DB records (re-exported for convenience) ────────────────────────
pub use crate::module_bindings::{
    LiveTrade as DbTrade, PaperExitKind, PaperOpenPosition, PaperTrade,
};
