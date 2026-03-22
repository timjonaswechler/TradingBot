use std::collections::HashMap;

use rand::Rng;

use crate::market_data::Candle;
use crate::metrics::{compute as compute_metrics, from_engine_trades};
use crate::optimizer::fitness::{score, FitnessWeights};
use crate::optimizer::genome::DualMacdGenome;
use crate::paper_trading::engine::{PaperTradingEngine, TradingConfig};
use crate::strategy::DualStrategy;

/// Lookup table: `(asset, interval)` → candles in chronological order (oldest first).
pub type CandlePool = HashMap<(String, String), Vec<Candle>>;

/// Configuration that controls a single evaluation run.
pub struct EvalConfig {
    /// Minimum number of candles for the backtest window (default 50).
    pub min_window_candles: usize,
    /// Extra randomisation added on top of `min_window_candles` (default 100).
    pub extra_random_candles: usize,
    /// Fitness weight configuration.
    pub fitness_weights: FitnessWeights,
    /// Paper-trading engine configuration.
    pub trading_cfg: TradingConfig,
}
use crate::metrics::{self, Metrics};
use crate::paper_trading::PaperTradingEngine;
use crate::metrics::Metrics;
use crate::paper_trading::{self, PaperTradingEngine, TradingConfig};

use super::fitness;
use super::genome::Genome;

/// Alle verfügbaren Candle-Daten: asset → interval → candles (älteste zuerst).
pub type CandlePool = HashMap<String, HashMap<String, Vec<Candle>>>;

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            min_window_candles:   50,
            extra_random_candles: 100,
            fitness_weights:      FitnessWeights::default(),
            trading_cfg:          TradingConfig::default(),
        }
    }
}

/// Result produced by a single genome evaluation.
#[derive(Debug, Clone)]
pub struct EvalResult {
    pub fitness:            f64,
    pub metrics:            crate::metrics::Metrics,
    pub primary_interval:   String,
    pub secondary_interval: String,
    pub asset:              String,
}

/// Evaluate `genome` on a random window from `pool` for the given `asset`.
///
/// Steps:
/// 1. Look up primary and secondary candle series from the pool.
/// 2. Validate minimum length requirements.
/// 3. Pick a random time window.
/// 4. Run a bar-by-bar paper-trading simulation.
/// 5. Compute metrics and a fitness score.
pub fn evaluate(
    genome: &DualMacdGenome,
    pool:   &CandlePool,
    asset:  &str,
    cfg:    &EvalConfig,
    rng:    &mut impl Rng,
) -> EvalResult {
    let bad = |msg: &str| {
        log::debug!("evaluate: {asset} — {msg}");
        EvalResult {
            fitness:            -1000.0,
            metrics:            crate::metrics::Metrics::default(),
            primary_interval:   genome.primary_interval.clone(),
            secondary_interval: genome.secondary_interval.clone(),
            asset:              asset.to_string(),
        }
    };

    // ── 1. Fetch candle series ────────────────────────────────────────────────
    let primary_key   = (asset.to_string(), genome.primary_interval.clone());
    let secondary_key = (asset.to_string(), genome.secondary_interval.clone());

    let primary_candles = match pool.get(&primary_key) {
        Some(c) => c,
        None    => return bad("no primary candles"),
    };
    let secondary_candles = match pool.get(&secondary_key) {
        Some(c) => c,
        None    => return bad("no secondary candles"),
    };

    // ── 2. Validate lengths ───────────────────────────────────────────────────
    let strategy = genome.to_strategy();
    let required  = strategy.required_history();
    let min_len   = required + cfg.min_window_candles;

    if primary_candles.len() < min_len || secondary_candles.len() < min_len {
        return bad("not enough candles");
    }

    // ── 3. Random window ──────────────────────────────────────────────────────
    let extra      = rng.gen_range(0..=cfg.extra_random_candles);
    let window_len = (min_len + extra)
        .min(primary_candles.len())
        .min(secondary_candles.len());

    let start_primary   = rng.gen_range(0..=primary_candles.len() - window_len);
    let start_secondary = rng.gen_range(0..=secondary_candles.len() - window_len);
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

    let mut equity_curve: Vec<(chrono::DateTime<chrono::Utc>, i64)> =
        Vec::with_capacity(window.len());

    let primary_window   = &primary_candles[start_primary..start_primary + window_len];
    let secondary_window = &secondary_candles[start_secondary..start_secondary + window_len];

    // ── 4. Bar-by-bar simulation ──────────────────────────────────────────────
    let mut engine = PaperTradingEngine::new(cfg.trading_cfg.clone());

    for i in required..window_len {
        // Reverse slices so index 0 = newest (strategy convention).
        let primary_slice: Vec<Candle>   = primary_window[0..=i].iter().rev().cloned().collect();
        let secondary_slice: Vec<Candle> = secondary_window[0..=i].iter().rev().cloned().collect();
        let candle = &window[t];
        let pt_signal = paper_trading::Signal::from(strategy.signal(&slice));
        engine.execute(&pt_signal, asset, candle);

        let pos_value: i64 = engine.positions.values()
            .map(|p| p.quantity * candle.close)
            .sum();
        equity_curve.push((window[t].timestamp, engine.cash + pos_value));
        equity_curve.push(engine.cash_cents + pos_value);
    }

        let sig = strategy.signal(&primary_slice, &secondary_slice);
        engine.execute(&sig, asset, &primary_window[i]);
        engine.snapshot_equity(asset, primary_window[i].close, primary_window[i].timestamp);
    }

    // ── 5. Metrics & fitness ─────────────────────────────────────────────────
    let trade_records = from_engine_trades(&engine.trades);
    let metrics = compute_metrics(
        &engine.equity_curve,
        &trade_records,
        cfg.trading_cfg.starting_capital_cents,
    );
    let fitness = score(&metrics, &cfg.fitness_weights);

    EvalResult {
        fitness,
        metrics,
        primary_interval:   genome.primary_interval.clone(),
        secondary_interval: genome.secondary_interval.clone(),
        asset: asset.to_string(),
    let trade_records = metrics::from_paper_trades(&engine.trades);
    let metrics = metrics::compute(&equity_curve, &trade_records, paper_cfg.starting_capital);
    let fitness = fitness::score(&metrics, fitness_cfg);

    EvalResult { fitness, metrics, asset: asset.to_string(), interval: interval.to_string() }
}

fn bad_result(asset: &str, interval: &str, _capital: i64) -> EvalResult {
    EvalResult {
        fitness:  f64::NEG_INFINITY,
        metrics:  Metrics::default(),
        asset:    asset.to_string(),
        interval: interval.to_string(),
    }
}
