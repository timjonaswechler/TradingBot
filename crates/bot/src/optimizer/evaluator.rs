use std::collections::HashMap;

use rand::Rng;

use crate::config::{CostsConfig, FitnessWeights, PaperTradingConfig, TaxConfig};
use crate::market_data::Candle;
use crate::metrics::Metrics;
use crate::paper_trading::{self, PaperTradingEngine, TradingConfig};

use super::fitness;
use super::genome::Genome;

/// Alle verfügbaren Candle-Daten: asset → interval → candles (älteste zuerst).
pub type CandlePool = HashMap<String, HashMap<String, Vec<Candle>>>;

/// Ergebnis einer einzelnen Individuum-Auswertung.
pub struct EvalResult {
    pub fitness:  f64,
    pub metrics:  Metrics,
    /// Welches Asset + Intervall wurde zufällig gewählt?
    pub asset:    String,
    pub interval: String,
}

/// Wählt zufällig ein Asset und ein Intervall aus dem Pool.
/// Wird einmal pro Generation aufgerufen, damit A und B auf denselben Daten verglichen werden.
pub fn pick_random_asset_interval(pool: &CandlePool, rng: &mut impl Rng) -> Option<(String, String)> {
    let assets: Vec<&String> = pool.keys().collect();
    if assets.is_empty() { return None; }
    let asset = assets[rng.gen_range(0..assets.len())].clone();
    let intervals: Vec<&String> = pool[&asset].keys().collect();
    if intervals.is_empty() { return None; }
    let interval = intervals[rng.gen_range(0..intervals.len())].clone();
    Some((asset, interval))
}

/// Bewertet ein Individuum auf einem vorgegebenen Asset+Intervall.
/// Das Zeitfenster innerhalb der Candle-Reihe wird zufällig gewählt —
/// so testen A und B auf denselben Marktdaten (fairer Vergleich), aber
/// auf unterschiedlichen Zeitabschnitten (Robustheit gegen Overfitting).
pub fn evaluate<G: Genome>(
    genome:      &G,
    pool:        &CandlePool,
    asset:       &str,
    interval:    &str,
    min_window:  usize,
    paper_cfg:   &PaperTradingConfig,
    costs_cfg:   &CostsConfig,
    tax_cfg:     &TaxConfig,
    fitness_cfg: &FitnessWeights,
    rng:         &mut impl Rng,
) -> EvalResult {
    let intervals_map = match pool.get(asset) {
        Some(m) => m,
        None    => return bad_result(asset, interval, paper_cfg.starting_capital),
    };
    let all_candles = match intervals_map.get(interval) {
        Some(c) => c,
        None    => return bad_result(asset, interval, paper_cfg.starting_capital),
    };

    // ── Backtesten auf zufälligem Zeitfenster ─────────────────────────────────
    let strategy = genome.to_strategy();
    let required  = strategy.required_history();
    let min_size  = required + min_window;

    if all_candles.len() < min_size {
        return bad_result(&asset, &interval, paper_cfg.starting_capital);
    }

    let max_start = all_candles.len() - min_size;
    let start     = rng.gen_range(0..=max_start);
    let max_extra = (all_candles.len() - start - min_size).min(min_size * 2);
    let extra     = if max_extra > 0 { rng.gen_range(0..=max_extra) } else { 0 };
    let window    = &all_candles[start..start + min_size + extra];

    let trading_cfg = TradingConfig {
        starting_capital_cents: paper_cfg.starting_capital,
        commission_type: paper_trading::CommissionType::Flat,
        commission_amount: costs_cfg.commission_amount,
        position_size_pct: paper_cfg.position_size_pct as f64 / 100.0,
        max_short_size_pct: 0.5,
        tax: paper_trading::TaxConfig {
            freistellungsauftrag_cents: tax_cfg.freistellungsauftrag,
            kirchensteuer: tax_cfg.kirchensteuer,
            kirchensteuer_rate: 0.09,
        },
    };
    let mut engine = PaperTradingEngine::new(trading_cfg);

    let mut equity_curve: Vec<i64> = Vec::with_capacity(window.len());

    for t in (required - 1)..window.len() {
        let slice: Vec<Candle> = window[t + 1 - required..=t]
            .iter()
            .rev()
            .cloned()
            .collect();

        let candle = &window[t];
        let pt_signal = paper_trading::Signal::from(strategy.signal(&slice));
        engine.execute(&pt_signal, asset, candle);

        let pos_value: i64 = engine.positions.values()
            .map(|p| p.quantity * candle.close)
            .sum();
        equity_curve.push(engine.cash_cents + pos_value);
    }

    if equity_curve.is_empty() {
        return bad_result(&asset, &interval, paper_cfg.starting_capital);
    }

    let start_ts = window[required - 1].timestamp;
    let end_ts   = window.last().unwrap().timestamp;
    let days     = (end_ts - start_ts).num_days().unsigned_abs();

    let metrics = Metrics::compute(&equity_curve, &engine.trades, days);
    let fitness = fitness::score(&metrics, fitness_cfg);

    EvalResult { fitness, metrics, asset: asset.to_string(), interval: interval.to_string() }
}

fn bad_result(asset: &str, interval: &str, capital: i64) -> EvalResult {
    EvalResult {
        fitness:  f64::NEG_INFINITY,
        metrics:  Metrics::compute(&[capital], &[], 0),
        asset:    asset.to_string(),
        interval: interval.to_string(),
    }
}
