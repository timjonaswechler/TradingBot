use crate::config::FitnessWeights;
use crate::metrics::Metrics;

/// Berechnet einen Fitness-Score der primär den Erwartungswert pro Trade optimiert.
///
/// Kernidee: Ein perfekter Trade hat avg_loss = 0 und win_rate = 100%.
/// Der Erwartungswert drückt das direkt aus:
///
///   expectancy = (win_rate/100) × avg_win%  −  (1 − win_rate/100) × avg_loss%
///
/// Ideal: expectancy = avg_win_pct  (alle Trades gewinnen, kein Verlust)
///
/// Zusätzlich: Sharpe als Stabilitätskriterium (verhindert dass der Optimizer
/// einen einzigen Glückstreffer auf einem Zeitfenster überfitten).
/// Maximaler realistischer Erwartungswert pro Trade in %.
/// Trades die darüber hinausgehen (z.B. Lucky-Window auf Penny Stocks)
/// zählen nicht mehr — verhindert Overfitting auf Ausreißer-Fenster.
const MAX_EXPECTANCY_PCT: f64 = 15.0; // ein Trade der im Schnitt 15% bringt ist bereits exzellent
const MAX_AVG_WIN_PCT:    f64 = 20.0;
const MAX_AVG_LOSS_PCT:   f64 = 20.0;
const MAX_SHARPE:         f64 = 4.0;  // Sharpe > 4 ist auf echten Daten extrem unwahrscheinlich

pub fn score(metrics: &Metrics, cfg: &FitnessWeights) -> f64 {
    if metrics.total_trades < cfg.min_trades {
        return f64::NEG_INFINITY;
    }

    // Win Rate: 0–100, direkt skaliert mit Gewicht → Score 0–100 bei Gewicht 1.0
    let win_rate_score   = metrics.win_rate_pct * cfg.win_rate;

    // Optionale Zusatzterme (alle 0.0 wenn nicht erwünscht)
    let expectancy       = metrics.expectancy_pct.clamp(-MAX_EXPECTANCY_PCT, MAX_EXPECTANCY_PCT);
    let avg_win          = metrics.avg_win_pct.min(MAX_AVG_WIN_PCT);
    let avg_loss         = metrics.avg_loss_pct.min(MAX_AVG_LOSS_PCT);
    let sharpe           = metrics.sharpe.clamp(-MAX_SHARPE, MAX_SHARPE);
    let drawdown         = metrics.max_drawdown_pct.abs();

    let expectancy_score = expectancy * cfg.expectancy;
    let sharpe_score     = sharpe     * cfg.sharpe;
    let avg_win_score    = avg_win    * cfg.avg_win;
    let avg_loss_penalty = avg_loss   * cfg.avg_loss;
    let drawdown_penalty = drawdown   * cfg.drawdown;

    win_rate_score + expectancy_score + sharpe_score + avg_win_score
        - avg_loss_penalty - drawdown_penalty
}
