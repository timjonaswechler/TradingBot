use shared::Candle;

/// Result of the Stochastic Oscillator.
#[derive(Debug, Clone, PartialEq)]
pub struct StochasticResult {
    /// %K — raw stochastic value (0-100)
    pub k: f64,
    /// %D — SMA(3) of %K
    pub d: f64,
}

/// Stochastic Oscillator (%K and %D).
///
/// This implements the common fast stochastic form:
/// - `%K` = current close's position inside the lookback high/low range
/// - `%D` = fixed 3-bar SMA of `%K`
///
/// TODO(#23)[https://github.com/timjonaswechler/TradingBot/issues/23]: Decide whether the strategy-facing API should stay fast-only or
/// expose a slow/full stochastic variant for platform parity.
///
/// Input: candles in chronological order (oldest first).
/// `period` is the %K lookback (typically 14).
/// `%D` is a 3-bar SMA of `%K` (fixed).
/// Needs at least `period + 2` candles.
pub fn stochastic(candles: &[Candle], period: usize) -> Option<StochasticResult> {
    let d_period = 3;
    if period == 0 || candles.len() < period + d_period - 1 {
        return None;
    }

    // Build %K for the last `d_period` bars
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
            50.0 // flat market
        } else {
            100.0 * (current_close - lowest) / (highest - lowest)
        };
        k_vals.push(k);
    }

    let k = *k_vals.last()?;
    let d = k_vals.iter().sum::<f64>() / d_period as f64;

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
            timeframe: "1d".into(),
        }
    }

    #[test]
    fn insufficient_data() {
        let c: Vec<Candle> = (1..=15)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert_eq!(stochastic(&c, 14), None); // needs 16
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=20)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert!(stochastic(&c, 14).is_some());
    }

    #[test]
    fn values_in_range() {
        let c: Vec<Candle> = (1..=20)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = stochastic(&c, 14).unwrap();
        assert!(r.k >= 0.0 && r.k <= 100.0);
        assert!(r.d >= 0.0 && r.d <= 100.0);
    }

    #[test]
    fn overbought_in_uptrend() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = stochastic(&c, 14).unwrap();
        assert!(r.k > 80.0, "%K {:.1} should be overbought in uptrend", r.k);
    }

    #[test]
    fn computes_fast_stochastic_k_and_d_from_last_three_windows() {
        let c = vec![
            candle(10.0, 0.0, 5.0),
            candle(12.0, 2.0, 8.0),
            candle(14.0, 4.0, 11.0),
            candle(16.0, 6.0, 14.0),
            candle(18.0, 8.0, 17.0),
        ];

        let r = stochastic(&c, 3).unwrap();

        let expected_k1 = 100.0 * (11.0 - 0.0) / (14.0 - 0.0);
        let expected_k2 = 100.0 * (14.0 - 2.0) / (16.0 - 2.0);
        let expected_k3 = 100.0 * (17.0 - 4.0) / (18.0 - 4.0);
        let expected_d = (expected_k1 + expected_k2 + expected_k3) / 3.0;

        assert!((r.k - expected_k3).abs() < 1e-10);
        assert!((r.d - expected_d).abs() < 1e-10);
    }
}
