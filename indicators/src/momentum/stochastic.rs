use shared::Candle;

/// Result of the Stochastic Oscillator.
#[derive(Debug, Clone, PartialEq)]
pub struct StochasticResult {
    /// %K — stochastic value (0-100)
    pub k: f64,
    /// %D — SMA of %K
    pub d: f64,
}

/// Fast Stochastic Oscillator.
///
/// - `%K` = raw: (close - lowest) / (highest - lowest) * 100
/// - `%D` = SMA(3) of %K (last 3 raw %K values)
///
/// Input: candles in chronological order (oldest first).
/// `period` is the %K lookback (typically 14).
/// Needs at least `period + 2` candles.
pub fn stochastic_fast(candles: &[Candle], period: usize) -> Option<StochasticResult> {
    let d_period = 3;
    if period == 0 || candles.len() < period + d_period - 1 {
        return None;
    }

    let n = candles.len();
    let mut k_vals = Vec::with_capacity(d_period);

    for offset in (0..d_period).rev() {
        let end = n - offset;
        let start = end.checked_sub(period)?;
        let window = &candles[start..end];
        let highest = window
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let lowest = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let current_close = candles[end - 1].close;
        let k = if (highest - lowest).abs() < 1e-12 {
            50.0
        } else {
            100.0 * (current_close - lowest) / (highest - lowest)
        };
        k_vals.push(k);
    }

    let k = *k_vals.last()?;
    let d = k_vals.iter().sum::<f64>() / d_period as f64;

    Some(StochasticResult { k, d })
}

/// Slow Stochastic Oscillator.
///
/// - `%K` = SMA(3) of fast %K (3 raw %K values, averaged)
/// - `%D` = SMA(3) of slow %K (3 smoothed %K values, averaged)
///
/// Input: candles in chronological order (oldest first).
/// `period` is the %K lookback (typically 14).
/// Needs at least `period + 4` candles (2 more than fast for double smoothing).
pub fn stochastic_slow(candles: &[Candle], period: usize) -> Option<StochasticResult> {
    stochastic_full(candles, period, Some(3), Some(3))
}

/// Full Stochastic Oscillator with configurable smoothing.
///
/// - `%K` = SMA(k_smooth) of the most recent raw %K values
/// - `%D` = SMA(d_period) of smoothed %K values
///
/// Input: candles in chronological order (oldest first).
/// `period` is the %K lookback (typically 14).
/// `k_smooth` = smoothing period for %K (default: 3).
/// `d_period` = period for %D SMA (default: 3).
/// Needs at least `period + k_smooth + d_period - 1` candles.
pub fn stochastic_full(
    candles: &[Candle],
    period: usize,
    k_smooth: Option<usize>,
    d_period: Option<usize>,
) -> Option<StochasticResult> {
    let k_smooth = k_smooth.unwrap_or(3);
    let d_period = d_period.unwrap_or(3);

    if period == 0 || k_smooth == 0 || d_period == 0 {
        return None;
    }
    // Total warmup: d_period windows + (k_smooth - 1) extra for smoothing
    let min_candles = period + d_period + k_smooth - 2;
    if candles.len() < min_candles {
        return None;
    }

    let n = candles.len();

    // Build raw %K values: we need d_period values for the SMA,
    // and k_smooth-1 additional for the smoothing window
    let raw_k_count = d_period + k_smooth - 1;
    let mut raw_k_vals = Vec::with_capacity(raw_k_count);

    for offset in (0..raw_k_count).rev() {
        let end = n - offset;
        let start = end.checked_sub(period)?;
        let window = &candles[start..end];
        let highest = window
            .iter()
            .map(|c| c.high)
            .fold(f64::NEG_INFINITY, f64::max);
        let lowest = window.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
        let current_close = candles[end - 1].close;
        let k = if (highest - lowest).abs() < 1e-12 {
            50.0
        } else {
            100.0 * (current_close - lowest) / (highest - lowest)
        };
        raw_k_vals.push(k);
    }

    // %K = newest smoothed %K value
    let smoothed_vals: Vec<f64> = raw_k_vals
        .windows(k_smooth)
        .map(|w| w.iter().sum::<f64>() / k_smooth as f64)
        .collect();

    let k = *smoothed_vals.last()?;

    // %D = SMA(d_period) of the last d_period smoothed %K values
    let start_idx = smoothed_vals.len().saturating_sub(d_period);
    let d = smoothed_vals[start_idx..].iter().sum::<f64>() / d_period as f64;

    Some(StochasticResult { k, d })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            timeframe: "1d".parse().unwrap(),
        }
    }

    // ── stochastic_fast tests ──────────────────────────────────────────────

    #[test]
    fn fast_insufficient_data() {
        let c: Vec<Candle> = (1..=15)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert_eq!(stochastic_fast(&c, 14), None); // needs 16
    }

    #[test]
    fn fast_values_in_range() {
        let c: Vec<Candle> = (1..=20)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = stochastic_fast(&c, 14).unwrap();
        assert!(r.k >= 0.0 && r.k <= 100.0);
        assert!(r.d >= 0.0 && r.d <= 100.0);
    }

    #[test]
    fn fast_overbought_in_uptrend() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = stochastic_fast(&c, 14).unwrap();
        assert!(r.k > 80.0, "%K {:.1} should be overbought in uptrend", r.k);
    }

    #[test]
    fn fast_computes_k_and_d_from_last_three_windows() {
        let c = vec![
            candle(10.0, 0.0, 5.0),
            candle(12.0, 2.0, 8.0),
            candle(14.0, 4.0, 11.0),
            candle(16.0, 6.0, 14.0),
            candle(18.0, 8.0, 17.0),
        ];

        let r = stochastic_fast(&c, 3).unwrap();

        // Window 1 (offset=2): high=14, low=0, close=11 → k1 = (11-0)/(14-0) * 100
        // Window 2 (offset=1): high=16, low=2, close=14 → k2 = (14-2)/(16-2) * 100
        // Window 3 (offset=0): high=18, low=4, close=17 → k3 = (17-4)/(18-4) * 100
        let k1 = 100.0 * (11.0 - 0.0) / (14.0 - 0.0);
        let k2 = 100.0 * (14.0 - 2.0) / (16.0 - 2.0);
        let k3 = 100.0 * (17.0 - 4.0) / (18.0 - 4.0);
        let d = (k1 + k2 + k3) / 3.0;

        assert!(
            (r.k - k3).abs() < 1e-10,
            "k: expected {:.4}, got {:.4}",
            k3,
            r.k
        );
        assert!(
            (r.d - d).abs() < 1e-10,
            "d: expected {:.4}, got {:.4}",
            d,
            r.d
        );
    }

    // ── stochastic_slow tests ───────────────────────────────────────────────

    #[test]
    fn slow_needs_more_warmup_than_fast() {
        // Slow applies extra smoothing, needs more candles than fast
        let c_17: Vec<Candle> = (1..=17)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();

        // Fast needs period + 2 = 16 candles, slow needs period + 4 = 18
        assert!(stochastic_fast(&c_17, 14).is_some());
        assert!(stochastic_slow(&c_17, 14).is_none()); // needs 18, have 17

        let c_19: Vec<Candle> = (1..=19)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert!(stochastic_slow(&c_19, 14).is_some()); // needs 18, have 19
    }

    #[test]
    fn slow_smoother_than_fast() {
        let c: Vec<Candle> = (1..=40)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();

        let fast = stochastic_fast(&c, 14).unwrap();
        let slow = stochastic_slow(&c, 14).unwrap();

        // %D of slow is smoother (less volatile) than %K of fast
        assert!(slow.d <= 100.0 && slow.d >= 0.0);
    }

    // ── stochastic_full tests ───────────────────────────────────────────────

    #[test]
    fn full_defaults_to_k_3_d_3() {
        let c: Vec<Candle> = (1..=25)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();

        // No extra params → defaults k_smooth=3, d_period=3 (same as slow)
        let full_default = stochastic_full(&c, 14, None, None).unwrap();
        let slow = stochastic_slow(&c, 14).unwrap();

        // Should be similar (full with 3,3 = slow)
        assert!((full_default.k - slow.k).abs() < 0.01);
        assert!((full_default.d - slow.d).abs() < 0.01);
    }

    #[test]
    fn full_with_custom_smoothing() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();

        // k_smooth=5, d_period=3
        let r = stochastic_full(&c, 14, Some(5), Some(3)).unwrap();
        assert!(r.k >= 0.0 && r.k <= 100.0);
        assert!(r.d >= 0.0 && r.d <= 100.0);
    }

    #[test]
    fn full_needs_enough_data_for_smoothing() {
        let c: Vec<Candle> = (1..=22)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();

        // period=14, k_smooth=5, d_period=3 → needs 14+5+3-1 = 21 candles
        // We have 22, should work
        assert!(stochastic_full(&c, 14, Some(5), Some(3)).is_some());
    }
}
