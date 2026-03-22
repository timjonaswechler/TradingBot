use crate::market_data::Candle;
use super::ema;

/// Returns slope of a linear regression over the last `lookback` values ending at index i.
/// For i < lookback-1, returns 0.0.
/// Input: chronological order (oldest first).
pub fn compute(series: &[f64], lookback: usize) -> Vec<f64> {
    let n = series.len();
    let mut result = vec![0.0f64; n];

    if lookback < 2 || n < lookback {
        return result;
    }

    for i in (lookback - 1)..n {
        let window = &series[(i + 1 - lookback)..=i];
        result[i] = linear_regression_slope(window);
    }

    result
}

/// Returns second derivative (slope of slope).
pub fn compute_acceleration(series: &[f64], lookback: usize) -> Vec<f64> {
    let slopes = compute(series, lookback);
    compute(&slopes, lookback)
}

/// Compute True Range for each candle. Returns Vec<f64> same length. Oldest first.
/// Candle input is newest-first → internally reverse.
pub fn compute_atr(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.is_empty() {
        return vec![];
    }

    let n = candles.len();
    let mut tr = vec![0.0f64; n];

    // Candles come newest-first; iterate in reverse (oldest-first) for TR calculation
    let ordered: Vec<&Candle> = candles.iter().rev().collect();

    // First bar: no previous close, TR = high - low
    tr[0] = (ordered[0].high - ordered[0].low) as f64 / 100.0;

    for i in 1..n {
        let high      = ordered[i].high  as f64 / 100.0;
        let low       = ordered[i].low   as f64 / 100.0;
        let prev_close = ordered[i - 1].close as f64 / 100.0;
        tr[i] = (high - low).max((high - prev_close).abs()).max((low - prev_close).abs());
    }

    ema::compute(&tr, period)
}

/// slope = Σ((x - x̄)(y - ȳ)) / Σ((x - x̄)²), where x ∈ {0, 1, …, n-1}
fn linear_regression_slope(window: &[f64]) -> f64 {
    let n = window.len();
    if n < 2 {
        return 0.0;
    }

    let x_mean = (n as f64 - 1.0) / 2.0;
    let y_mean: f64 = window.iter().sum::<f64>() / n as f64;

    let (num, den) = window.iter().enumerate().fold((0.0f64, 0.0f64), |(num, den), (i, &y)| {
        let dx = i as f64 - x_mean;
        (num + dx * (y - y_mean), den + dx * dx)
    });

    if den == 0.0 { 0.0 } else { num / den }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slope_linear_series() {
        // y = 2x + 1 → slope must be exactly 2.0
        let data: Vec<f64> = (0..10).map(|i| 2.0 * i as f64 + 1.0).collect();
        let slopes = compute(&data, 4);
        for &s in &slopes[3..] {
            assert!((s - 2.0).abs() < 1e-8, "Expected slope ~2.0, got {}", s);
        }
    }

    #[test]
    fn test_slope_constant_series() {
        let data = vec![5.0; 10];
        let slopes = compute(&data, 3);
        for &s in &slopes[2..] {
            assert!(s.abs() < 1e-10, "Expected slope ~0.0, got {}", s);
        }
    }

    #[test]
    fn test_slope_length() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(compute(&data, 3).len(), data.len());
    }

    #[test]
    fn test_slope_early_values_are_zero() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let slopes = compute(&data, 3);
        assert_eq!(slopes[0], 0.0);
        assert_eq!(slopes[1], 0.0);
    }

    #[test]
    fn test_acceleration_length() {
        let data: Vec<f64> = (0..20).map(|i| i as f64).collect();
        assert_eq!(compute_acceleration(&data, 3).len(), data.len());
    }

    #[test]
    fn test_atr_length() {
        use chrono::Utc;
        // Build newest-first candles
        let candles: Vec<Candle> = (0..10i64)
            .rev()
            .map(|i| Candle {
                timestamp: Utc::now(),
                open:   10000 + i * 10,
                high:   10100 + i * 10,
                low:    9900  + i * 10,
                close:  10050 + i * 10,
                volume: 1000,
            })
            .collect();
        assert_eq!(compute_atr(&candles, 3).len(), candles.len());
    }

    #[test]
    fn test_atr_positive_values() {
        use chrono::Utc;
        let candles: Vec<Candle> = (0..10i64)
            .rev()
            .map(|i| Candle {
                timestamp: Utc::now(),
                open:   10000 + i * 10,
                high:   10100 + i * 10,
                low:    9900  + i * 10,
                close:  10050 + i * 10,
                volume: 1000,
            })
            .collect();
        let atr = compute_atr(&candles, 3);
        for &v in &atr[2..] {
            assert!(v > 0.0, "ATR should be positive, got {}", v);
        }
    }
}
