/// Main bot binary — runs the data collector and logs portfolio state.
/// For backtesting use `cargo run --bin backtest`.
/// For optimization use `cargo run --bin optimize`.

use bot::{collector, config, db};

use anyhow::Result;
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = config::Config::load(Path::new("config.toml"))?;
    log::info!("=== TradingBot started ===");
    log::info!("Watchlist: {:?}", cfg.assets.watchlist);

    let db = db::Db::open(&cfg.db.path)?;
    db.ensure_state_tables()?;

    // Incrementally update market data
    let http = reqwest::Client::new();
    let new_candles = collector::run(&cfg, &db, &http).await?;
    log::info!("Collector: {} new candles saved", new_candles);

    let cash = db.load_cash(cfg.paper_trading.starting_capital)?;
    log::info!("Cash: {:.2}€", cash as f64 / 100.0);
    log::info!(
        "Run `cargo run --bin backtest -- --asset <SYMBOL>` to backtest a strategy."
    );

    Ok(())
}
