use crate::metrics::Metrics;

/// Weights controlling the composite fitness score.
/// All fields have sensible defaults; override via config if needed.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FitnessWeights {
    /// Sharpe ratio contribution (default 1.0)
    pub sharpe: f64,
    /// Win-rate contribution (default 0.5)
    pub win_rate: f64,
    /// Expectancy contribution (default 1.0)
    pub expectancy: f64,
    /// Avg-win contribution (default 0.3)
    pub avg_win: f64,
    /// Avg-loss penalty coefficient (default 0.3)
    pub avg_loss: f64,
    /// Max-drawdown penalty coefficient (default 0.5)
    pub drawdown: f64,
    /// Minimum trades required; below this the genome scores -1000.0 (default 5)
    pub min_trades: usize,
}

impl Default for FitnessWeights {
    fn default() -> Self {
        Self {
            sharpe:     1.0,
            win_rate:   0.5,
            expectancy: 1.0,
            avg_win:    0.3,
            avg_loss:   0.3,
            drawdown:   0.5,
            min_trades: 5,
        }
    }
}

/// Composite fitness score computed from [`Metrics`].
///
/// Returns `-1000.0` if `metrics.total_trades < weights.min_trades`.
pub fn score(metrics: &Metrics, weights: &FitnessWeights) -> f64 {
    if metrics.total_trades < weights.min_trades {
        return -1000.0;
    }

    let win_rate_score   = metrics.win_rate_pct * weights.win_rate;
    let expectancy_score = metrics.expectancy_pct.clamp(-15.0, 15.0) * weights.expectancy;
    let sharpe_score     = metrics.sharpe.clamp(-4.0, 4.0) * weights.sharpe;
    let avg_win_score    = metrics.avg_win_pct.min(20.0) * weights.avg_win;
    let avg_loss_penalty = metrics.avg_loss_pct.min(20.0) * weights.avg_loss;
    let drawdown_penalty = metrics.max_drawdown_pct.abs() * weights.drawdown;

    win_rate_score + expectancy_score + sharpe_score + avg_win_score
        - avg_loss_penalty - drawdown_penalty
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::Metrics;

    fn metrics_with_trades(n: usize) -> Metrics {
        Metrics {
            total_trades:     n,
            winning_trades:   n / 2,
            losing_trades:    n - n / 2,
            win_rate_pct:     50.0,
            avg_win_pct:      5.0,
            avg_loss_pct:     3.0,
            expectancy_pct:   1.0,
            sharpe:           1.2,
            max_drawdown_pct: -8.0,
            total_return_pct: 12.0,
            ..Default::default()
        }
    }

    #[test]
    fn score_above_min_trades_is_finite() {
        let m = metrics_with_trades(10);
        let w = FitnessWeights::default();
        let s = score(&m, &w);
        assert!(s.is_finite(), "expected finite score, got {s}");
        assert!(s > -1000.0);
    }

    #[test]
    fn score_below_min_trades_returns_sentinel() {
        let mut m = metrics_with_trades(4);
        m.total_trades = 4;
        let w = FitnessWeights::default(); // min_trades = 5
        assert_eq!(score(&m, &w), -1000.0);
    }

    #[test]
    fn score_exactly_at_min_trades_is_finite() {
        let m = metrics_with_trades(5);
        let w = FitnessWeights::default();
        let s = score(&m, &w);
        assert!(s.is_finite());
        assert!(s > -1000.0);
    }
}
