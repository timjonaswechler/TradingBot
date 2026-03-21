/// Computes EMA over a f64 series. Returns Vec<f64> same length.
/// For bars where EMA is not yet computable (i < period-1), returns series[i] (SMA seed).
/// Input: data in chronological order (oldest first).
pub fn compute(data: &[f64], period: usize) -> Vec<f64> {
    if data.is_empty() || period == 0 {
        return vec![];
    }
    if period > data.len() {
        return data.to_vec();
    }

    let n = data.len();
    let alpha = 2.0 / (period as f64 + 1.0);
    let mut result = vec![0.0f64; n];

    // Echo raw values before the SMA seed window is full
    for (i, &v) in data[..period - 1].iter().enumerate() {
        result[i] = v;
    }

    // Seed with SMA of first `period` values
    result[period - 1] = data[..period].iter().sum::<f64>() / period as f64;

    // Apply EMA formula for remaining bars
    for i in period..n {
        result[i] = alpha * data[i] + (1.0 - alpha) * result[i - 1];
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ema_length() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let ema = compute(&data, 3);
        assert_eq!(ema.len(), data.len());
    }

    #[test]
    fn test_ema_convergence() {
        let data = vec![5.0; 20];
        let ema = compute(&data, 5);
        for &v in &ema[4..] {
            assert!((v - 5.0).abs() < 1e-10, "EMA should converge to constant value");
        }
    }

    #[test]
    fn test_ema_seed_values() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let ema = compute(&data, 3);
        assert_eq!(ema[0], data[0]);
        assert_eq!(ema[1], data[1]);
        let expected_seed = (1.0 + 2.0 + 3.0) / 3.0;
        assert!((ema[2] - expected_seed).abs() < 1e-10);
    }

    #[test]
    fn test_ema_empty() {
        let ema = compute(&[], 5);
        assert!(ema.is_empty());
    }

    #[test]
    fn test_ema_period_larger_than_data() {
        let data = vec![1.0, 2.0];
        let ema = compute(&data, 5);
        assert_eq!(ema.len(), data.len());
        assert_eq!(ema[0], data[0]);
        assert_eq!(ema[1], data[1]);
    }

    #[test]
    fn test_ema_period_one() {
        let data = vec![1.0, 2.0, 3.0, 4.0];
        let ema = compute(&data, 1);
        // alpha = 1.0 when period = 1, so EMA collapses to the data itself
        for (e, d) in ema.iter().zip(data.iter()) {
            assert!((e - d).abs() < 1e-10);
        }
    }
}
