use super::ema::ema_series;

/// Double Exponential Moving Average — less lag than a plain EMA.
///
/// Formula: `DEMA = 2 * EMA(n) - EMA(EMA(n))`
///
/// Input: closes in chronological order (oldest first).
/// Needs at least `2 * period - 1` bars.
pub fn dema(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() < 2 * period - 1 {
        return None;
    }
    let ema1 = ema_series(closes, period)?;
    let ema2 = ema_series(&ema1, period)?;
    let last1 = ema1.last()?;
    let last2 = ema2.last()?;
    Some(2.0 * last1 - last2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        // needs 2*3-1 = 5 bars
        assert_eq!(dema(&[1.0, 2.0, 3.0, 4.0], 3), None);
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<f64> = (1..=20).map(|x| x as f64).collect();
        let result = dema(&c, 3);
        assert!(result.is_some());
    }

    #[test]
    fn trending_up_dema_above_sma() {
        // In a perfectly rising series DEMA leads EMA which leads SMA
        let c: Vec<f64> = (1..=30).map(|x| x as f64).collect();
        let d = dema(&c, 5).unwrap();
        // Last value is 30.0 — DEMA should be close to it and > SMA(5) of last 5
        let sma_last: f64 = c[25..].iter().sum::<f64>() / 5.0; // (26+27+28+29+30)/5=28
        assert!(d > sma_last, "DEMA {d} should lead SMA {sma_last}");
    }
}
