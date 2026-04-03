/// Simple Moving Average of the last `period` values.
///
/// Input: closes in chronological order (oldest first).
/// Returns `None` when `closes.len() < period` or `period == 0`.
pub fn sma(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() < period {
        return None;
    }
    let slice = &closes[closes.len() - period..];
    Some(slice.iter().sum::<f64>() / period as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic() {
        let c = [1.0, 2.0, 3.0, 4.0, 5.0];
        // SMA(3) of last 3 = (3+4+5)/3 = 4.0
        assert_eq!(sma(&c, 3), Some(4.0));
    }

    #[test]
    fn full_window() {
        let c = [2.0, 4.0, 6.0];
        assert_eq!(sma(&c, 3), Some(4.0));
    }

    #[test]
    fn insufficient_data() {
        assert_eq!(sma(&[1.0, 2.0], 3), None);
    }

    #[test]
    fn period_zero() {
        assert_eq!(sma(&[1.0, 2.0], 0), None);
    }
}
