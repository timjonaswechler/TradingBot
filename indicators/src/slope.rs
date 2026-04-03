/// Linear regression slope over the last `period` closes.
///
/// Positive → uptrend, negative → downtrend.
/// Magnitude reflects the steepness of the trend.
///
/// Input: closes in chronological order (oldest first).
/// Needs at least `period` values.
pub fn slope(closes: &[f64], period: usize) -> Option<f64> {
    if period < 2 || closes.len() < period {
        return None;
    }

    let slice = &closes[closes.len() - period..];
    let n = period as f64;

    // x values: 0, 1, 2, ..., period-1
    let x_mean = (n - 1.0) / 2.0;
    let y_mean = slice.iter().sum::<f64>() / n;

    let (numerator, denominator) = slice.iter().enumerate().fold((0.0f64, 0.0f64), |(num, den), (i, &y)| {
        let x = i as f64 - x_mean;
        (num + x * (y - y_mean), den + x * x)
    });

    if denominator.abs() < 1e-12 {
        return Some(0.0);
    }

    Some(numerator / denominator)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        assert_eq!(slope(&[1.0, 2.0], 3), None);
    }

    #[test]
    fn period_one_returns_none() {
        assert_eq!(slope(&[1.0, 2.0, 3.0], 1), None);
    }

    #[test]
    fn perfect_uptrend_has_positive_slope() {
        let c = [1.0, 2.0, 3.0, 4.0, 5.0];
        let s = slope(&c, 5).unwrap();
        assert!(s > 0.0, "slope {s} should be positive");
    }

    #[test]
    fn perfect_downtrend_has_negative_slope() {
        let c = [5.0, 4.0, 3.0, 2.0, 1.0];
        let s = slope(&c, 5).unwrap();
        assert!(s < 0.0, "slope {s} should be negative");
    }

    #[test]
    fn flat_series_has_zero_slope() {
        let c = [3.0; 5];
        assert_eq!(slope(&c, 5), Some(0.0));
    }

    #[test]
    fn slope_equals_one_for_unit_steps() {
        // y = x → slope must be exactly 1.0
        let c = [0.0, 1.0, 2.0, 3.0, 4.0];
        let s = slope(&c, 5).unwrap();
        assert!((s - 1.0).abs() < 1e-10);
    }
}
