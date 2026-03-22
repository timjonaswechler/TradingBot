use crate::market_data::Candle;
use super::{ema, slope};

pub struct MacdResult {
    /// MACD line = EMA(fast) - EMA(slow), chronological order (oldest first), len = candles.len()
    pub macd_line: Vec<f64>,
    /// Signal line = EMA(macd_line, signal_period), same len
    pub signal_line: Vec<f64>,
    /// Histogram = macd_line - signal_line, same len
    pub histogram: Vec<f64>,
    /// histogram[i] / close_prices[i]
    pub normalized_histogram: Vec<f64>,
}

/// Computes MACD from candles (newest-first input → internally reversed to oldest-first).
/// Returns MacdResult in chronological order (oldest first).
pub fn compute(candles: &[Candle], fast: usize, slow: usize, signal: usize) -> MacdResult {
    if candles.is_empty() {
        return MacdResult {
            macd_line: vec![],
            signal_line: vec![],
            histogram: vec![],
            normalized_histogram: vec![],
        };
    }

    let close_prices: Vec<f64> = candles.iter().rev().map(|c| c.close as f64 / 100.0).collect();

    let ema_fast = ema::compute(&close_prices, fast);
    let ema_slow = ema::compute(&close_prices, slow);

    let macd_line: Vec<f64> = ema_fast.iter().zip(&ema_slow).map(|(f, s)| f - s).collect();
    let signal_line = ema::compute(&macd_line, signal);
    let histogram: Vec<f64> = macd_line.iter().zip(&signal_line).map(|(m, s)| m - s).collect();

    let normalized_histogram: Vec<f64> = histogram
        .iter()
        .zip(&close_prices)
        .map(|(&h, &c)| if c == 0.0 { 0.0 } else { h / c })
        .collect();

    MacdResult { macd_line, signal_line, histogram, normalized_histogram }
}

pub struct SlopeAnalysis {
    pub macd_slope: Vec<f64>,
    pub signal_slope: Vec<f64>,
    pub histogram_slope: Vec<f64>,
    pub histogram_acceleration: Vec<f64>,
}

/// Computes slope analysis from a MacdResult.
/// All slopes are per-bar rate of change, normalized by close price.
pub fn compute_slope_analysis(
    result: &MacdResult,
    close_prices: &[f64],
    lookback: usize,
) -> SlopeAnalysis {
    let normalize = |raw: Vec<f64>| -> Vec<f64> {
        raw.iter()
            .zip(close_prices)
            .map(|(&s, &c)| if c == 0.0 { 0.0 } else { s / c })
            .collect()
    };

    SlopeAnalysis {
        macd_slope:             normalize(slope::compute(&result.macd_line,  lookback)),
        signal_slope:           normalize(slope::compute(&result.signal_line, lookback)),
        histogram_slope:        normalize(slope::compute(&result.histogram,   lookback)),
        histogram_acceleration: normalize(slope::compute_acceleration(&result.histogram, lookback)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_candles(closes: &[i64]) -> Vec<Candle> {
        closes.iter().rev().map(|&c| Candle {
            timestamp: Utc::now(),
            open: c, high: c + 10, low: c - 10, close: c, volume: 1000,
        }).collect()
    }

    #[test]
    fn test_macd_histogram_equals_macd_minus_signal() {
        let closes: Vec<i64> = (1..=50).map(|i| i * 100).collect();
        let result = compute(&make_candles(&closes), 12, 26, 9);

        assert_eq!(result.macd_line.len(), closes.len());
        assert_eq!(result.signal_line.len(), closes.len());
        assert_eq!(result.histogram.len(), closes.len());

        for i in 0..result.histogram.len() {
            let expected = result.macd_line[i] - result.signal_line[i];
            assert!(
                (result.histogram[i] - expected).abs() < 1e-10,
                "histogram[{i}] mismatch: {} != {expected}", result.histogram[i],
            );
        }
    }

    #[test]
    fn test_macd_normalized_histogram() {
        let closes: Vec<i64> = (100..=150).map(|i| i * 100).collect();
        let result = compute(&make_candles(&closes), 3, 5, 2);
        let close_prices: Vec<f64> = closes.iter().map(|&c| c as f64 / 100.0).collect();

        for (i, (&norm, &close)) in result.normalized_histogram.iter().zip(&close_prices).enumerate() {
            if close != 0.0 {
                let expected = result.histogram[i] / close;
                assert!((norm - expected).abs() < 1e-10, "normalized_histogram[{i}] mismatch");
            }
        }
    }

    #[test]
    fn test_macd_lengths() {
        let closes: Vec<i64> = (1..=30).map(|i| i * 100).collect();
        let result = compute(&make_candles(&closes), 3, 5, 2);

        assert_eq!(result.macd_line.len(), closes.len());
        assert_eq!(result.signal_line.len(), closes.len());
        assert_eq!(result.histogram.len(), closes.len());
        assert_eq!(result.normalized_histogram.len(), closes.len());
    }

    #[test]
    fn test_macd_empty_candles() {
        let result = compute(&[], 12, 26, 9);
        assert!(result.macd_line.is_empty());
        assert!(result.signal_line.is_empty());
        assert!(result.histogram.is_empty());
        assert!(result.normalized_histogram.is_empty());
    }

    #[test]
    fn test_slope_analysis_lengths() {
        let closes: Vec<i64> = (1..=30).map(|i| i * 100).collect();
        let result = compute(&make_candles(&closes), 3, 5, 2);
        let close_prices: Vec<f64> = closes.iter().map(|&c| c as f64 / 100.0).collect();

        let analysis = compute_slope_analysis(&result, &close_prices, 3);
        assert_eq!(analysis.macd_slope.len(), closes.len());
        assert_eq!(analysis.signal_slope.len(), closes.len());
        assert_eq!(analysis.histogram_slope.len(), closes.len());
        assert_eq!(analysis.histogram_acceleration.len(), closes.len());
    }
}
