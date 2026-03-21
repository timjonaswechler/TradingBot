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

    let primary_window   = &primary_candles[start_primary..start_primary + window_len];
    let secondary_window = &secondary_candles[start_secondary..start_secondary + window_len];

    // ── 4. Bar-by-bar simulation ──────────────────────────────────────────────
    let mut engine = PaperTradingEngine::new(cfg.trading_cfg.clone());

    for i in required..window_len {
        // Reverse slices so index 0 = newest (strategy convention).
        let primary_slice: Vec<Candle>   = primary_window[0..=i].iter().rev().cloned().collect();
        let secondary_slice: Vec<Candle> = secondary_window[0..=i].iter().rev().cloned().collect();

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
    }
}
