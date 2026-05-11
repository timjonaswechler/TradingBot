use shared::Candle;

/// Commodity Channel Index.
///
/// Formula: `CCI = (TP - SMA(TP, n)) / (0.015 * mean_deviation)`
/// where TP (Typical Price) = (high + low + close) / 3.
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `period` candles.
pub fn cci(candles: &[Candle], period: usize) -> Option<f64> {
    if period == 0 || candles.len() < period {
        return None;
    }

    let slice = &candles[candles.len() - period..];
    let tp: Vec<f64> = slice
        .iter()
        .map(|c| (c.high + c.low + c.close) / 3.0)
        .collect();

    let sma = tp.iter().sum::<f64>() / period as f64;
    let mean_dev = tp.iter().map(|t| (t - sma).abs()).sum::<f64>() / period as f64;

    if mean_dev < 1e-12 {
        return Some(0.0);
    }

    let current_tp = *tp.last()?;
    Some((current_tp - sma) / (0.015 * mean_dev))
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
        let c: Vec<Candle> = (1..=19)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert_eq!(cci(&c, 20), None);
    }

    #[test]
    fn flat_market_returns_zero() {
        let c: Vec<Candle> = vec![candle(2.0, 0.0, 1.0); 20];
        assert_eq!(cci(&c, 20), Some(0.0));
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert!(cci(&c, 20).is_some());
    }

    #[test]
    fn positive_in_uptrend() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = cci(&c, 20).unwrap();
        assert!(r > 0.0, "CCI {r:.1} should be positive in uptrend");
    }
}
