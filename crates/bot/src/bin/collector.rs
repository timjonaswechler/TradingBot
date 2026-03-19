use anyhow::Result;
use bot::{collector, config};
use std::path::Path;

/// Standalone Data Collector — fetcht inkrementell Marktdaten ohne Trading-Logik.
/// Ideal als Cron-Job während Marktzeiten.
///
/// Verwendung:
///   cargo run --bin collector              # alle Assets + Intervalle
///   cargo run --bin collector -- --asset AAPL
#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let asset_filter: Option<String> = std::env::args()
        .skip_while(|a| a != "--asset")
        .nth(1);

    let cfg = config::Config::load(Path::new("config.toml"))?;

    // Bei --asset: temporäre Config mit nur diesem Asset
    let effective_cfg = if let Some(ref asset) = asset_filter {
        let mut c = cfg;
        c.assets.watchlist = vec![asset.clone()];
        c
    } else {
        cfg
    };

    let db   = bot::db::Db::open(&effective_cfg.db.path)?;
    let http = reqwest::Client::new();

    let start = std::time::Instant::now();
    let n     = collector::run(&effective_cfg, &db, &http).await?;

    println!("\n══ Collector fertig: {n} neue Candles in {:.1}s ══", start.elapsed().as_secs_f64());
    Ok(())
}
