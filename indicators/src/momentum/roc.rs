/// Rate of Change — percentage price change over `period` bars.
///
/// Formula: `ROC = (close - close[period]) / close[period] * 100`
///
/// Input: closes in chronological order (oldest first).
/// Needs at least `period + 1` values.
pub fn roc(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() <= period {
        return None;
    }
    let current = *closes.last()?;
    let previous = closes[closes.len() - 1 - period];
    if previous.abs() < 1e-12 {
        return None; // division by zero guard
    }
    Some((current - previous) / previous * 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        assert_eq!(roc(&[1.0, 2.0], 2), None);
    }

    #[test]
    fn doubled_price_gives_100_percent() {
        let c = [10.0, 10.0, 20.0];
        assert_eq!(roc(&c, 2), Some(100.0));
    }

    #[test]
    fn halved_price_gives_minus_50_percent() {
        let c = [20.0, 15.0, 10.0];
        assert_eq!(roc(&c, 2), Some(-50.0));
    }

    #[test]
    fn no_change_gives_zero() {
        let c = [5.0, 6.0, 5.0];
        assert_eq!(roc(&c, 2), Some(0.0));
    }
}
