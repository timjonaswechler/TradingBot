use bot::{collector, config, db, paper_trading, strategy};

use anyhow::Result;
use std::{collections::HashMap, path::Path};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cfg = config::Config::load(Path::new("config.toml"))?;
    log::info!("=== TradingBot gestartet ===");
    log::info!("Watchlist: {:?}", cfg.assets.watchlist);

    let db = db::Db::open(&cfg.db.path)?;
    db.ensure_state_tables()?;

    // ── 1. Marktdaten inkrementell aktualisieren ──────────────────────────────
    let http = reqwest::Client::new();
    let new_candles = collector::run(&cfg, &db, &http).await?;
    log::info!("Collector: {new_candles} neue Candles gespeichert");

    // ── 2. Strategie & Paper Trading ──────────────────────────────────────────
    let strategy = strategy::from_config(&cfg.strategy)?;
    log::info!("Strategie: {}", strategy.name());

    let cash                = db.load_cash(cfg.paper_trading.starting_capital)?;
    let exemption_remaining = db.load_exemption_remaining(cfg.tax.freistellungsauftrag)?;
    let positions           = db.load_positions()?;

    let mut engine = paper_trading::PaperTradingEngine::new(
        cash,
        exemption_remaining,
        positions,
        cfg.costs.clone(),
        cfg.tax.clone(),
        cfg.paper_trading.position_size_pct,
    );
    log::info!(
        "Portfolio: {:.2}€ Cash, {} Positionen",
        engine.cash as f64 / 100.0,
        engine.positions.len()
    );

    let primary = cfg.data.primary_interval().to_string();

    for asset in &cfg.assets.watchlist {
        let history = db.get_candles(asset, &primary, strategy.required_history())?;
        if history.len() < strategy.required_history() {
            log::warn!(
                "{asset}: Nicht genug Historie ({}/{}), überspringe",
                history.len(), strategy.required_history()
            );
            continue;
        }

        let signal = strategy.signal(&history);
        log::info!("{asset}: Signal = {:?}", signal);

        let current_price = history[0].close;
        if let Some(trade) = engine.execute(&signal, asset, current_price, strategy.name())? {
            db.save_trade(&trade)?;
        }
    }

    // ── 3. State persistieren ─────────────────────────────────────────────────
    db.save_cash(engine.cash)?;
    db.save_exemption_remaining(engine.exemption_remaining)?;
    db.save_positions(&engine.positions)?;

    // ── 4. Zusammenfassung ────────────────────────────────────────────────────
    let prices: HashMap<String, i64> = cfg.assets.watchlist.iter()
        .filter_map(|a| {
            db.get_candles(a, &primary, 1).ok()?
                .into_iter().next()
                .map(|c| (a.clone(), c.close))
        })
        .collect();

    let total = engine.total_value(&prices);
    log::info!("══════════════════════════════════════════════");
    log::info!("Cash:                  {:.2}€", engine.cash as f64 / 100.0);
    log::info!("Gesamtwert:            {:.2}€", total as f64 / 100.0);
    log::info!("G/L:                   {:.2}€", (total - cfg.paper_trading.starting_capital) as f64 / 100.0);
    log::info!("Offene Positionen:     {}", engine.positions.len());
    log::info!("Trades diese Session:  {}", engine.trades.len());
    log::info!("Freistellungsauftrag:  {:.2}€ verbleibend", engine.exemption_remaining as f64 / 100.0);

    Ok(())
}
