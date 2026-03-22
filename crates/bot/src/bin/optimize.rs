/// optimize binary — entry point for the DualMacd genetic optimizer.
///
/// Usage:
///   cargo run -p bot --bin optimize
///
/// Requires a [optimizer] block in config.toml and candle data in the DB.
use std::collections::HashMap;

use anyhow::{bail, Result};
use std::path::Path;

use bot::{
    config,
    db,
    optimizer::{run, CandlePool, OptimizerConfig},
};

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

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
