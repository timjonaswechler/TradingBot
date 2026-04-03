/// Exponential Moving Average.
///
/// Input: closes in chronological order (oldest first).
/// Seeded with the SMA of the first `period` bars, then applies the standard
/// multiplier `k = 2 / (period + 1)` for every subsequent bar.
/// Returns `None` when `closes.len() < period` or `period == 0`.
pub fn ema(closes: &[f64], period: usize) -> Option<f64> {
    ema_at(closes, period, 0)
}

/// Returns the full EMA series (same length as input minus the seed warmup,
/// i.e. `closes.len() - period + 1` elements). Used internally by DEMA/TEMA.
pub fn ema_series(closes: &[f64], period: usize) -> Option<Vec<f64>> {
    if period == 0 || closes.len() < period {
        return None;
    }
    let k = 2.0 / (period as f64 + 1.0);
    let mut val: f64 = closes[..period].iter().sum::<f64>() / period as f64;
    let mut out = Vec::with_capacity(closes.len() - period + 1);
    out.push(val);
    for &price in &closes[period..] {
        val = price * k + val * (1.0 - k);
        out.push(val);
    }
    Some(out)
}

/// EMA with an `offset` so callers can request the value N bars back.
/// `offset = 0` → current bar, `offset = 1` → one bar ago.
pub fn ema_at(closes: &[f64], period: usize, offset: usize) -> Option<f64> {
    if period == 0 || closes.len() < period + offset {
        return None;
    }
    // Work on the slice ending at `closes.len() - offset`
    let end = closes.len() - offset;
    let slice = &closes[..end];

    let k = 2.0 / (period as f64 + 1.0);
    // Seed: SMA of first `period` bars
    let mut val: f64 = slice[..period].iter().sum::<f64>() / period as f64;
    for &price in &slice[period..] {
        val = price * k + val * (1.0 - k);
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        assert_eq!(ema(&[1.0, 2.0], 3), None);
    }

    #[test]
    fn period_zero() {
        assert_eq!(ema(&[1.0, 2.0, 3.0], 0), None);
    }

    #[test]
    fn exact_period_equals_sma_seed() {
        // With only `period` bars, EMA == SMA of those bars
        let c = [1.0, 2.0, 3.0, 4.0, 5.0];
        let result = ema(&c, 5).unwrap();
        let expected = 3.0; // (1+2+3+4+5)/5
        assert!((result - expected).abs() < 1e-10);
    }

    #[test]
    fn one_extra_bar() {
        // SMA(3) of [1,2,3] = 2.0 ; k = 2/(3+1) = 0.5
        // EMA after bar 4 (value=4): 4*0.5 + 2*0.5 = 3.0
        let c = [1.0, 2.0, 3.0, 4.0];
        let result = ema(&c, 3).unwrap();
        assert!((result - 3.0).abs() < 1e-10);
    }

    #[test]
    fn offset_one_equals_previous_ema() {
        let c = [1.0, 2.0, 3.0, 4.0, 5.0];
        // EMA(3) without last bar should equal EMA(3) on c[..4]
        let prev = ema(&c[..4], 3).unwrap();
        let off  = ema_at(&c, 3, 1).unwrap();
        assert!((prev - off).abs() < 1e-10);
    }
}
