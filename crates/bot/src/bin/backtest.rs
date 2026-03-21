/// Backtest CLI binary.
///
/// Usage:
///   cargo run --bin backtest -- --asset AAPL [--primary 1d] [--secondary 1h] \
///                               [--genome genome.toml] [--config config.toml]
///
/// Prerequisites: run `cargo run --bin collector` first to populate the DB.

use anyhow::{bail, Context, Result};
use std::path::PathBuf;

use bot::{
    config::Config,
    db::Db,
    metrics,
    optimizer::DualMacdGenome,
    paper_trading::{PaperTradingEngine, TradingConfig},
    strategy::{dual_macd::DualMacdStrategy, Strategy},
};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // ── Parse CLI args ────────────────────────────────────────────────────────
    let args: Vec<String> = std::env::args().collect();

    let config_path = arg_value(&args, "--config")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("config.toml"));

    let cfg = Config::load(&config_path)
        .with_context(|| format!("failed to load config from '{}'", config_path.display()))?;

    let asset = match arg_value(&args, "--asset") {
        Some(a) => a,
        None => bail!("--asset <SYMBOL> is required. Example: --asset AAPL"),
    };

    let primary_interval = arg_value(&args, "--primary")
        .unwrap_or_else(|| cfg.strategy.primary_interval.clone());

    let secondary_interval = arg_value(&args, "--secondary")
        .unwrap_or_else(|| cfg.strategy.secondary_interval.clone());

    let genome_path = arg_value(&args, "--genome").map(PathBuf::from);

    // ── Open DB ───────────────────────────────────────────────────────────────
    let db = Db::open(&cfg.db.path)?;

    // ── Load candles ──────────────────────────────────────────────────────────
    let primary_candles = db.get_all_candles_asc(&asset, &primary_interval)?;
    let secondary_candles = db.get_all_candles_asc(&asset, &secondary_interval)?;

    if primary_candles.is_empty() {
        bail!(
            "No {} candles found for {}. Run `cargo run --bin collector` first.",
            primary_interval, asset
        );
    }

    // ── Load or default genome ────────────────────────────────────────────────
    let genome = if let Some(ref path) = genome_path {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read genome file '{}'", path.display()))?;
        DualMacdGenome::from_toml(&contents)
            .map_err(|e| anyhow::anyhow!("failed to parse genome from '{}': {}", path.display(), e))?
    } else {
        DualMacdGenome::default_genome()
    };

    // ── Build strategy ────────────────────────────────────────────────────────
    let strategy = DualMacdStrategy { params: genome.params.clone() };
    let required = strategy.required_history();

    if primary_candles.len() < required {
        bail!(
            "Not enough candles for {} (have {}, need {}). Run `cargo run --bin collector`.",
            asset, primary_candles.len(), required
        );
    }

    // ── Run paper trading simulation ──────────────────────────────────────────
    let trading_cfg = TradingConfig {
        starting_capital_cents: cfg.paper_trading.starting_capital,
    };
    let mut engine = PaperTradingEngine::new(trading_cfg);

    for i in required..primary_candles.len() {
        // Primary slice: newest-first view of candles up to and including index i
        let primary_slice: Vec<_> = primary_candles[0..=i]
            .iter()
            .rev()
            .cloned()
            .collect();

        // Secondary slice aligned to same window
        let secondary_slice: Vec<_> = if secondary_candles.is_empty() {
            vec![]
        } else {
            let sec_end = secondary_candles.len().min(i + 1);
            secondary_candles[0..sec_end]
                .iter()
                .rev()
                .cloned()
                .collect()
        };

        let sig = strategy.signal(&primary_slice, &secondary_slice);
        engine.execute(&sig, &asset, &primary_candles[i]);
        engine.snapshot_equity(&asset, primary_candles[i].close, primary_candles[i].timestamp);
    }

    // ── Compute and print metrics ─────────────────────────────────────────────
    let trade_records = metrics::from_engine_trades(&engine.trades);
    let m = metrics::compute(
        &engine.equity_curve,
        &trade_records,
        cfg.paper_trading.starting_capital,
    );

    println!(
        "=== Backtest Results: {} ({} / {}) ===",
        asset, primary_interval, secondary_interval
    );
    println!("  Total trades:    {}", m.total_trades);
    println!("  Win rate:        {:.1}%", m.win_rate_pct);
    println!("  Expectancy:      {:.2}%", m.expectancy_pct);
    println!("  Sharpe ratio:    {:.2}", m.sharpe);
    println!("  Max drawdown:    {:.1}%", m.max_drawdown_pct);
    println!("  Total return:    {:.1}%", m.total_return_pct);
    println!("  Total PnL:       €{:.2}", m.total_pnl_cents as f64 / 100.0);

    Ok(())
}

/// Returns the value following `key` in `args`, if present.
fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == key)
        .map(|w| w[1].clone())
}
