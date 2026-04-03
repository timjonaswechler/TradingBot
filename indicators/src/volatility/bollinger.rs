/// Result of Bollinger Bands.
#[derive(Debug, Clone, PartialEq)]
pub struct BbResult {
    pub upper:  f64,
    pub middle: f64, // SMA
    pub lower:  f64,
}

/// Bollinger Bands.
///
/// `middle` = SMA(period), `upper/lower` = middle +/- `std_dev` * standard deviation.
///
/// Input: closes in chronological order (oldest first).
/// Needs at least `period` values.
pub fn bollinger(closes: &[f64], period: usize, std_dev: f64) -> Option<BbResult> {
    if period == 0 || closes.len() < period {
        return None;
    }

    let slice = &closes[closes.len() - period..];
    let mean = slice.iter().sum::<f64>() / period as f64;

    let variance = slice.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / period as f64;
    let sd = variance.sqrt();

    Some(BbResult {
        upper:  mean + std_dev * sd,
        middle: mean,
        lower:  mean - std_dev * sd,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        assert_eq!(bollinger(&[1.0, 2.0], 3, 2.0), None);
    }

    #[test]
    fn flat_series_has_zero_width() {
        let c = [5.0; 20];
        let r = bollinger(&c, 20, 2.0).unwrap();
        assert!((r.upper - r.lower).abs() < 1e-10);
        assert_eq!(r.middle, 5.0);
    }

    #[test]
    fn upper_above_lower() {
        let c: Vec<f64> = (1..=20).map(|x| x as f64).collect();
        let r = bollinger(&c, 20, 2.0).unwrap();
        assert!(r.upper > r.middle);
        assert!(r.middle > r.lower);
    }

    #[test]
    fn std_dev_multiplier_scales_bands() {
        let c: Vec<f64> = (1..=20).map(|x| x as f64).collect();
        let r1 = bollinger(&c, 20, 1.0).unwrap();
        let r2 = bollinger(&c, 20, 2.0).unwrap();
        let width1 = r1.upper - r1.lower;
        let width2 = r2.upper - r2.lower;
        assert!((width2 - 2.0 * width1).abs() < 1e-10);
    }
}
