//! Terminal backtester.
//!
//! Loads candles from SpacetimeDB (local cache), runs a `.rhai` strategy
//! through the Trading Runtime, prints a summary.
//!
//! Usage:
//! ```
//! backtester \
//!     --strategy strategies/sma_cross.rhai \
//!     --symbol   AAPL
//! ```
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use tracing::info;

use backtester::{
    plan::{execute_plan, render_markdown, DatasetCandleRequest},
    run_runtime_backtest_with_loader, RuntimeBacktestConfig,
};
use db_layer::{get_candles, get_candles_before, get_candles_in_range, SpacetimeClient};

#[derive(Parser, Debug)]
#[command(
    name = "backtester",
    about = "Runtime-backed backtester — no DB writes"
)]
struct Cli {
    /// Path to the `.rhai` strategy file.
    #[arg(short, long)]
    strategy: String,

    /// Path to the `.rhai` backtest plan.
    #[arg(short, long)]
    plan: Option<String>,

    /// Symbol to backtest (must already be seeded in SpacetimeDB).
    #[arg(short = 'S', long)]
    symbol: Option<String>,

    /// Starting paper balance.
    #[arg(short, long, default_value_t = 10_000.0)]
    balance: f64,

    /// SpacetimeDB URL.
    #[arg(long, default_value = "http://127.0.0.1:3000")]
    db_url: String,

    /// SpacetimeDB module name.
    #[arg(long, default_value = "trading-bot")]
    db_module: String,

    /// Maximum candles to load (u32::MAX by default — "everything").
    #[arg(long, default_value_t = u32::MAX)]
    max_candles: u32,

    /// Print every trade, not just the summary.
    #[arg(long)]
    verbose: bool,
}

#[derive(Debug)]
enum RunMode {
    Direct { symbol: String },
    Plan { plan_path: String },
}

impl Cli {
    fn run_mode(&self) -> Result<RunMode> {
        match (&self.plan, &self.symbol) {
            (Some(plan_path), _) => Ok(RunMode::Plan {
                plan_path: plan_path.clone(),
            }),
            (None, Some(symbol)) => Ok(RunMode::Direct {
                symbol: symbol.clone(),
            }),
            (None, None) => Err(anyhow!(
                "`--symbol` is required unless `--plan` is provided"
            )),
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backtester=info".into()),
        )
        .init();

    let cli = Cli::parse();
    let strategy_src = std::fs::read_to_string(&cli.strategy)
        .map_err(|e| anyhow!("Cannot read strategy '{}': {e}", cli.strategy))?;

    match cli.run_mode()? {
        RunMode::Plan { plan_path } => run_plan_mode(&cli, &strategy_src, &plan_path),
        RunMode::Direct { symbol } => run_direct_mode(&cli, &strategy_src, &symbol),
    }
}

fn run_plan_mode(cli: &Cli, strategy_src: &str, plan_path: &str) -> Result<()> {
    let plan_src = std::fs::read_to_string(plan_path)
        .map_err(|e| anyhow!("Cannot read plan '{}': {e}", plan_path))?;

    info!(
        url = cli.db_url,
        module = cli.db_module,
        "Connecting to SpacetimeDB"
    );
    let client = SpacetimeClient::connect(&cli.db_url, &cli.db_module)?;

    let report = execute_plan(strategy_src, &plan_src, |symbol, timeframe, request| {
        let interval = timeframe.to_string();
        let candles = match request {
            DatasetCandleRequest::WarmupPrefix { before_ms, count } => {
                let requested = u32::try_from(count).map_err(|_| {
                    anyhow!("Warmup requirement {count} exceeds SpacetimeDB candle query limit")
                })?;
                get_candles_before(
                    &client.conn,
                    symbol,
                    &interval,
                    before_ms,
                    requested.min(cli.max_candles),
                )
                .with_context(|| {
                    format!("failed to load warmup candles from DB for {symbol}/{interval}")
                })?
            }
            DatasetCandleRequest::Range { start_ms, end_ms } => get_candles_in_range(
                &client.conn,
                symbol,
                &interval,
                start_ms,
                end_ms,
                cli.max_candles,
            )
            .with_context(|| {
                format!("failed to load ranged candles from DB for {symbol}/{interval}")
            })?,
        };
        if !candles.is_empty() {
            info!(
                symbol,
                interval,
                ?request,
                count = candles.len(),
                "Candles loaded for plan dataset"
            );
        }
        Ok(candles)
    })?;

    print!("{}", render_markdown(&report, &cli.strategy));
    Ok(())
}

fn run_direct_mode(cli: &Cli, strategy_src: &str, symbol: &str) -> Result<()> {
    // ── Load candles from SpacetimeDB ─────────────────────────────────────────
    info!(
        url = cli.db_url,
        module = cli.db_module,
        "Connecting to SpacetimeDB"
    );
    let client = SpacetimeClient::connect(&cli.db_url, &cli.db_module)?;
    // ── Run ──────────────────────────────────────────────────────────────────
    let runtime_result = run_runtime_backtest_with_loader(
        strategy_src,
        RuntimeBacktestConfig::new(symbol.to_string(), cli.balance),
        |symbol, timeframe| {
            let interval = timeframe.to_string();
            let candles = get_candles(&client.conn, symbol, &interval, cli.max_candles)
                .with_context(|| {
                    format!("failed to load candles from DB for {symbol}/{interval}")
                })?;
            if !candles.is_empty() {
                info!(symbol, interval, count = candles.len(), "Candles loaded");
            }
            Ok(candles)
        },
    )?;
    if runtime_result.result.equity_curve.is_empty() {
        return Err(anyhow!(
            "No tradable candles for {}/{} — run `just seed` first.",
            symbol,
            runtime_result.effective_config.primary_timeframe,
        ));
    }
    info!(
        primary_timeframe = %runtime_result.effective_config.primary_timeframe,
        secondary_timeframes = ?runtime_result.effective_config.secondary_timeframes,
        warmup_requirement = runtime_result.warmup_plan.effective_requirement(),
        "Runtime backtest configured"
    );
    let result = runtime_result.result;

    // ── Report ───────────────────────────────────────────────────────────────
    let m = result.metrics;
    let b = result.benchmark;
    println!();
    println!("═══ Backtest summary ═══");
    println!("Strategy          : {}", cli.strategy);
    println!(
        "Symbol / interval : {} / {}",
        symbol, runtime_result.effective_config.primary_timeframe
    );
    println!("Period            : {:>12.2} years", m.years);
    println!("Initial balance   : {:>12.2}", cli.balance);
    println!("Final balance     : {:>12.2}", result.final_balance);
    println!("Final equity      : {:>12.2}", m.final_equity);
    println!("Peak equity       : {:>12.2}", m.peak_equity);
    println!(
        "Max drawdown      : {:>12.2}  ({:.1}%)",
        m.max_drawdown,
        m.max_drawdown_pct * 100.0
    );
    println!("Trades            : {:>12}", m.trade_count);
    println!("  Wins / Losses   : {:>5} / {:>4}", m.wins, m.losses);
    println!("Win rate          : {:>11.1}%", m.win_rate * 100.0);
    println!("Total realised PnL: {:>12.2}", m.total_pnl);
    println!("Total costs       : {:>12.2}", m.total_costs);
    println!("Avg cost / trade  : {:>12.2}", m.average_cost_per_trade);
    let mar_strat = if m.max_drawdown_pct > 0.0 {
        m.cagr / m.max_drawdown_pct
    } else {
        0.0
    };
    let mar_bh = if b.max_drawdown_pct > 0.0 {
        b.cagr / b.max_drawdown_pct
    } else {
        0.0
    };

    println!();
    println!("─── Risk-adjusted scores ───");
    println!(
        "CAGR (strategy)   : {:>11.2}%   {}",
        m.cagr * 100.0,
        rate_cagr(m.cagr)
    );
    println!(
        "CAGR (buy & hold) : {:>11.2}%   {}",
        b.cagr * 100.0,
        rate_cagr(b.cagr)
    );
    println!(
        "MaxDD (strategy)  : {:>11.2}%   {}",
        m.max_drawdown_pct * 100.0,
        rate_maxdd(m.max_drawdown_pct)
    );
    println!(
        "MaxDD (buy & hold): {:>11.2}%   {}",
        b.max_drawdown_pct * 100.0,
        rate_maxdd(b.max_drawdown_pct)
    );
    println!(
        "Sharpe (ann.)     : {:>12.2}   {}",
        m.sharpe,
        rate_sharpe(m.sharpe)
    );
    println!(
        "MAR (strategy)    : {:>12.2}   {}",
        mar_strat,
        rate_mar(mar_strat)
    );
    println!(
        "MAR (buy & hold)  : {:>12.2}   {}",
        mar_bh,
        rate_mar(mar_bh)
    );
    println!(
        "Time in market    : {:>11.1}%   {}",
        m.time_in_market_pct * 100.0,
        rate_tim(m.time_in_market_pct)
    );
    println!("B&H final equity  : {:>12.2}", b.final_equity);
    println!();
    println!("Legend: CAGR=annualised return  MaxDD=max drawdown  Sharpe=return/volatility");
    println!("        MAR=CAGR/MaxDD (return per unit of pain)");
    println!();

    if cli.verbose && !result.trades.is_empty() {
        println!("─── Trades ───");
        println!(
            "{:<4} {:<6} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>9} {:>9}  reason",
            "#", "side", "entry_dt", "exit_dt", "entry", "exit", "size", "gross", "costs", "net", "entry_adj", "exit_adj"
        );
        for (i, t) in result.trades.iter().enumerate() {
            let entry_adjustment = t
                .entry_fill
                .as_ref()
                .map(|fill| fill.price_adjustment)
                .unwrap_or(0.0);
            println!(
                "{:<4} {:<6} {:>10} {:>10} {:>10.2} {:>10.2} {:>10.4} {:>10.2} {:>10.2} {:>10.2} {:>9.2} {:>9.2}  {}",
                i + 1,
                format!("{}", t.side),
                format_date(t.entry_time),
                format_date(t.exit_time),
                t.entry_price,
                t.exit_price,
                t.size,
                t.gross_pnl,
                t.total_costs,
                t.net_realized_pnl,
                entry_adjustment,
                t.exit_fill.price_adjustment,
                t.exit_reason,
            );
        }
        println!();
    }

    Ok(())
}

/// Qualitative rating helpers. Thresholds reflect common practitioner rules of
/// thumb for long-horizon equity strategies (not investment advice).
fn rate_cagr(c: f64) -> &'static str {
    let p = c * 100.0;
    if p < 0.0 {
        "(losing money — strategy destroys capital)"
    } else if p < 3.0 {
        "(poor — barely beats cash/inflation)"
    } else if p < 10.0 {
        "(ok — beats bonds, lags equities)"
    } else if p < 20.0 {
        "(good — equity-like return)"
    } else if p < 40.0 {
        "(excellent — top-quartile if sustainable)"
    } else {
        "(exceptional — verify it's not a fluke / overfit)"
    }
}

fn rate_maxdd(d: f64) -> &'static str {
    let p = d * 100.0;
    if p < 5.0 {
        "(minimal — very smooth)"
    } else if p < 15.0 {
        "(low — easy to stomach)"
    } else if p < 30.0 {
        "(moderate — typical for equities)"
    } else if p < 50.0 {
        "(high — painful but survivable)"
    } else {
        "(severe — most retail traders capitulate here)"
    }
}

fn rate_sharpe(s: f64) -> &'static str {
    if s < 0.0 {
        "(negative — losing on risk-adjusted basis)"
    } else if s < 0.5 {
        "(poor — barely better than noise)"
    } else if s < 1.0 {
        "(mediocre — not really tradeable alone)"
    } else if s < 2.0 {
        "(good — institutionally tradeable)"
    } else if s < 3.0 {
        "(great — rare in practice)"
    } else {
        "(suspicious — likely overfit or look-ahead bug)"
    }
}

fn rate_mar(m: f64) -> &'static str {
    if m < 0.0 {
        "(losing — negative return per unit of drawdown)"
    } else if m < 0.3 {
        "(poor — too much pain per dollar earned)"
    } else if m < 0.6 {
        "(ok — typical buy-and-hold territory)"
    } else if m < 1.0 {
        "(good — solid edge)"
    } else {
        "(excellent — managed-futures grade)"
    }
}

fn rate_tim(t: f64) -> &'static str {
    let p = t * 100.0;
    if p < 20.0 {
        "(mostly cash — missing compounding)"
    } else if p < 50.0 {
        "(selective — only trades on signal)"
    } else if p < 80.0 {
        "(active — usually invested)"
    } else {
        "(near buy-and-hold exposure)"
    }
}

/// Format Unix millisecond timestamp as "YYYY-MM-DD" (UTC, proleptic Gregorian).
fn format_date(ts_ms: i64) -> String {
    let days = ts_ms.div_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

/// Howard Hinnant's `civil_from_days` — days since 1970-01-01 → (year, month, day).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_mode_allows_omitting_symbol() {
        let cli = Cli::try_parse_from(["backtester", "--strategy", "s.rhai", "--plan", "p.rhai"])
            .unwrap();

        assert!(matches!(cli.run_mode().unwrap(), RunMode::Plan { .. }));
    }

    #[test]
    fn direct_mode_requires_symbol_when_no_plan_is_given() {
        let cli = Cli::try_parse_from(["backtester", "--strategy", "s.rhai"]).unwrap();
        let err = cli.run_mode().unwrap_err();

        assert!(err.to_string().contains("--symbol"));
        assert!(err.to_string().contains("--plan"));
    }
}
