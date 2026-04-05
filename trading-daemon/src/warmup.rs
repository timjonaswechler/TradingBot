/// Engine warmup: load historical candles from SpacetimeDB into the engine
/// before live ticking begins.
use anyhow::Result;
use tracing::{info, warn};

use db_layer::{get_candles_before, DbConnection};
use engine::Engine;

/// Load up to `warmup_bars` historical candles for `symbol`/`timeframe` from
/// the SpacetimeDB local cache and feed them into the engine.
///
/// Returns the number of candles actually loaded.
///
/// Warns (but does not fail) when fewer candles are available than requested —
/// this happens when the DB hasn't been seeded yet.
pub fn warmup_engine(
    conn:      &DbConnection,
    engine:    &mut Engine,
    symbol:    &str,
    timeframe: &str,
    warmup_bars: usize,
) -> Result<usize> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let candles = get_candles_before(
        conn,
        symbol,
        timeframe,
        now_ms,
        warmup_bars as u32,
    );

    let loaded = candles.len();

    if loaded == 0 {
        warn!(
            symbol, timeframe, warmup_bars,
            "No historical candles in DB — engine starts cold. \
             Run `trading-daemon seed` to populate historical data."
        );
        return Ok(0);
    }

    if loaded < warmup_bars {
        warn!(
            symbol, timeframe,
            available = loaded,
            requested = warmup_bars,
            "Fewer candles than requested — engine partially warmed. \
             Consider seeding more history."
        );
    }

    engine::warmup::warmup(engine, candles)
        .map_err(|e| anyhow::anyhow!("Engine warmup failed: {e}"))?;

    info!(symbol, timeframe, loaded, "Engine warmed up");
    Ok(loaded)
}
