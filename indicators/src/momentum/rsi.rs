/// Relative Strength Index (Wilder smoothing).
///
/// Input: closes in chronological order (oldest first).
/// Returns a value in [0, 100]. Returns `None` when `closes.len() <= period`.
pub fn rsi(closes: &[f64], period: usize) -> Option<f64> {
    if period == 0 || closes.len() <= period {
        return None;
    }

    // First average gain/loss over initial `period` bars
    let mut avg_gain = 0.0f64;
    let mut avg_loss = 0.0f64;

    for i in 1..=period {
        let change = closes[i] - closes[i - 1];
        if change > 0.0 {
            avg_gain += change;
        } else {
            avg_loss += change.abs();
        }
    }
    avg_gain /= period as f64;
    avg_loss /= period as f64;

    // Wilder smooth for remaining bars
    for i in (period + 1)..closes.len() {
        let change = closes[i] - closes[i - 1];
        let gain = if change > 0.0 { change } else { 0.0 };
        let loss = if change < 0.0 { change.abs() } else { 0.0 };
        avg_gain = (avg_gain * (period as f64 - 1.0) + gain) / period as f64;
        avg_loss = (avg_loss * (period as f64 - 1.0) + loss) / period as f64;
    }

    if avg_loss == 0.0 {
        return Some(100.0);
    }
    let rs = avg_gain / avg_loss;
    Some(100.0 - 100.0 / (1.0 + rs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        assert_eq!(rsi(&[1.0; 14], 14), None); // needs > 14
    }

    #[test]
    fn all_gains_returns_100() {
        let c: Vec<f64> = (1..=20).map(|x| x as f64).collect();
        assert_eq!(rsi(&c, 14), Some(100.0));
    }

    #[test]
    fn all_losses_returns_0() {
        let c: Vec<f64> = (1..=20).rev().map(|x| x as f64).collect();
        assert_eq!(rsi(&c, 14), Some(0.0));
    }

    #[test]
    fn value_in_range() {
        let c = [
            44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.15, 43.61, 44.33, 44.83, 45.10,
            45.15, 43.61, 44.33,
        ];
        let r = rsi(&c, 14).unwrap();
        assert!((0.0..=100.0).contains(&r));
    }
}
