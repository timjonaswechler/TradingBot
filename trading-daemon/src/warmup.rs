/// Engine warmup: load historical candles from SpacetimeDB into the engine
/// before live ticking begins.
use anyhow::Result;
use tracing::{info, warn};

use db_layer::{get_candles_before, DbConnection};
use engine::Engine;

/// Result of a warmup pass: how many candles were loaded and the timestamp
/// of the newest one. The caller uses `high_water_ts` as an `on_insert`
/// filter so the first replayed candle doesn't double-count a bar the
/// engine has already seen.
pub struct WarmupResult {
    pub loaded: usize,
    pub high_water_ts: Option<i64>,
}

/// Load up to `warmup_bars` historical candles for `symbol`/`timeframe` from
/// the SpacetimeDB local cache and feed them into the engine.
///
/// Uses `i64::MAX` as the upper bound — every candle currently in the cache
/// is considered. The newest timestamp seen is returned so the live-tick
/// path can drop redundant replays.
///
/// Warns (but does not fail) when fewer candles are available than requested.
pub fn warmup_engine(
    conn: &DbConnection,
    engine: &mut Engine,
    symbol: &str,
    timeframe: &str,
    warmup_bars: usize,
) -> Result<WarmupResult> {
    let candles = get_candles_before(conn, symbol, timeframe, i64::MAX, warmup_bars as u32);

    let loaded = candles.len();
    let high_water_ts = candles.last().map(|c| c.timestamp);

    if loaded == 0 {
        warn!(
            symbol,
            timeframe,
            warmup_bars,
            "No historical candles in DB — engine starts cold. \
             Run `trading-daemon seed` to populate historical data."
        );
        return Ok(WarmupResult {
            loaded: 0,
            high_water_ts: None,
        });
    }

    if loaded < warmup_bars {
        warn!(
            symbol,
            timeframe,
            available = loaded,
            requested = warmup_bars,
            "Fewer candles than requested — engine partially warmed. \
             Consider seeding more history."
        );
    }

    engine::warmup::warmup(engine, candles)
        .map_err(|e| anyhow::anyhow!("Engine warmup failed: {e}"))?;

    info!(symbol, timeframe, loaded, high_water_ts, "Engine warmed up");
    Ok(WarmupResult {
        loaded,
        high_water_ts,
    })
}
