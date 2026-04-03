use super::ema::ema_series;

/// Triple Exponential Moving Average — even less lag than DEMA.
///
/// Formula: `TEMA = 3*EMA1 - 3*EMA2 + EMA3`
///
/// Input: closes in chronological order (oldest first).
/// Needs at least `3 * period - 2` bars.
pub fn tema(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() < 3 * period - 2 {
        return None;
    }
    let ema1 = ema_series(closes, period)?;
    let ema2 = ema_series(&ema1, period)?;
    let ema3 = ema_series(&ema2, period)?;
    let e1 = ema1.last()?;
    let e2 = ema2.last()?;
    let e3 = ema3.last()?;
    Some(3.0 * e1 - 3.0 * e2 + e3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        // needs 3*3-2 = 7 bars
        assert_eq!(tema(&[1.0; 6], 3), None);
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<f64> = (1..=30).map(|x| x as f64).collect();
        assert!(tema(&c, 3).is_some());
    }

    #[test]
    fn tema_leads_dema_in_uptrend() {
        use super::super::dema::dema;
        let c: Vec<f64> = (1..=40).map(|x| x as f64).collect();
        let t = tema(&c, 5).unwrap();
        let d = dema(&c, 5).unwrap();
        assert!(t > d, "TEMA {t} should lead DEMA {d} in an uptrend");
    }
}
