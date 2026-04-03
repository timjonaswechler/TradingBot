use shared::Candle;

/// Average True Range (Wilder smoothing).
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `period + 1` candles (one extra for the first TR calculation).
pub fn atr(candles: &[Candle], period: usize) -> Option<f64> {
    if period == 0 || candles.len() < period + 1 {
        return None;
    }

    // True Range series
    let tr: Vec<f64> = candles.windows(2).map(|w| {
        let prev = &w[0];
        let cur  = &w[1];
        (cur.high - cur.low)
            .max((cur.high - prev.close).abs())
            .max((cur.low  - prev.close).abs())
    }).collect();

    // Seed: simple average of first `period` TRs
    let mut atr_val: f64 = tr[..period].iter().sum::<f64>() / period as f64;

    // Wilder smooth
    for &t in &tr[period..] {
        atr_val = (atr_val * (period as f64 - 1.0) + t) / period as f64;
    }

    Some(atr_val)
}

/// Returns the full ATR series (length = `candles.len() - period`).
/// Used internally by Keltner Channels.
pub fn atr_series(candles: &[Candle], period: usize) -> Option<Vec<f64>> {
    if period == 0 || candles.len() < period + 1 {
        return None;
    }
    let tr: Vec<f64> = candles.windows(2).map(|w| {
        let prev = &w[0];
        let cur  = &w[1];
        (cur.high - cur.low)
            .max((cur.high - prev.close).abs())
            .max((cur.low  - prev.close).abs())
    }).collect();

    let mut val: f64 = tr[..period].iter().sum::<f64>() / period as f64;
    let mut out = vec![val];
    for &t in &tr[period..] {
        val = (val * (period as f64 - 1.0) + t) / period as f64;
        out.push(val);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64) -> Candle {
        Candle { timestamp: 0, symbol: "T".into(), open: l, high: h, low: l, close: c, volume: 1.0, timeframe: "1d".into() }
    }

    #[test]
    fn insufficient_data() {
        let c: Vec<Candle> = (1..=14).map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64)).collect();
        assert_eq!(atr(&c, 14), None); // needs 15
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=30).map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64)).collect();
        assert!(atr(&c, 14).is_some());
    }

    #[test]
    fn atr_is_positive() {
        let c: Vec<Candle> = (1..=30).map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64)).collect();
        let r = atr(&c, 14).unwrap();
        assert!(r > 0.0);
    }

    #[test]
    fn constant_range_converges_to_range() {
        // Each candle has high-low = 2.0, no gaps → TR = 2.0 always
        let c: Vec<Candle> = (1..=30).map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64)).collect();
        let r = atr(&c, 14).unwrap();
        assert!((r - 2.0).abs() < 1e-6, "ATR {r:.6} should converge to range 2.0");
    }
}
