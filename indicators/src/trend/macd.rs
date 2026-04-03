use super::ema::{ema_at, ema_series};

/// Result of a MACD calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct MacdResult {
    /// MACD line = EMA(fast) - EMA(slow)
    pub line: f64,
    /// Signal line = EMA(signal_period) of MACD line
    pub signal: f64,
    /// Histogram = line - signal
    pub histogram: f64,
}

/// MACD indicator.
///
/// Input: closes in chronological order (oldest first).
/// Typical parameters: fast=12, slow=26, signal=9.
/// Needs at least `slow + signal - 1` bars.
pub fn macd(
    closes: &[f64],
    fast: usize,
    slow: usize,
    signal_period: usize,
) -> Option<MacdResult> {
    if fast == 0 || slow == 0 || signal_period == 0 || fast >= slow {
        return None;
    }
    let needed = slow + signal_period - 1;
    if closes.len() < needed {
        return None;
    }

    // Build full EMA(fast) and EMA(slow) series
    let ema_fast = ema_series(closes, fast)?;
    let ema_slow = ema_series(closes, slow)?;

    // MACD line series: align by taking the tail of ema_fast to match ema_slow length
    let diff = ema_fast.len() - ema_slow.len();
    let macd_line: Vec<f64> = ema_slow
        .iter()
        .enumerate()
        .map(|(i, s)| ema_fast[i + diff] - s)
        .collect();

    // Signal line = EMA(signal_period) of the MACD line series
    let signal_line = ema_at(&macd_line, signal_period, 0)?;
    let last_macd = *macd_line.last()?;

    Some(MacdResult {
        line:      last_macd,
        signal:    signal_line,
        histogram: last_macd - signal_line,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insufficient_data() {
        // needs 26+9-1 = 34 bars
        let c = vec![1.0f64; 33];
        assert_eq!(macd(&c, 12, 26, 9), None);
    }

    #[test]
    fn fast_must_be_less_than_slow() {
        let c = vec![1.0f64; 40];
        assert_eq!(macd(&c, 26, 12, 9), None);
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<f64> = (1..=50).map(|x| x as f64).collect();
        let r = macd(&c, 12, 26, 9);
        assert!(r.is_some());
    }

    #[test]
    fn histogram_is_line_minus_signal() {
        let c: Vec<f64> = (1..=50).map(|x| x as f64).collect();
        let r = macd(&c, 12, 26, 9).unwrap();
        assert!((r.histogram - (r.line - r.signal)).abs() < 1e-10);
    }

    #[test]
    fn uptrend_produces_positive_macd_line() {
        // In a strongly rising series, fast EMA > slow EMA
        let c: Vec<f64> = (1..=60).map(|x| x as f64).collect();
        let r = macd(&c, 12, 26, 9).unwrap();
        assert!(r.line > 0.0, "MACD line should be positive in uptrend");
    }
}
