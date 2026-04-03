use serde::{Deserialize, Serialize};

/// A single OHLCV candlestick.
///
/// Timestamps are Unix milliseconds (`i64`).  
/// `volume` is `f64` to accommodate both integer equity volumes and
/// fractional crypto volumes (e.g. 0.5 BTC).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Candle {
    /// Unix timestamp in milliseconds (candle open time).
    pub timestamp: i64,
    /// Ticker symbol, e.g. `"AAPL"` or `"BTC-USD"`.
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    /// Fractional-safe volume (covers stocks + crypto).
    pub volume: f64,
    /// Timeframe string, e.g. `"1m"`, `"5m"`, `"1h"`, `"1d"`.
    pub timeframe: String,
}

impl Candle {
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
            timeframe: "1d".into(),
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
}
