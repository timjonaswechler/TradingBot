/// Standard Fibonacci retracement levels between an explicit `low` and `high`.
///
/// This is intentionally a low-level math helper, not the repo's long-term
/// structure-aware Fibonacci model. It does **not**:
/// - detect swings from candles
/// - consume pivot structure
/// - expose anchored state
/// - know trend direction beyond the caller's chosen `low`/`high`
///
/// Long-term direction: keep this helper for manual use, but model the primary
/// strategy-facing Fibonacci flow as an anchored evaluator built from pivot
/// pairs (see ADR `docs/adr/0001-fibonacci-as-anchored-structure-indicator.md`).
/// If we later rename this helper to something clearer like
/// `fibonacci_levels`, the behavior should stay the same.
///
/// Returns a `Vec` of prices at the standard ratios:
/// `[0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0]`
/// ordered from low to high (i.e. 0% = low, 100% = high).
pub fn fibonacci_retracements(low: f64, high: f64) -> Vec<f64> {
    let range = high - low;
    [0.0, 0.236, 0.382, 0.5, 0.618, 0.786, 1.0]
        .iter()
        .map(|&r| low + r * range)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_level_is_low() {
        let levels = fibonacci_retracements(100.0, 200.0);
        assert_eq!(levels[0], 100.0);
    }

    #[test]
    fn last_level_is_high() {
        let levels = fibonacci_retracements(100.0, 200.0);
        assert_eq!(*levels.last().unwrap(), 200.0);
    }

    #[test]
    fn golden_ratio_level() {
        let levels = fibonacci_retracements(0.0, 100.0);
        // 61.8% level
        assert!((levels[4] - 61.8).abs() < 1e-6);
    }

    #[test]
    fn returns_seven_levels() {
        assert_eq!(fibonacci_retracements(50.0, 150.0).len(), 7);
    }

    #[test]
    fn levels_are_ordered() {
        let levels = fibonacci_retracements(100.0, 200.0);
        for w in levels.windows(2) {
            assert!(w[1] >= w[0]);
        }
    }
}
