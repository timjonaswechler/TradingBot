use crate::Timeframe;
use serde::{Deserialize, Serialize};

/// A single completed OHLCV candlestick.
///
/// `timestamp` is the Unix millisecond open/start boundary of the completed
/// candle interval. `volume` is `f64` to accommodate both integer equity volumes
/// and fractional crypto volumes (e.g. 0.5 BTC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    /// Unix timestamp in milliseconds at the candle interval open/start boundary.
    pub timestamp: i64,
    /// Ticker symbol, e.g. `"AAPL"` or `"BTC-USD"`.
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Fractional-safe volume (covers stocks + crypto).
    pub volume: f64,
    /// Canonical candle timeframe, e.g. `1m`, `5m`, `1h`, `1d`.
    pub timeframe: Timeframe,
}

impl Candle {
    /// Derived close/end boundary of the completed candle interval.
    ///
    /// Candle timestamps identify the open/start boundary. Close-boundary logic
    /// should derive this value instead of treating `timestamp` as a close time.
    /// The calculation saturates at `i64::MAX` rather than plumbing arithmetic
    /// errors through every market-data call site.
    pub fn close_time(&self) -> i64 {
        self.timestamp.saturating_add(self.timeframe.duration_ms())
    }

    /// Absolute size of the candle body (`|close - open|`).
    pub fn body(&self) -> f64 {
        (self.close - self.open).abs()
    }

    /// Full high-to-low range of the candle.
    pub fn range(&self) -> f64 {
        self.high - self.low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Candle {
        Candle {
            timestamp: 1_700_000_000_000,
            symbol: "AAPL".into(),
            open: 100.0,
            high: 110.0,
            low: 90.0,
            close: 105.0,
            volume: 1_000.0,
            timeframe: Timeframe::days(1),
        }
    }

    #[test]
    fn body_is_abs_close_minus_open() {
        assert_eq!(sample().body(), 5.0);
    }

    #[test]
    fn range_is_high_minus_low() {
        assert_eq!(sample().range(), 20.0);
    }

    #[test]
    fn close_time_is_open_timestamp_plus_timeframe_duration() {
        let candle = sample();

        assert_eq!(
            candle.close_time(),
            candle.timestamp + Timeframe::days(1).duration_ms()
        );
    }

    #[test]
    fn close_time_saturates_at_i64_max() {
        let candle = Candle {
            timestamp: i64::MAX - 1,
            timeframe: Timeframe::minutes(1),
            ..sample()
        };

        assert_eq!(candle.close_time(), i64::MAX);
    }
}
