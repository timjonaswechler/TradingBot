/// Backtest CLI binary.
use bot::{config, db, metrics, paper_trading::PaperTradingEngine, strategy};
use bot::{config, db, metrics::Metrics, paper_trading, strategy};

use anyhow::Result;
use std::path::Path;

/// Backtest-Binary: liest historische Candles aus SQLite,
/// simuliert die Strategie Schritt für Schritt und gibt Metriken aus.
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
    let cfg = config::Config::load(Path::new("config.toml"))?;
    let db  = db::Db::open(&cfg.db.path)?;

    // Asset aus Kommandozeile oder alle aus Watchlist
    let assets: Vec<String> = {
        let mut args = std::env::args();
        if let Some(pos) = args.position(|a| a == "--asset") {
            std::env::args().nth(pos + 1)
                .map(|a| vec![a])
                .unwrap_or_else(|| cfg.assets.watchlist.clone())
        } else {
            cfg.assets.watchlist.clone()
        }
    };

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
    struct AssetResult {
        asset:   String,
        metrics: metrics::Metrics,
        trades:  usize,
    }
    let mut results: Vec<AssetResult> = Vec::new();

    let trading_cfg = paper_trading::TradingConfig::from_app_config(&cfg);

    for asset in &assets {
        let candles = db.get_all_candles_asc(asset, cfg.data.primary_interval())?;
        let h = strategy.required_history();

        if candles.len() < h {
            println!(
                "  {asset}: ⚠  Nicht genug Daten ({}/{} Candles). Zuerst `cargo run -p bot`.",
                candles.len(), h
            );
            continue;
        }

        println!(
            "  {asset}: {} Candles  ({} → {})",
            candles.len(),
            candles.first().unwrap().timestamp.format("%Y-%m-%d"),
            candles.last().unwrap().timestamp.format("%Y-%m-%d"),
        );

        let mut engine = paper_trading::PaperTradingEngine::new(trading_cfg.clone());

        let mut equity_curve: Vec<(chrono::DateTime<chrono::Utc>, i64)> =
            Vec::with_capacity(candles.len() - h + 1);

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
            let candle = &candles[t];
            let pt_signal = paper_trading::Signal::from(strategy.signal(&window));
            engine.execute(&pt_signal, asset, candle);

            let pos_value: i64 = engine
                .positions
                .values()
                .map(|p| p.quantity * candle.close)
                .sum();
            equity_curve.push((candles[t].timestamp, engine.cash + pos_value));
            equity_curve.push(engine.cash_cents + pos_value);
        }

        let trade_records = metrics::from_paper_trades(&engine.trades);
        let m = metrics::compute(&equity_curve, &trade_records, cfg.paper_trading.starting_capital);
        let trade_count = engine.trades.len();

        results.push(AssetResult { asset: asset.clone(), metrics: m, trades: trade_count });
    }

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
    // ── Detailausgabe pro Asset ───────────────────────────────────────────────
    for r in &results {
        let m = &r.metrics;
        println!("\n═══ Backtest: {} ══════════════════════════════════════════════════", r.asset);
        println!("  Startkapital:     {:.2} €", cfg.paper_trading.starting_capital as f64 / 100.0);
        println!();
        println!("── Rendite ─────────────────────────────────────────────────────────────");
        println!("  Total Return:     {:+.2} %", m.total_return_pct);
        println!("  Total PnL:        {:.2} €", m.total_pnl_cents as f64 / 100.0);
        println!();
        println!("── Risiko ──────────────────────────────────────────────────────────────");
        println!("  Sharpe Ratio:     {:.2}", m.sharpe);
        println!("  Max Drawdown:     {:.2} %", m.max_drawdown_pct);
        println!();
        println!("── Trades ──────────────────────────────────────────────────────────────");
        println!("  Gesamt:           {}", m.total_trades);
        println!("  Gewinner:         {} ({:.1} %)", m.winning_trades, m.win_rate_pct);
        println!("  Verlierer:        {}", m.losing_trades);
        println!();
        println!("── Ø Trade-Performance ─────────────────────────────────────────────────");
        println!("  Ø Gewinn/Trade:   {:+.2} %", m.avg_win_pct);
        println!("  Ø Verlust/Trade:  -{:.2} %", m.avg_loss_pct);
        println!("  Erwartungswert:   {:+.2} % pro Trade", m.expectancy_pct);
        println!("════════════════════════════════════════════════════════════════════════");
    }

    // ── Vergleichstabelle (nur bei mehreren Assets) ───────────────────────────
    if results.len() > 1 {
        let sep = "═".repeat(80);
        let div = "─".repeat(80);
        println!("\n {sep}");
        println!(" VERGLEICH");
        println!(" {div}");
        println!(
            " {:<8}  {:>9}  {:>8}  {:>8}  {:>7}  {:>7}",
            "Asset", "Return %", "Sharpe", "MaxDD %", "WinRate", "Trades"
        );
        println!(" {div}");

        let mut sorted = results.iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| b.metrics.total_return_pct.partial_cmp(&a.metrics.total_return_pct).unwrap_or(std::cmp::Ordering::Equal));

        for r in &sorted {
            let m = &r.metrics;
            println!(
                " {:<8}  {:>+8.2}%  {:>8.2}  {:>7.2}%  {:>7.1}%  {:>7}",
                r.asset,
                m.total_return_pct,
                m.sharpe,
                m.max_drawdown_pct,
                m.win_rate_pct,
                r.trades,
            );
        }
        println!(" {sep}\n");
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
