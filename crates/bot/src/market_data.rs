use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct Candle {
    pub timestamp: DateTime<Utc>,
    pub open:      i64, // in Cent
    pub high:      i64,
    pub low:       i64,
    pub close:     i64,
    pub volume:    i64,
}

// Yahoo Finance API Response
#[derive(Debug, Deserialize)]
struct YfResponse {
    chart: YfChart,
}

#[derive(Debug, Deserialize)]
struct YfChart {
    result: Option<Vec<YfResult>>,
    error:  Option<YfError>,
}

#[derive(Debug, Deserialize)]
struct YfError {
    code:        String,
    description: String,
}

#[derive(Debug, Deserialize)]
struct YfResult {
    #[serde(default)]
    timestamp:  Option<Vec<i64>>,
    #[serde(default)]
    indicators: YfIndicators,
}

#[derive(Debug, Default, Deserialize)]
struct YfIndicators {
    #[serde(default)]
    quote: Vec<YfQuote>,
}

#[derive(Debug, Default, Deserialize)]
struct YfQuote {
    #[serde(default)]
    open:   Vec<Option<f64>>,
    #[serde(default)]
    high:   Vec<Option<f64>>,
    #[serde(default)]
    low:    Vec<Option<f64>>,
    #[serde(default)]
    close:  Vec<Option<f64>>,
    #[serde(default)]
    volume: Vec<Option<f64>>,
}

/// Liefert den frühesten erlaubten Unix-Timestamp für ein Intervall (mit Sicherheitspuffer).
/// Intraday-Intervalle haben bei Yahoo Finance ein hartes History-Limit.
pub fn period1_for_interval(interval: &str) -> i64 {
    let now = chrono::Utc::now().timestamp();
    match interval {
        "1m"                  => now -   6 * 86_400,  // Limit ~7d, Puffer 1d
        "2m" | "5m"           => now -  59 * 86_400,  // Limit ~60d, Puffer 1d
        "15m" | "30m"         => now -  59 * 86_400,
        "60m" | "1h"          => now - 729 * 86_400,  // Limit ~730d, Puffer 1d
        "90m"                 => now -  59 * 86_400,
        _                     => 0, // EOD: Epoch = komplette verfügbare History
    }
}

/// Vollständiger Erstabzug: holt historische OHLCV-Daten von Yahoo Finance.
/// Für EOD-Intervalle (1d, 1wk, …) wird period1=0 gesetzt um die komplette
/// verfügbare History zu laden — `range=max` liefert Yahoo nur ~167 Punkte.
pub async fn fetch_history(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    _range: &str, // wird ignoriert, period1/period2 ist zuverlässiger
) -> Result<Vec<Candle>> {
    let period1 = period1_for_interval(interval);
    let period2 = chrono::Utc::now().timestamp();
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, interval, period1, period2
    );
    fetch_url(client, &url).await
}

/// Inkrementelles Update: holt nur Candles seit `since_ts` (Unix-Timestamp).
/// `since_ts` wird auf das erlaubte History-Limit des Intervalls geclampt —
/// verhindert Fehler wenn die DB einen sehr alten Timestamp enthält.
pub async fn fetch_since(
    client: &reqwest::Client,
    symbol: &str,
    interval: &str,
    since_ts: i64,
) -> Result<Vec<Candle>> {
    let now      = Utc::now().timestamp();
    let earliest = period1_for_interval(interval);
    let period1  = since_ts.max(earliest);
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&period1={}&period2={}",
        symbol, interval, period1, now
    );
    fetch_url(client, &url).await
}

async fn fetch_url(client: &reqwest::Client, url: &str) -> Result<Vec<Candle>> {
    let text = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .context("Yahoo Finance request fehlgeschlagen")?
        .text()
        .await
        .context("Yahoo Finance response lesen fehlgeschlagen")?;

    let response: YfResponse = serde_json::from_str(&text)
        .with_context(|| format!("Yahoo Finance JSON parsing fehlgeschlagen: {}", &text[..text.len().min(200)]))?;

    // Expliziter Fehler von Yahoo Finance (z.B. "Not Found", "Too Many Requests")
    if let Some(err) = response.chart.error {
        anyhow::bail!("Yahoo Finance Fehler: {} – {}", err.code, err.description);
    }

    let result = response
        .chart
        .result
        .unwrap_or_default()
        .into_iter()
        .next()
        .context("Keine Daten von Yahoo Finance (leeres result)")?;

    let quote = result
        .indicators
        .quote
        .into_iter()
        .next()
        .context("Keine Quote-Daten")?;

    let timestamps = result.timestamp.unwrap_or_default();
    if timestamps.is_empty() {
        return Ok(vec![]);
    }

    let candles = timestamps
        .into_iter()
        .enumerate()
        .filter_map(|(i, ts)| {
            let open   = (quote.open.get(i)?.as_ref()?.clone()   * 100.0) as i64;
            let high   = (quote.high.get(i)?.as_ref()?.clone()   * 100.0) as i64;
            let low    = (quote.low.get(i)?.as_ref()?.clone()    * 100.0) as i64;
            let close  = (quote.close.get(i)?.as_ref()?.clone()  * 100.0) as i64;
            let volume = quote.volume.get(i)?.unwrap_or(0.0) as i64;

            Some(Candle {
                timestamp: DateTime::from_timestamp(ts, 0)?,
                open,
                high,
                low,
                close,
                volume,
            })
        })
        .collect();

    Ok(candles)
}
