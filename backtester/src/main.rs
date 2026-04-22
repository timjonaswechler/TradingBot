//! Terminal backtester.
//!
//! Loads candles from SpacetimeDB (local cache), runs a `.rhai` strategy
//! through the in-memory engine, prints a summary.
//!
//! Usage:
//! ```
//! backtester \
//!     --strategy strategies/sma_cross.rhai \
//!     --symbol   AAPL \
//!     --interval 1d
//! ```
use anyhow::{anyhow, Result};
use clap::Parser;
use tracing::info;

use backtester::{run_backtest, BacktestConfig};
use db_layer::{get_candles, SpacetimeClient};
use engine::{detect_warmup_period, Engine};

#[derive(Parser, Debug)]
#[command(name = "backtester", about = "In-memory backtester — no DB writes")]
struct Cli {
    /// Path to the `.rhai` strategy file.
    #[arg(short, long)]
    strategy: String,

    /// Symbol to backtest (must already be seeded in SpacetimeDB).
    #[arg(short = 'S', long)]
    symbol: String,

    /// Timeframe, e.g. `1d`, `1h`, `15m`.
    #[arg(short, long, default_value = "1d")]
    interval: String,

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

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "backtester=info".into()),
        )
        .init();

    let cli = Cli::parse();

    // ── Load strategy + build engine (single compile, AST reused for warmup) ──
    let src = std::fs::read_to_string(&cli.strategy)
        .map_err(|e| anyhow!("Cannot read strategy '{}': {e}", cli.strategy))?;
    let mut engine = Engine::new(&src)?;
    let warmup_bars = detect_warmup_period(engine.ast(), engine.scope());
    info!(strategy = cli.strategy, warmup_bars, "Strategy compiled");

    // ── Load candles from SpacetimeDB ─────────────────────────────────────────
    info!(url = cli.db_url, module = cli.db_module, "Connecting to SpacetimeDB");
    let client = SpacetimeClient::connect(&cli.db_url, &cli.db_module)?;
    let candles = get_candles(&client.conn, &cli.symbol, &cli.interval, cli.max_candles);
    if candles.is_empty() {
        return Err(anyhow!(
            "No candles for {}/{} — run `just seed` first.",
            cli.symbol, cli.interval,
        ));
    }
    info!(count = candles.len(), "Candles loaded");

    // ── Run ──────────────────────────────────────────────────────────────────
    let result = run_backtest(
        &mut engine,
        candles,
        BacktestConfig { initial_balance: cli.balance, warmup_bars },
    )?;

    // ── Report ───────────────────────────────────────────────────────────────
    let m = result.metrics;
    let b = result.benchmark;
    println!();
    println!("═══ Backtest summary ═══");
    println!("Strategy          : {}", cli.strategy);
    println!("Symbol / interval : {} / {}", cli.symbol, cli.interval);
    println!("Period            : {:>12.2} years", m.years);
    println!("Initial balance   : {:>12.2}", cli.balance);
    println!("Final balance     : {:>12.2}", result.final_balance);
    println!("Final equity      : {:>12.2}", m.final_equity);
    println!("Peak equity       : {:>12.2}", m.peak_equity);
    println!("Max drawdown      : {:>12.2}  ({:.1}%)", m.max_drawdown, m.max_drawdown_pct * 100.0);
    println!("Trades            : {:>12}",    m.trade_count);
    println!("  Wins / Losses   : {:>5} / {:>4}", m.wins, m.losses);
    println!("Win rate          : {:>11.1}%",  m.win_rate * 100.0);
    println!("Total realised PnL: {:>12.2}", m.total_pnl);
    let mar_strat = if m.max_drawdown_pct > 0.0 { m.cagr / m.max_drawdown_pct } else { 0.0 };
    let mar_bh    = if b.max_drawdown_pct > 0.0 { b.cagr / b.max_drawdown_pct } else { 0.0 };

    println!();
    println!("─── Risk-adjusted scores ───");
    println!("CAGR (strategy)   : {:>11.2}%   {}", m.cagr * 100.0, rate_cagr(m.cagr));
    println!("CAGR (buy & hold) : {:>11.2}%   {}", b.cagr * 100.0, rate_cagr(b.cagr));
    println!("MaxDD (strategy)  : {:>11.2}%   {}", m.max_drawdown_pct * 100.0, rate_maxdd(m.max_drawdown_pct));
    println!("MaxDD (buy & hold): {:>11.2}%   {}", b.max_drawdown_pct * 100.0, rate_maxdd(b.max_drawdown_pct));
    println!("Sharpe (ann.)     : {:>12.2}   {}", m.sharpe, rate_sharpe(m.sharpe));
    println!("MAR (strategy)    : {:>12.2}   {}", mar_strat, rate_mar(mar_strat));
    println!("MAR (buy & hold)  : {:>12.2}   {}", mar_bh,    rate_mar(mar_bh));
    println!("Time in market    : {:>11.1}%   {}", m.time_in_market_pct * 100.0, rate_tim(m.time_in_market_pct));
    println!("B&H final equity  : {:>12.2}", b.final_equity);
    println!();
    println!("Legend: CAGR=annualised return  MaxDD=max drawdown  Sharpe=return/volatility");
    println!("        MAR=CAGR/MaxDD (return per unit of pain)");
    println!();

    if cli.verbose && !result.trades.is_empty() {
        println!("─── Trades ───");
        println!("{:<4} {:<6} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10}  reason",
                 "#", "side", "entry_dt", "exit_dt", "entry", "exit", "size", "pnl");
        for (i, t) in result.trades.iter().enumerate() {
            println!(
                "{:<4} {:<6} {:>10} {:>10} {:>10.2} {:>10.2} {:>10.4} {:>10.2}  {}",
                i + 1,
                format!("{}", t.side),
                format_date(t.entry_time),
                format_date(t.exit_time),
                t.entry_price, t.exit_price, t.size, t.pnl, t.exit_reason,
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
    if p <  0.0 { "(losing money — strategy destroys capital)" }
    else if p <  3.0 { "(poor — barely beats cash/inflation)" }
    else if p < 10.0 { "(ok — beats bonds, lags equities)" }
    else if p < 20.0 { "(good — equity-like return)" }
    else if p < 40.0 { "(excellent — top-quartile if sustainable)" }
    else             { "(exceptional — verify it's not a fluke / overfit)" }
}

fn rate_maxdd(d: f64) -> &'static str {
    let p = d * 100.0;
    if p <  5.0 { "(minimal — very smooth)" }
    else if p < 15.0 { "(low — easy to stomach)" }
    else if p < 30.0 { "(moderate — typical for equities)" }
    else if p < 50.0 { "(high — painful but survivable)" }
    else             { "(severe — most retail traders capitulate here)" }
}

fn rate_sharpe(s: f64) -> &'static str {
    if s <  0.0 { "(negative — losing on risk-adjusted basis)" }
    else if s < 0.5 { "(poor — barely better than noise)" }
    else if s < 1.0 { "(mediocre — not really tradeable alone)" }
    else if s < 2.0 { "(good — institutionally tradeable)" }
    else if s < 3.0 { "(great — rare in practice)" }
    else            { "(suspicious — likely overfit or look-ahead bug)" }
}

fn rate_mar(m: f64) -> &'static str {
    if m <  0.0 { "(losing — negative return per unit of drawdown)" }
    else if m < 0.3 { "(poor — too much pain per dollar earned)" }
    else if m < 0.6 { "(ok — typical buy-and-hold territory)" }
    else if m < 1.0 { "(good — solid edge)" }
    else            { "(excellent — managed-futures grade)" }
}

fn rate_tim(t: f64) -> &'static str {
    let p = t * 100.0;
    if p < 20.0 { "(mostly cash — missing compounding)" }
    else if p < 50.0 { "(selective — only trades on signal)" }
    else if p < 80.0 { "(active — usually invested)" }
    else             { "(near buy-and-hold exposure)" }
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
    let y   = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp  = (5 * doy + 2) / 153;
    let d   = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m   = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y   = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
