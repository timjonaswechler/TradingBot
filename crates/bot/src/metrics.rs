use chrono::{DateTime, Utc};

use crate::paper_trading::{Trade, TradeSide};
use crate::paper_trading::engine::Trade as EngineTrade;

/// Alle Performance-Metriken eines Backtests.
#[derive(Debug, Clone, Default)]
pub struct Metrics {
    // Rendite
    pub total_return_pct: f64, // z.B. 12.4  (= +12,4 %)
    pub cagr_pct:         f64, // annualisiert
    pub start_value:      i64, // in Cent
    pub end_value:        i64, // in Cent

    // Risiko
    pub sharpe:           f64, // > 1,0 gut | > 2,0 sehr gut
    pub max_drawdown_pct: f64, // negativ, z.B. -8.2 (= -8,2 %)

    // Trades — absolut
    pub total_trades:   usize,
    pub winning_trades: usize,
    pub losing_trades:  usize,
    pub win_rate_pct:   f64,  // 0–100
    pub profit_factor:  f64,  // Bruttogewinn / |Bruttoverlust|

    // Trades — prozentual (asset-unabhängig, für Optimizer)
    pub avg_win_pct:    f64,  // Ø Gewinn pro Gewinn-Trade in %
    pub avg_loss_pct:   f64,  // Ø Verlust pro Verlust-Trade in % (positiver Wert)
    pub best_trade_pct: f64,  // bester einzelner Trade in %
    pub worst_trade_pct:f64,  // schlechtester einzelner Trade in %
    pub expectancy_pct: f64,  // Erwartungswert pro Trade in %
                              // = win_rate * avg_win - loss_rate * avg_loss

    // Kosten
    pub total_fees: i64, // Cent
    pub total_tax:  i64, // Cent
}

impl Metrics {
    /// Berechnet alle Metriken aus der Equity-Kurve und den Trades.
    ///
    /// `equity_curve` – Portfolio-Gesamtwert (in Cent) pro Candle-Schritt,
    ///                  chronologisch aufsteigend.
    /// `days`         – Anzahl Kalendertage des Backtestzeitraums.
    pub fn compute(equity_curve: &[i64], trades: &[Trade], days: u64) -> Self {
        let start_value = equity_curve.first().copied().unwrap_or(0);
        let end_value   = equity_curve.last().copied().unwrap_or(0);

        // ── Total Return ─────────────────────────────────────────────────────
        let total_return_pct = if start_value > 0 {
            (end_value as f64 - start_value as f64) / start_value as f64 * 100.0
        } else {
            0.0
        };

        // ── CAGR ─────────────────────────────────────────────────────────────
        // (Endwert / Startwert) ^ (365 / Tage) − 1
        let cagr_pct = if start_value > 0 && days > 0 {
            let ratio = end_value as f64 / start_value as f64;
            (ratio.powf(365.0 / days as f64) - 1.0) * 100.0
        } else {
            0.0
        };

        // ── Tägliche Renditen für Sharpe & Volatilität ───────────────────────
        let daily_returns: Vec<f64> = equity_curve
            .windows(2)
            .map(|w| {
                if w[0] == 0 {
                    0.0
                } else {
                    (w[1] as f64 - w[0] as f64) / w[0] as f64
                }
            })
            .collect();

        let sharpe = sharpe_ratio(&daily_returns);

        // ── Max Drawdown ──────────────────────────────────────────────────────
        let max_drawdown_pct = max_drawdown(equity_curve);

        // ── Trade-Statistiken (nur SELL-Trades haben G/L) ────────────────────
        let sell_trades: Vec<&Trade> = trades
            .iter()
            .filter(|t| t.side == TradeSide::Sell)
            .collect();

        let total_trades   = sell_trades.len();
        let winning_trades = sell_trades.iter().filter(|t| t.gain_loss.unwrap_or(0) > 0).count();
        let losing_trades  = sell_trades.iter().filter(|t| t.gain_loss.unwrap_or(0) < 0).count();

        let win_rate_pct = if total_trades > 0 {
            winning_trades as f64 / total_trades as f64 * 100.0
        } else {
            0.0
        };

        let gross_profit: i64 = sell_trades.iter().filter_map(|t| t.gain_loss).filter(|&g| g > 0).sum();
        let gross_loss:   i64 = sell_trades.iter().filter_map(|t| t.gain_loss).filter(|&g| g < 0).map(|g| g.abs()).sum();

        let profit_factor = if gross_loss > 0 {
            gross_profit as f64 / gross_loss as f64
        } else if gross_profit > 0 {
            f64::INFINITY
        } else {
            0.0
        };

        // ── Prozentuale Trade-Metriken (asset-unabhängig) ─────────────────────
        let win_pcts: Vec<f64> = sell_trades.iter()
            .filter_map(|t| t.gain_loss_pct)
            .filter(|&p| p > 0.0)
            .collect();

        let loss_pcts: Vec<f64> = sell_trades.iter()
            .filter_map(|t| t.gain_loss_pct)
            .filter(|&p| p < 0.0)
            .map(|p| p.abs())
            .collect();

        let all_pcts: Vec<f64> = sell_trades.iter()
            .filter_map(|t| t.gain_loss_pct)
            .collect();

        let avg_win_pct = if win_pcts.is_empty() { 0.0 } else {
            win_pcts.iter().sum::<f64>() / win_pcts.len() as f64
        };
        let avg_loss_pct = if loss_pcts.is_empty() { 0.0 } else {
            loss_pcts.iter().sum::<f64>() / loss_pcts.len() as f64
        };
        let best_trade_pct  = all_pcts.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let worst_trade_pct = all_pcts.iter().cloned().fold(f64::INFINITY,     f64::min);

        let loss_rate = 1.0 - win_rate_pct / 100.0;
        let expectancy_pct = win_rate_pct / 100.0 * avg_win_pct - loss_rate * avg_loss_pct;

        // ── Kosten ────────────────────────────────────────────────────────────
        let total_fees: i64 = trades.iter().map(|t| t.fee).sum();
        let total_tax:  i64 = trades.iter().filter_map(|t| t.tax).sum();

        Self {
            total_return_pct,
            cagr_pct,
            start_value,
            end_value,
            sharpe,
            max_drawdown_pct,
            total_trades,
            winning_trades,
            losing_trades,
            win_rate_pct,
            profit_factor,
            avg_win_pct,
            avg_loss_pct,
            best_trade_pct:  if best_trade_pct  == f64::NEG_INFINITY { 0.0 } else { best_trade_pct },
            worst_trade_pct: if worst_trade_pct == f64::INFINITY     { 0.0 } else { worst_trade_pct },
            expectancy_pct,
            total_fees,
            total_tax,
        }
    }

    /// Druckt eine übersichtliche Zusammenfassung in die Konsole.
    pub fn print(&self, asset: &str, strategy_name: &str, days: u64) {
        let gl_sign = if self.end_value >= self.start_value { "+" } else { "" };

        println!("\n═══ Backtest: {asset} – {strategy_name} ══════════════════════════════");
        println!("  Zeitraum:         {} Tage", days);
        println!("  Startkapital:     {:.2} €", self.start_value as f64 / 100.0);
        println!("  Endkapital:       {:.2} €", self.end_value   as f64 / 100.0);
        println!();
        println!("── Rendite ─────────────────────────────────────────────────────────────");
        println!(
            "  Total Return:     {}{:.2} %",
            gl_sign, self.total_return_pct
        );
        println!("  CAGR:             {:+.2} % p.a.", self.cagr_pct);
        println!();
        println!("── Risiko ──────────────────────────────────────────────────────────────");
        println!("  Sharpe Ratio:     {:.2}", self.sharpe);
        println!("  Max Drawdown:     {:.2} %", self.max_drawdown_pct);
        println!();
        println!("── Trades ──────────────────────────────────────────────────────────────");
        println!("  Gesamt:           {}", self.total_trades);
        println!("  Gewinner:         {} ({:.1} %)", self.winning_trades, self.win_rate_pct);
        println!("  Verlierer:        {}", self.losing_trades);
        println!("  Profit Factor:    {:.2}", self.profit_factor);
        println!();
        println!("── Ø Trade-Performance (% des eingesetzten Kapitals) ───────────────────");
        println!("  Ø Gewinn/Trade:   {:+.2} %", self.avg_win_pct);
        println!("  Ø Verlust/Trade:  -{:.2} %", self.avg_loss_pct);
        println!("  Erwartungswert:   {:+.2} % pro Trade", self.expectancy_pct);
        println!("  Bester Trade:     {:+.2} %", self.best_trade_pct);
        println!("  Schlechtster:     {:+.2} %", self.worst_trade_pct);
        println!();
        println!("── Kosten & Steuern ────────────────────────────────────────────────────");
        println!("  Gebühren:         {:.2} €", self.total_fees as f64 / 100.0);
        println!("  Steuern:          {:.2} €", self.total_tax  as f64 / 100.0);
        println!("════════════════════════════════════════════════════════════════════════");
    }
}

// ── Hilfsfunktionen ──────────────────────────────────────────────────────────

/// Annualisierte Sharpe Ratio (252 Handelstage, risikofreier Zinssatz = 0).
fn sharpe_ratio(daily_returns: &[f64]) -> f64 {
    if daily_returns.len() < 2 {
        return 0.0;
    }
    let n    = daily_returns.len() as f64;
    let mean = daily_returns.iter().sum::<f64>() / n;
    let var  = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n;
    let std  = var.sqrt();

    if std == 0.0 {
        0.0
    } else {
        mean / std * 252.0_f64.sqrt()
    }
}

// ── Optimizer-facing API ──────────────────────────────────────────────────────

/// Trade record produced by the optimizer's paper-trading engine.
#[derive(Debug, Clone)]
pub struct TradeRecord {
    pub timestamp:         DateTime<Utc>,
    pub pnl_cents:         i64,
    pub entry_price_cents: i64,
    pub exit_price_cents:  i64,
    pub quantity:          i64,
    pub commission_cents:  i64,
}

/// Compute metrics from a `(timestamp, equity_cents)` curve.
/// STUB — returns `Metrics::default()`. Replace when merging.
pub fn compute(
    _equity_curve: &[(DateTime<Utc>, i64)],
    _trades:       &[TradeRecord],
    _capital:      i64,
) -> Metrics {
    Metrics::default()
}

/// Convert engine trades to `TradeRecord`s.
/// STUB — returns empty vec. Replace when merging.
pub fn from_engine_trades(_trades: &[EngineTrade]) -> Vec<TradeRecord> {
    vec![]
}

/// Größter prozentualer Verlust von Hochpunkt zu Tiefpunkt (negativ).
fn max_drawdown(equity: &[i64]) -> f64 {
    let mut peak   = f64::MIN;
    let mut max_dd = 0.0f64;

    for &val in equity {
        let v = val as f64;
        if v > peak {
            peak = v;
        }
        if peak > 0.0 {
            let dd = (v - peak) / peak * 100.0;
            if dd < max_dd {
                max_dd = dd;
            }
        }
    }
    max_dd
}
