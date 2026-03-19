use bot::{config, db, metrics::Metrics, paper_trading::PaperTradingEngine, strategy};

use anyhow::Result;
use std::path::Path;

/// Backtest-Binary: liest historische Candles aus SQLite,
/// simuliert die Strategie Schritt für Schritt und gibt Metriken aus.
///
/// Voraussetzung: erst `cargo run -p bot` ausführen um Daten zu laden.
///
/// Optionale Argumente:
///   --asset SPY          (default: alle Assets aus watchlist)
fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cfg = config::Config::load(Path::new("config.toml"))?;
    let db  = db::Db::open(&cfg.db.path)?;

    // Asset aus Kommandozeile oder alle aus Watchlist
    let assets: Vec<String> = {
        let mut args = std::env::args();
        if let Some(pos) = args.position(|a| a == "--asset") {
            // --asset wurde gefunden, nächstes Argument ist der Asset-Name
            std::env::args().nth(pos + 1)
                .map(|a| vec![a])
                .unwrap_or_else(|| cfg.assets.watchlist.clone())
        } else {
            cfg.assets.watchlist.clone()
        }
    };

    let strategy = strategy::from_config(&cfg.strategy)?;

    println!("\n Backtest: {}  |  Strategie: {}", assets.join(", "), strategy.name());
    println!(" Startkapital pro Asset: {:.2} €\n", cfg.paper_trading.starting_capital as f64 / 100.0);

    // Ergebnisse sammeln für Tabelle am Ende
    struct AssetResult {
        asset:   String,
        metrics: Metrics,
        days:    u64,
        trades:  usize,
    }
    let mut results: Vec<AssetResult> = Vec::new();

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

        let mut engine = PaperTradingEngine::new(
            cfg.paper_trading.starting_capital,
            cfg.tax.freistellungsauftrag,
            vec![],
            cfg.costs.clone(),
            cfg.tax.clone(),
            cfg.paper_trading.position_size_pct,
        );

        let mut equity_curve: Vec<i64> = Vec::with_capacity(candles.len() - h + 1);

        for t in (h - 1)..candles.len() {
            let window: Vec<_> = candles[t + 1 - h..=t]
                .iter()
                .rev()
                .cloned()
                .collect();

            let current_price = candles[t].close;
            let signal = strategy.signal(&window);
            engine.execute(&signal, asset, current_price, strategy.name())?;

            let pos_value: i64 = engine
                .positions
                .iter()
                .filter(|p| p.asset == *asset)
                .map(|p| p.quantity * current_price)
                .sum();
            equity_curve.push(engine.cash + pos_value);
        }

        let start_ts = candles[h - 1].timestamp;
        let end_ts   = candles.last().unwrap().timestamp;
        let days     = (end_ts - start_ts).num_days().unsigned_abs();

        let m = Metrics::compute(&equity_curve, &engine.trades, days);
        let trade_count = engine.trades.len();

        results.push(AssetResult { asset: asset.clone(), metrics: m, days, trades: trade_count });
    }

    if results.is_empty() {
        println!("\n Keine Ergebnisse – bitte zuerst Daten laden.");
        return Ok(());
    }

    // ── Detailausgabe pro Asset ───────────────────────────────────────────────
    for r in &results {
        r.metrics.print(&r.asset, strategy.name(), r.days);
    }

    // ── Vergleichstabelle (nur bei mehreren Assets) ───────────────────────────
    if results.len() > 1 {
        let sep = "═".repeat(88);
        let div = "─".repeat(88);
        println!("\n {sep}");
        println!(" VERGLEICH");
        println!(" {div}");
        println!(
            " {:<8}  {:>9}  {:>8}  {:>8}  {:>8}  {:>8}  {:>7}",
            "Asset", "Return %", "CAGR %", "Sharpe", "MaxDD %", "WinRate", "Trades"
        );
        println!(" {div}");

        let mut sorted = results.iter().collect::<Vec<_>>();
        sorted.sort_by(|a, b| b.metrics.cagr_pct.partial_cmp(&a.metrics.cagr_pct).unwrap());

        for r in &sorted {
            let m = &r.metrics;
            println!(
                " {:<8}  {:>+8.2}%  {:>+7.2}%  {:>8.2}  {:>7.2}%  {:>7.1}%  {:>7}",
                r.asset,
                m.total_return_pct,
                m.cagr_pct,
                m.sharpe,
                m.max_drawdown_pct,
                m.win_rate_pct,
                r.trades,
            );
        }
        println!(" {sep}\n");
    }

    Ok(())
}
