/// Yahoo Finance HTTP client for fetching historical OHLCV candles.
///
/// Uses the unofficial Yahoo Finance v8 chart API.  Filters out the last
/// (incomplete) candle when the current candle period has not yet closed.
use anyhow::{anyhow, Result};
use domain::{Candle, Timeframe};
use serde::Deserialize;

/// Yahoo Finance chart API response.
#[derive(Debug, Deserialize)]
struct YahooResponse {
    chart: YahooChart,
}

#[derive(Debug, Deserialize)]
struct YahooChart {
    result: Option<Vec<YahooResult>>,
    error: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct YahooResult {
    timestamp: Option<Vec<i64>>,
    indicators: YahooIndicators,
}

#[derive(Debug, Deserialize)]
struct YahooIndicators {
    quote: Vec<YahooQuote>,
}

#[derive(Debug, Deserialize)]
struct YahooQuote {
    open: Vec<Option<f64>>,
    high: Vec<Option<f64>>,
    low: Vec<Option<f64>>,
    close: Vec<Option<f64>>,
    volume: Vec<Option<f64>>,
}

// ── Interval helpers ──────────────────────────────────────────────────────────

/// Parse a Yahoo interval into the canonical runtime/domain timeframe.
pub fn interval_timeframe(interval: &str) -> Result<Timeframe> {
    match interval {
        "1wk" => Ok(Timeframe::weeks(1)),
        canonical => canonical
            .parse()
            .map_err(|e| anyhow!("Unsupported Yahoo interval '{interval}': {e}")),
    }
}

/// Convert a timeframe string to milliseconds.
pub fn interval_ms(interval: &str) -> Result<i64> {
    Ok(interval_timeframe(interval)?.duration_ms())
}

// ── Main fetch function ───────────────────────────────────────────────────────

/// Fetch historical candles for `symbol` / `interval` from Yahoo Finance.
///
/// Returns candles with `timestamp >= from_ts_ms`, with the last (incomplete)
/// candle filtered out if the current period has not yet closed.
pub async fn fetch_candles(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    from_ts_ms: i64,
) -> Result<Vec<Candle>> {
    let now_ms = chrono_now_ms();
    let period1 = from_ts_ms / 1000; // Yahoo uses seconds
    let period2 = now_ms / 1000;

    // Yahoo Finance v8 chart endpoint
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{symbol}\
         ?interval={interval}&period1={period1}&period2={period2}&includePrePost=false"
    );

    tracing::debug!(url = %url, "Fetching from Yahoo Finance");

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(anyhow!(
            "Yahoo Finance returned {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        ));
    }

    let body: YahooResponse = resp
        .json()
        .await
        .map_err(|e| anyhow!("Failed to parse Yahoo response: {e}"))?;

    if let Some(err) = body.chart.error {
        return Err(anyhow!("Yahoo Finance error: {err}"));
    }

    let result = body
        .chart
        .result
        .and_then(|r| r.into_iter().next())
        .ok_or_else(|| anyhow!("Yahoo Finance returned empty result for {symbol}/{interval}"))?;

    let timestamps = result
        .timestamp
        .ok_or_else(|| anyhow!("No timestamps in Yahoo response"))?;

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("No quote data in Yahoo response"))?;

    let timeframe = interval_timeframe(interval)?;
    let int_ms = interval_ms(interval)?;
    let symbol_str = symbol.to_string();

    let mut candles: Vec<Candle> = timestamps
        .iter()
        .enumerate()
        .filter_map(|(i, &ts_sec)| {
            let ts_ms = ts_sec * 1000;

            // Skip candles before requested start.
            if ts_ms < from_ts_ms {
                return None;
            }

            // Filter incomplete candle: if candle hasn't closed yet.
            if ts_ms + int_ms > now_ms {
                return None;
            }

            let open = quote.open.get(i)?.as_ref().copied()?;
            let high = quote.high.get(i)?.as_ref().copied()?;
            let low = quote.low.get(i)?.as_ref().copied()?;
            let close = quote.close.get(i)?.as_ref().copied()?;
            let volume = quote.volume.get(i)?.unwrap_or(0.0);

            // Skip candles with zero/invalid data.
            if close <= 0.0 || high <= 0.0 {
                return None;
            }

            Some(Candle {
                timestamp: ts_ms,
                symbol: symbol_str.clone(),
                open,
                high,
                low,
                close,
                volume,
                timeframe,
            })
        })
        .collect();

    candles.sort_by_key(|c| c.timestamp);
    tracing::info!(
        symbol,
        interval,
        count = candles.len(),
        "Fetched candles from Yahoo"
    );
    Ok(candles)
}

fn chrono_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}
