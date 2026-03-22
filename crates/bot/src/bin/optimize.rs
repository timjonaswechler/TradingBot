/// Optimize CLI binary.
///
/// Usage:
///   cargo run --bin optimize -- [--population 25] [--generations 100] \
///                               [--output best_genome.toml] [--config config.toml]
///
/// Prerequisites: run `cargo run --bin collector` first to populate the DB.

use anyhow::{bail, Context, Result};
/// optimize binary — entry point for the DualMacd genetic optimizer.
///
/// Usage:
///   cargo run -p bot --bin optimize
///
/// Requires a [optimizer] block in config.toml and candle data in the DB.
use std::collections::HashMap;
use std::path::PathBuf;

use bot::{
    config::Config,
    db::Db,
    optimizer::{CandlePool, OptimizerConfig, run as optimizer_run},
};
use anyhow::{bail, Result};
use std::path::Path;

use bot::{
    config,
    db,
    optimizer::{run, CandlePool, OptimizerConfig},
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

    let population_size = arg_value(&args, "--population")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cfg.optimizer.population_size);

    let max_generations = arg_value(&args, "--generations")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(cfg.optimizer.max_generations);

    let output_path = arg_value(&args, "--output")
        .unwrap_or_else(|| "best_genome.toml".to_string());

    // ── Open DB ───────────────────────────────────────────────────────────────
    let db = Db::open(&cfg.db.path)?;

    // ── Determine asset list: CLI assets override > optimizer.assets > watchlist
    let assets: Vec<String> = if !cfg.optimizer.assets.is_empty() {
        cfg.optimizer.assets.clone()
    } else {
        cfg.assets.watchlist.clone()
    };

    if assets.is_empty() {
        bail!("No assets configured. Set optimizer.assets or assets.watchlist in config.toml.");
    }

    let primary_interval   = cfg.strategy.primary_interval.clone();
    let secondary_interval = cfg.strategy.secondary_interval.clone();
    // Deduplicate so we don't load the same interval twice when primary == secondary.
    let mut intervals: Vec<&str> = vec![&primary_interval];
    if secondary_interval != primary_interval {
        intervals.push(&secondary_interval);
    }

    // ── Load candles into pool ────────────────────────────────────────────────
    let mut pool: CandlePool = HashMap::new();
    let mut total_series = 0usize;

    for asset in &assets {
        for interval in &intervals {
            match db.get_all_candles_asc(asset, interval) {
                Ok(candles) if !candles.is_empty() => {
                    total_series += 1;
                    pool.insert((asset.clone(), interval.to_string()), candles);
                }
                Ok(_) => {
                    log::warn!("No candles for {} {} — skipping.", asset, interval);
                }
                Err(e) => {
                    log::warn!("Failed to load candles for {} {}: {} — skipping.", asset, interval, e);
    let cfg = config::Config::load(Path::new("config.toml"))?;
    let db  = db::Db::open(&cfg.db.path)?;

    // ── Load all candle data ──────────────────────────────────────────────────
    let mut pool: CandlePool = HashMap::new();
    let mut total_series = 0usize;

    for asset in &cfg.assets.watchlist {
        for interval in &cfg.data.intervals {
            if let Ok(candles) = db.get_all_candles_asc(asset, interval) {
                if !candles.is_empty() {
                    total_series += 1;
                    pool.insert((asset.clone(), interval.clone()), candles);
                }
            }
        }
    }

    if pool.is_empty() {
        bail!(
            "No candle data found. Run `cargo run --bin collector` first."
        );
    }

    let assets_count = assets.len();
    let opt_cfg = OptimizerConfig {
        population_size,
        max_generations,
        initial_mutation: cfg.optimizer.initial_mutation,
        assets,
    };

    println!("=== Optimizer ===");
    println!("  Assets:      {} ({} series)", assets_count, total_series);
    println!("  Population:  {}", population_size);
    println!("  Generations: {}", max_generations);
    println!("  Mutation:    {:.0}%", cfg.optimizer.initial_mutation * 100.0);
    println!("  Min window:  {} candles", cfg.optimizer.min_window_candles);
    println!("  Fitness:     sharpe×{}  win_rate×{}  expectancy×{}  drawdown×{}",
        cfg.optimizer.fitness.sharpe,
        cfg.optimizer.fitness.win_rate,
        cfg.optimizer.fitness.expectancy,
        cfg.optimizer.fitness.drawdown,
    );
    println!("────────────────────────────────────────");

    // ── Run optimizer ─────────────────────────────────────────────────────────
    let result = optimizer_run(opt_cfg, &pool);

    // ── Print results ─────────────────────────────────────────────────────────
    println!("\n=== Optimization Complete ===");
    println!("  Best fitness:  {:.4}", result.best_fitness);
    println!("  Generations:   {}", result.generations.len());
    println!(
        "  Winner MACD:   fast={} slow={} signal={}",
        result.winner.params.fast,
        result.winner.params.slow,
        result.winner.params.signal,
    );

    // ── Save best genome ──────────────────────────────────────────────────────
    if let Some(parent) = std::path::Path::new(&output_path).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(&output_path, result.winner.to_toml())?;
    println!("  Saved to: {}", output_path);

    Ok(())
}

/// Returns the value following `key` in `args`, if present.
fn arg_value(args: &[String], key: &str) -> Option<String> {
    args.windows(2)
        .find(|w| w[0] == key)
        .map(|w| w[1].clone())
        bail!("No candle data found. Run `cargo run -p bot --bin collector` first.");
    }

    println!("\n══ DualMacd Optimizer ═══════════════════════════════════════════════════");
    println!("  Assets:       {} ({total_series} asset×interval pairs)", cfg.assets.watchlist.len());
    println!("════════════════════════════════════════════════════════════════════════\n");

    let opt_cfg = OptimizerConfig {
        assets: cfg.assets.watchlist.clone(),
        ..Default::default()
    };

    let result = run(opt_cfg, &pool);

    println!("\n══ Best genome ═════════════════════════════════════════════════════════");
    println!("  Fitness: {:.4}", result.best_fitness);
    println!("  Primary interval:   {}", result.winner.primary_interval);
    println!("  Secondary interval: {}", result.winner.secondary_interval);
    println!();
    println!("{}", result.winner.to_toml());
    println!("════════════════════════════════════════════════════════════════════════");

    std::fs::create_dir_all("data")?;
    let toml_str = result.winner.to_toml();
    std::fs::write("data/best_dual_macd.toml", &toml_str)?;
    println!("  Saved to: data/best_dual_macd.toml");

    Ok(())
}
