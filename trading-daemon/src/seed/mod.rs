/// Seed orchestration: load historical candles for all configured
/// asset/interval combinations from Yahoo Finance into SpacetimeDB.
pub mod yahoo;

use anyhow::Result;
use db_layer::{
    count_candles, get_candle_timestamps, insert_candle, DbConnection, SpacetimeClient,
};
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::Config;

/// Parse an ISO 8601 date string ("2020-01-01") to Unix milliseconds.
fn date_to_ms(date_str: &str) -> anyhow::Result<i64> {
    // Simple parser: YYYY-MM-DD
    let parts: Vec<&str> = date_str.splitn(3, '-').collect();
    if parts.len() != 3 {
        anyhow::bail!("Invalid date format '{}' — expected YYYY-MM-DD", date_str);
    }
    let year: i64 = parts[0].parse()?;
    let month: i64 = parts[1].parse()?;
    let day: i64 = parts[2].parse()?;

    // Days since Unix epoch (1970-01-01) — simplified Gregorian calculation.
    let days = days_since_epoch(year, month, day);
    Ok(days * 86_400_000)
}

fn days_since_epoch(year: i64, month: i64, day: i64) -> i64 {
    let mut days: i64 = 0;
    for yr in 1970..year {
        days += if is_leap(yr) { 366 } else { 365 };
    }
    let month_days = [0i64, 31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for mo in 1..month {
        days += month_days[mo as usize];
        if mo == 2 && is_leap(year) {
            days += 1;
        }
    }
    days + day - 1
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Run the full seed process for all assets in the config.
pub async fn run(config: &Config, from_override: Option<String>) -> Result<()> {
    let from_str = from_override.as_deref().unwrap_or(&config.seed.from);

    let from_ms = date_to_ms(from_str).map_err(|e| anyhow::anyhow!("Invalid --from date: {e}"))?;

    info!(from = from_str, "Starting seed");

    // Connect to SpacetimeDB
    let client = SpacetimeClient::connect(&config.database.url, &config.database.module)?;
    let conn = client.conn; // already Arc<DbConnection>

    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    // Spawn parallel tasks for each asset × interval combination
    let mut handles = Vec::new();

    for asset in &config.assets {
        for interval in &asset.intervals {
            let conn_clone = conn.clone();
            let http_clone = http.clone();
            let symbol = asset.symbol.clone();
            let interval_clone = interval.clone();
            let from_ms_copy = from_ms;

            let handle = tokio::spawn(async move {
                if let Err(e) = seed_one(
                    conn_clone,
                    &http_clone,
                    &symbol,
                    &interval_clone,
                    from_ms_copy,
                )
                .await
                {
                    warn!(symbol, interval = interval_clone, error = %e, "Seed failed");
                }
                // Rate limiting: small delay between requests
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            });

            handles.push(handle);
        }
    }

    for handle in handles {
        let _ = handle.await;
    }

    info!("Seed complete.");
    Ok(())
}

/// Seed one symbol/interval combination.
async fn seed_one(
    conn: Arc<DbConnection>,
    http: &reqwest::Client,
    symbol: &str,
    interval: &str,
    from_ms: i64,
) -> Result<()> {
    // Fetch the full requested range from Yahoo, then drop timestamps we
    // already have in the cache. This lets us backfill older history when
    // the user passes a `--from` earlier than our oldest stored bar.
    let canonical_interval = yahoo::interval_timeframe(interval)?.to_string();
    let existing = count_candles(&*conn, symbol, &canonical_interval);
    let existing_ts = get_candle_timestamps(&*conn, symbol, &canonical_interval);

    info!(
        symbol,
        interval, canonical_interval, existing, from_ms, "Seeding"
    );

    let candles = yahoo::fetch_candles(http, symbol, interval, from_ms).await?;

    let fetched = candles.len();
    let new_candles: Vec<_> = candles
        .into_iter()
        .filter(|c| !existing_ts.contains(&c.timestamp))
        .collect();
    let skipped = fetched - new_candles.len();

    if new_candles.is_empty() {
        info!(
            symbol,
            interval, fetched, skipped, "No new candles to insert"
        );
        return Ok(());
    }

    // Insert in a blocking task (SDK is sync)
    let symbol_s = symbol.to_string();
    let interval_s = interval.to_string();
    let inserted = tokio::task::spawn_blocking(move || {
        let mut count = 0usize;
        for candle in &new_candles {
            match insert_candle(&*conn, candle, "yahoo") {
                Ok(_) => count += 1,
                Err(e) => warn!(
                    symbol = symbol_s,
                    interval = interval_s,
                    error = %e,
                    "Failed to insert candle"
                ),
            }
        }
        count
    })
    .await?;

    info!(
        symbol,
        interval, fetched, skipped, inserted, "Seed done for asset"
    );
    Ok(())
}
