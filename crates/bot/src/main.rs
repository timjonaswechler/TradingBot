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

    let cash = db.load_cash(cfg.paper_trading.starting_capital)?;
    let positions = db.load_positions()?;

    let mut engine = paper_trading::PaperTradingEngine::new(
        paper_trading::TradingConfig::from_app_config(&cfg),
    );
    // Restore cash from DB
    engine.cash_cents = cash;
    // Restore positions from DB
    for p in positions {
        engine.positions.insert(p.symbol.clone(), p);
    }

    log::info!(
        "Portfolio: {:.2}€ Cash, {} Positionen",
        engine.cash_cents as f64 / 100.0,
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

        let strat_signal = strategy.signal(&history);
        log::info!("{asset}: Signal = {:?}", strat_signal);

        let candle = &history[0]; // newest candle
        let pt_signal = paper_trading::Signal::from(strat_signal);
        let trade_count_before = engine.trades.len();
        engine.execute(&pt_signal, asset, candle);
        if engine.trades.len() > trade_count_before {
            let trade = engine.trades.last().unwrap();
            db.save_trade(trade)?;
        }
    }

    // ── 3. State persistieren ─────────────────────────────────────────────────
    db.save_cash(engine.cash_cents)?;
    db.save_positions(&engine.positions)?;

    // ── 4. Zusammenfassung ────────────────────────────────────────────────────
    let prices: HashMap<String, i64> = cfg.assets.watchlist.iter()
        .filter_map(|a| {
            db.get_candles(a, &primary, 1).ok()?
                .into_iter().next()
                .map(|c| (a.clone(), c.close))
        })
        .collect();

    let pos_value: i64 = engine.positions.iter()
        .map(|(sym, pos)| prices.get(sym).copied().unwrap_or(0) * pos.quantity)
        .sum();
    let total = engine.cash_cents + pos_value;

    log::info!("══════════════════════════════════════════════");
    log::info!("Cash:                  {:.2}€", engine.cash_cents as f64 / 100.0);
    log::info!("Gesamtwert:            {:.2}€", total as f64 / 100.0);
    log::info!("G/L:                   {:.2}€", (total - cfg.paper_trading.starting_capital) as f64 / 100.0);
    log::info!("Offene Positionen:     {}", engine.positions.len());
    log::info!("Trades diese Session:  {}", engine.trades.len());

    Ok(())
}
