use domain::Candle;

/// Williams %R.
///
/// Returns a value in [-100, 0].
/// - Near 0    → overbought
/// - Near -100 → oversold
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `period` candles.
pub fn williams_r(candles: &[Candle], period: usize) -> Option<f64> {
    if period == 0 || candles.len() < period {
        return None;
    }

    let slice = &candles[candles.len() - period..];
    let highest = slice
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let lowest = slice.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    let close = candles.last()?.close;

    if (highest - lowest).abs() < 1e-12 {
        return Some(-50.0); // flat market — middle value
    }

    Some(-100.0 * (highest - close) / (highest - lowest))
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

    #[test]
    fn insufficient_data() {
        assert_eq!(williams_r(&[candle(2.0, 1.0, 1.5)], 2), None);
    }

    #[test]
    fn close_at_high_returns_zero() {
        let c = vec![
            candle(5.0, 1.0, 3.0),
            candle(10.0, 1.0, 10.0), // close = high
        ];
        assert_eq!(williams_r(&c, 2), Some(0.0));
    }

    #[test]
    fn close_at_low_returns_minus_100() {
        let c = vec![
            candle(10.0, 1.0, 5.0),
            candle(10.0, 1.0, 1.0), // close = low
        ];
        assert_eq!(williams_r(&c, 2), Some(-100.0));
    }

    #[test]
    fn value_in_range() {
        let c: Vec<Candle> = (1..=20)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = williams_r(&c, 14).unwrap();
        assert!(r >= -100.0 && r <= 0.0);
    }
}
