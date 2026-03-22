use bot::{config, db, metrics, paper_trading::PaperTradingEngine, strategy};
use bot::{config, db, metrics::Metrics, paper_trading, strategy};

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

        for t in (h - 1)..candles.len() {
            let window: Vec<_> = candles[t + 1 - h..=t]
                .iter()
                .rev()
                .cloned()
                .collect();

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

    if results.is_empty() {
        println!("\n Keine Ergebnisse – bitte zuerst Daten laden.");
        return Ok(());
    }

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

    Ok(())
}
