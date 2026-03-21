use std::collections::HashMap;

use bot::{config, db, optimizer::{engine, evaluator::CandlePool, genome::*}};
use bot::strategy::macd_enhanced::MacdEnhancedParams;

use anyhow::{bail, Result};
use std::path::Path;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let cfg = config::Config::load(Path::new("config.toml"))?;
    let opt = cfg.optimizer.as_ref()
        .ok_or_else(|| anyhow::anyhow!("[optimizer] Block fehlt in config.toml"))?;

    let db = db::Db::open(&cfg.db.path)?;

    // ── Alle Candles für alle Assets + alle Intervalle laden ─────────────────
    // Jedes Individuum wählt beim Backtesten zufällig Asset + Intervall.
    let mut pool: CandlePool = HashMap::new();
    let mut total_series = 0usize;

    for asset in &cfg.assets.watchlist {
        let mut interval_map: HashMap<String, Vec<bot::market_data::Candle>> = HashMap::new();
        for interval in &cfg.data.intervals {
            if let Ok(candles) = db.get_all_candles_asc(asset, interval) {
                if !candles.is_empty() {
                    total_series += 1;
                    interval_map.insert(interval.clone(), candles);
                }
            }
        }
        if !interval_map.is_empty() {
            pool.insert(asset.clone(), interval_map);
        }
    }

    if pool.is_empty() {
        bail!("Keine Candle-Daten gefunden. Zuerst: cargo run -p bot --bin collector");
    }

    println!("\n══ Optimizer ═══════════════════════════════════════════════════════════");
    println!("  Strategie:   {}", opt.strategy);
    println!("  Assets:      {} ({total_series} Asset×Intervall-Kombinationen)",
             pool.len());
    println!("  Generationen:{}", opt.max_generations);
    println!("  Population:  {} ({}×{} Gruppen)", opt.population_size,
             opt.population_size / 2, 2);
    println!("  Mutation:    {:.0}%", opt.mutation_magnitude * 100.0);
    println!("  Fenster:     min. {} Candles", opt.min_window_candles);
    println!("  Fitness:     WinRate×{}  Exp×{}  AvgWin×{}  AvgLoss×{}  Sharpe×{}  DD×{}",
             opt.fitness.win_rate, opt.fitness.expectancy, opt.fitness.avg_win,
             opt.fitness.avg_loss, opt.fitness.sharpe, opt.fitness.drawdown);
    println!("               Score = win_rate_pct × {:.1}  (0–100, Ziel: 100)",
             opt.fitness.win_rate);
    println!("────────────────────────────────────────────────────────────────────────\n");

    match opt.strategy.as_str() {
        "macd_enhanced" => run_macd_enhanced(&cfg, &pool)?,
        "rsi"           => run_rsi(&cfg, &pool)?,
        other           => bail!("Unbekannte Strategie: '{other}'. Verfügbar: macd_enhanced, rsi"),
    }

    Ok(())
}

// ── MACD Enhanced ─────────────────────────────────────────────────────────────

fn run_macd_enhanced(cfg: &config::Config, pool: &CandlePool) -> Result<()> {
    let opt = cfg.optimizer.as_ref().unwrap();
    let s   = &cfg.strategy;

    let base = MacdEnhancedParams {
        fast_period:   s.macd_fast.unwrap_or(12),
        slow_period:   s.macd_slow.unwrap_or(26),
        signal_period: s.macd_signal.unwrap_or(9),
        ..Default::default()
    };
    let seed = MacdEnhancedGenome(base);

    // Vorherige Gewinner laden wenn vorhanden
    let prev = MacdEnhancedGenome::load_prev_winners("data/best_macd_enhanced.toml");
    if prev.is_some() {
        println!("  Vorherige Ergebnisse gefunden: data/best_macd_enhanced.toml");
    }

    let (result, _logs) = engine::run(&seed, prev, pool, opt, &cfg.paper_trading, &cfg.costs, &cfg.tax);

    print_and_save("macd_enhanced", &result.winner_a, result.score_a, &result.winner_b, result.score_b)
}

// ── RSI ───────────────────────────────────────────────────────────────────────

fn run_rsi(cfg: &config::Config, pool: &CandlePool) -> Result<()> {
    let opt    = cfg.optimizer.as_ref().unwrap();
    let period = cfg.strategy.rsi_period.unwrap_or(14);
    let seed   = RsiGenome::new_random(period, &mut rand::thread_rng());

    let prev = None; // RSI: noch kein Laden implementiert
    let (result, _logs) = engine::run(&seed, prev, pool, opt, &cfg.paper_trading, &cfg.costs, &cfg.tax);

    print_and_save("rsi", &result.winner_a, result.score_a, &result.winner_b, result.score_b)
}

// ── Ausgabe & Speichern ───────────────────────────────────────────────────────

fn print_and_save<G: Genome>(
    name:    &str,
    a:       &G,
    score_a: f64,
    b:       &G,
    score_b: f64,
) -> Result<()> {
    println!("\n══ Ergebnis ════════════════════════════════════════════════════════════");
    println!("── Gewinner A  (Fitness: {score_a:+.4}) ─────────────────────────────────");
    println!("{}", a.to_toml());
    println!("── Gewinner B  (Fitness: {score_b:+.4}) ─────────────────────────────────");
    println!("{}", b.to_toml());
    println!("════════════════════════════════════════════════════════════════════════");

    std::fs::create_dir_all("data")?;
    let path = format!("data/best_{name}.toml");
    let now  = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");

    let content = format!(
        "# Optimierungsergebnis: {name}\n\
         # Erstellt:  {now}\n\n\
         [winner_a]\n\
         fitness = {score_a:.6}\n\
         {}\n\
         [winner_b]\n\
         fitness = {score_b:.6}\n\
         {}\n",
        a.to_toml(),
        b.to_toml(),
    );

    std::fs::write(&path, &content)?;
    println!("\n  Gespeichert: {path}");
    println!("  → Werte aus [winner_a] in config.toml [strategy] eintragen um sie im Bot zu nutzen.");
    Ok(())
}
