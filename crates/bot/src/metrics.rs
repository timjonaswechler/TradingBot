use chrono::{DateTime, Utc};

/// A closed trade record (compatible with paper_trading::engine::Trade)
#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub timestamp: DateTime<Utc>,
    pub pnl_cents: i64,
    pub entry_price_cents: i64,
    pub exit_price_cents: i64,
    pub quantity: i64,
    pub commission_cents: i64,
}

/// Complete performance metrics from a backtest run
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub total_trades: usize,
    pub winning_trades: usize,
    pub losing_trades: usize,

    /// Win rate as percentage 0–100
    pub win_rate_pct: f64,
    /// Average PnL of winning trades as % of position value
    pub avg_win_pct: f64,
    /// Average PnL of losing trades as % of position value (positive number, represents loss magnitude)
    pub avg_loss_pct: f64,
    /// Per-trade expectancy in % = win_rate/100 * avg_win - (1-win_rate/100) * avg_loss
    pub expectancy_pct: f64,

    /// Annualized Sharpe ratio (assumes daily equity curve)
    pub sharpe: f64,
    /// Maximum peak-to-trough drawdown as % of peak equity
    pub max_drawdown_pct: f64,
    /// Total return from starting capital as %
    pub total_return_pct: f64,
    /// Total realized PnL in cents
    pub total_pnl_cents: i64,
}

/// Compute metrics from equity curve and closed trades.
///
/// # Arguments
/// * `equity_curve` - (timestamp, equity_cents) pairs in chronological order (oldest first)
/// * `trades` - All closed trade records
/// * `starting_capital_cents` - Initial portfolio value
pub fn compute(
    equity_curve: &[(DateTime<Utc>, i64)],
    trades: &[TradeRecord],
    starting_capital_cents: i64,
) -> Metrics {
    if trades.is_empty() {
        return Metrics::default();
    }

    let total_trades = trades.len();

    // Single pass: accumulate win/loss counts, sums, and pct sums
    let mut winning_trades = 0usize;
    let mut losing_trades = 0usize;
    let mut win_pct_sum = 0.0f64;
    let mut loss_pct_sum = 0.0f64;

    for t in trades {
        let position_value = t.entry_price_cents * t.quantity;
        let pct = if position_value != 0 {
            t.pnl_cents as f64 / position_value as f64 * 100.0
        } else {
            0.0
        };
        if t.pnl_cents > 0 {
            winning_trades += 1;
            win_pct_sum += pct;
        } else if t.pnl_cents < 0 {
            losing_trades += 1;
            loss_pct_sum += pct.abs();
        }
    }

    let win_rate_pct = winning_trades as f64 / total_trades as f64 * 100.0;
    let avg_win_pct = if winning_trades > 0 { win_pct_sum / winning_trades as f64 } else { 0.0 };
    let avg_loss_pct = if losing_trades > 0 { loss_pct_sum / losing_trades as f64 } else { 0.0 };
    let expectancy_pct =
        (win_rate_pct / 100.0) * avg_win_pct - (1.0 - win_rate_pct / 100.0) * avg_loss_pct;

    let sharpe = compute_sharpe(equity_curve);
    let max_drawdown_pct = compute_max_drawdown(equity_curve, starting_capital_cents);

    let final_equity = equity_curve
        .last()
        .map(|e| e.1)
        .unwrap_or(starting_capital_cents);
    let total_return_pct = if starting_capital_cents != 0 {
        (final_equity - starting_capital_cents) as f64 / starting_capital_cents as f64 * 100.0
    } else {
        0.0
    };

    let total_pnl_cents: i64 = trades.iter().map(|t| t.pnl_cents).sum();

    Metrics {
        total_trades,
        winning_trades,
        losing_trades,
        win_rate_pct,
        avg_win_pct,
        avg_loss_pct,
        expectancy_pct,
        sharpe,
        max_drawdown_pct,
        total_return_pct,
        total_pnl_cents,
    }
}

fn compute_sharpe(equity_curve: &[(DateTime<Utc>, i64)]) -> f64 {
    if equity_curve.len() < 2 {
        return 0.0;
    }

    // Two-pass over windows: first for mean, then for variance — no intermediate Vec.
    let n = (equity_curve.len() - 1) as f64;
    let mean = equity_curve
        .windows(2)
        .map(|w| (w[1].1 - w[0].1) as f64 / w[0].1 as f64)
        .sum::<f64>()
        / n;

    let variance = equity_curve
        .windows(2)
        .map(|w| {
            let r = (w[1].1 - w[0].1) as f64 / w[0].1 as f64;
            (r - mean).powi(2)
        })
        .sum::<f64>()
        / n;

    let std_dev = variance.sqrt();
    if std_dev == 0.0 {
        return 0.0;
    }

    mean / std_dev * 252_f64.sqrt()
}

fn compute_max_drawdown(equity_curve: &[(DateTime<Utc>, i64)], starting_capital_cents: i64) -> f64 {
    let mut peak = starting_capital_cents;
    let mut max_dd = 0.0_f64;

    for &(_, equity) in equity_curve {
        if equity > peak {
            peak = equity;
        }
        if peak > 0 {
            let dd = (peak - equity) as f64 / peak as f64 * 100.0;
            if dd > max_dd {
                max_dd = dd;
            }
        }
    }

    max_dd
}

/// Convert the real `paper_trading::Trade` records (produced by `PaperTradingEngine`) into
/// `TradeRecord`s for metrics computation.
///
/// Entry price is back-calculated from `gain_loss_pct` when available; falls back to exit price
/// times quantity.
pub fn from_paper_trades(trades: &[crate::paper_trading::Trade]) -> Vec<TradeRecord> {
    use crate::paper_trading::TradeSide;

    trades
        .iter()
        .filter(|t| t.side == TradeSide::Sell && t.gain_loss.unwrap_or(0) != 0)
        .map(|t| {
            let position_value_cents = t
                .gain_loss_pct
                .filter(|&p| p != 0.0)
                .map(|pct| (t.gain_loss.unwrap_or(0) as f64 / (pct / 100.0)) as i64)
                .unwrap_or(t.price * t.quantity);
            let entry_price_cents = if t.quantity > 0 {
                position_value_cents / t.quantity
            } else {
                t.price
            };
            TradeRecord {
                timestamp: chrono::DateTime::from_timestamp(t.timestamp, 0).unwrap_or_default(),
                pnl_cents: t.gain_loss.unwrap_or(0),
                entry_price_cents,
                exit_price_cents: t.price,
                quantity: t.quantity,
                commission_cents: t.fee,
            }
        })
        .collect()
}

/// Convert from paper_trading::engine::Trade to TradeRecord.
/// Only includes closing trades (Sell and Cover — those with pnl_cents != 0).
pub fn from_engine_trades(
    trades: &[crate::paper_trading::engine::Trade],
) -> Vec<TradeRecord> {
    use crate::paper_trading::engine::TradeSide;
    use chrono::TimeZone;

    trades
        .iter()
        .filter(|t| {
            matches!(t.side, TradeSide::Sell | TradeSide::Cover) && t.pnl_cents != 0
        })
        .map(|t| TradeRecord {
            timestamp: Utc.timestamp_opt(t.timestamp, 0).single().unwrap_or_default(),
            pnl_cents: t.pnl_cents,
            entry_price_cents: t.entry_price_cents,
            exit_price_cents: t.price_cents,
            quantity: t.quantity,
            commission_cents: t.commission_cents,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_curve(values: &[i64]) -> Vec<(DateTime<Utc>, i64)> {
        values
            .iter()
            .enumerate()
            .map(|(i, &v)| (Utc.timestamp_opt(i as i64 * 86400, 0).unwrap(), v))
            .collect()
    }

    fn make_trade(pnl_cents: i64, entry_price_cents: i64, quantity: i64) -> TradeRecord {
        TradeRecord {
            timestamp: Utc.timestamp_opt(0, 0).unwrap(),
            pnl_cents,
            entry_price_cents,
            exit_price_cents: entry_price_cents,
            quantity,
            commission_cents: 0,
        }
    }

    #[test]
    fn test_perfect_win_rate() {
        let trades = vec![
            make_trade(500, 10_000, 1),
            make_trade(300, 10_000, 1),
            make_trade(200, 10_000, 1),
        ];
        let curve = make_curve(&[100_000, 100_500, 100_800, 101_000]);
        let m = compute(&curve, &trades, 100_000);

        assert_eq!(m.total_trades, 3);
        assert_eq!(m.winning_trades, 3);
        assert_eq!(m.losing_trades, 0);
        assert_eq!(m.win_rate_pct, 100.0);
    }

    #[test]
    fn test_expectancy() {
        // 2 wins at 10%, 1 loss at 5% → win_rate=66.67%, avg_win=10, avg_loss=5
        // expectancy = 0.6667*10 - 0.3333*5 ≈ 5.0
        let trades = vec![
            make_trade(1_000, 10_000, 1),  // +10%
            make_trade(1_000, 10_000, 1),  // +10%
            make_trade(-500, 10_000, 1),   // -5%
        ];
        let curve = make_curve(&[100_000, 100_500, 101_000, 101_500]);
        let m = compute(&curve, &trades, 100_000);

        assert_eq!(m.total_trades, 3);
        assert_eq!(m.winning_trades, 2);
        assert_eq!(m.losing_trades, 1);

        let expected_win_rate = 2.0 / 3.0 * 100.0;
        let expected_expectancy = (expected_win_rate / 100.0) * 10.0
            - (1.0 - expected_win_rate / 100.0) * 5.0;

        assert!((m.win_rate_pct - expected_win_rate).abs() < 0.01);
        assert!((m.avg_win_pct - 10.0).abs() < 0.01);
        assert!((m.avg_loss_pct - 5.0).abs() < 0.01);
        assert!((m.expectancy_pct - expected_expectancy).abs() < 0.01);
    }

    #[test]
    fn test_max_drawdown() {
        // Equity goes up to 125_000 then down to 100_000 → drawdown = 20%
        let curve = make_curve(&[100_000, 110_000, 125_000, 112_500, 100_000]);
        let trades = vec![make_trade(100, 10_000, 1)];
        let m = compute(&curve, &trades, 100_000);

        assert!((m.max_drawdown_pct - 20.0).abs() < 0.01);
    }

    #[test]
    fn test_sharpe_positive() {
        // Steady upward equity curve → positive Sharpe
        let curve = make_curve(&[100_000, 101_000, 102_000, 103_000, 104_000, 105_000]);
        let trades = vec![make_trade(500, 10_000, 1)];
        let m = compute(&curve, &trades, 100_000);

        assert!(m.sharpe > 0.0, "Sharpe should be positive, got {}", m.sharpe);
    }

    #[test]
    fn test_zero_trades() {
        let curve = make_curve(&[100_000, 101_000]);
        let m = compute(&curve, &[], 100_000);

        assert_eq!(m.total_trades, 0);
        assert_eq!(m.winning_trades, 0);
        assert_eq!(m.losing_trades, 0);
        assert_eq!(m.win_rate_pct, 0.0);
        assert_eq!(m.sharpe, 0.0);
        assert_eq!(m.max_drawdown_pct, 0.0);
        assert_eq!(m.total_pnl_cents, 0);
    }

    #[test]
    fn test_total_pnl_and_return() {
        let trades = vec![
            make_trade(5_000, 50_000, 1),
            make_trade(-2_000, 50_000, 1),
        ];
        let curve = make_curve(&[100_000, 103_000]);
        let m = compute(&curve, &trades, 100_000);

        assert_eq!(m.total_pnl_cents, 3_000);
        assert!((m.total_return_pct - 3.0).abs() < 0.01);
    }
}
