use rand::Rng;

use crate::strategy::dual_macd::{DualMacdParams, DualMacdStrategy};

/// Allowed candle intervals for primary / secondary timeframes.
const ALLOWED_INTERVALS: &[&str] = &["1d", "1wk", "1h", "30m", "15m"];

/// Genome encoding for the DualMacd strategy.
/// Every field is tunable by the genetic algorithm.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DualMacdGenome {
    pub params:             DualMacdParams,
    pub primary_interval:   String, // e.g. "1d"
    pub secondary_interval: String, // e.g. "1h"
}

impl DualMacdGenome {
    /// Create a genome with default params and sensible default intervals.
    pub fn default_genome() -> Self {
        Self {
            params:             DualMacdParams::default(),
            primary_interval:   "1d".to_string(),
            secondary_interval: "1h".to_string(),
        }
    }

    /// Create a genome with fully randomised params.
    pub fn random(rng: &mut impl Rng) -> Self {
        let params = DualMacdParams {
            fast:                    rng.gen_range(5_usize..=19),
            slow:                    rng.gen_range(15_usize..=49),
            signal:                  rng.gen_range(5_usize..=14),
            primary_crossover_weight:  rng.gen_range(0.0_f64..3.0),
            primary_histogram_weight:  rng.gen_range(0.0_f64..3.0),
            primary_slope_weight:      rng.gen_range(0.0_f64..3.0),
            primary_slope_lookback:    rng.gen_range(1_usize..=9),
            secondary_drop_threshold:  rng.gen_range(0.0005_f64..0.05),
            secondary_drop_weight:     rng.gen_range(0.0_f64..3.0),
            secondary_slope_lookback:  rng.gen_range(1_usize..=4),
            month_start_boost:         rng.gen_range(0.0_f64..1.0),
            month_start_days:          rng.gen_range(1_usize..=4),
            month_end_caution:         rng.gen_range(-1.0_f64..0.0),
            month_end_days:            rng.gen_range(1_usize..=4),
            quarter_end_caution:       rng.gen_range(-0.5_f64..0.0),
            year_end_boost:            rng.gen_range(0.0_f64..0.5),
            crash_atr_multiplier:      rng.gen_range(1.5_f64..5.0),
            bull_trend_threshold:      rng.gen_range(0.0001_f64..0.001),
            bear_trend_threshold:      rng.gen_range(-0.001_f64..-0.0001),
            long_ema_period:           rng.gen_range(100_usize..=199),
            atr_period:                rng.gen_range(5_usize..=29),
            atr_median_period:         rng.gen_range(50_usize..=199),
            buy_threshold:             rng.gen_range(0.0_f64..5.0),
            sell_threshold:            rng.gen_range(-3.0_f64..0.0),
            short_threshold:           rng.gen_range(-5.0_f64..-0.5),
        };
        let primary_interval = random_interval(rng).to_string();
        let secondary_interval = random_interval(rng).to_string();
        Self { params, primary_interval, secondary_interval }
    }

    /// Return a mutated copy of this genome.
    ///
    /// `magnitude` ∈ [0.0, 1.0]:
    /// - `f64` fields: `new = (old + magnitude * rng.gen_range(-range/2..=range/2)).clamp(min, max)`
    /// - `usize` fields: flip ±1 with probability `magnitude / 2`
    /// - `bool` fields: flip with probability `magnitude / 2`
    /// - interval strings: pick from allowed list with probability `magnitude / 3`
    pub fn mutate(&self, magnitude: f64, rng: &mut impl Rng) -> Self {
        let p = &self.params;
        let params = DualMacdParams {
            fast:  mu(p.fast,  magnitude, 5,   20,  rng),
            slow:  mu(p.slow,  magnitude, 15,  50,  rng),
            signal:mu(p.signal,magnitude, 5,   15,  rng),
            primary_crossover_weight:  mf(p.primary_crossover_weight,  magnitude, 0.0,    3.0,    rng),
            primary_histogram_weight:  mf(p.primary_histogram_weight,  magnitude, 0.0,    3.0,    rng),
            primary_slope_weight:      mf(p.primary_slope_weight,      magnitude, 0.0,    3.0,    rng),
            primary_slope_lookback:    mu(p.primary_slope_lookback,    magnitude, 1,      10,     rng),
            secondary_drop_threshold:  mf(p.secondary_drop_threshold,  magnitude, 0.0005, 0.05,   rng),
            secondary_drop_weight:     mf(p.secondary_drop_weight,     magnitude, 0.0,    3.0,    rng),
            secondary_slope_lookback:  mu(p.secondary_slope_lookback,  magnitude, 1,      5,      rng),
            month_start_boost:         mf(p.month_start_boost,         magnitude, 0.0,    1.0,    rng),
            month_start_days:          mu(p.month_start_days,          magnitude, 1,      5,      rng),
            month_end_caution:         mf(p.month_end_caution,         magnitude, -1.0,   0.0,    rng),
            month_end_days:            mu(p.month_end_days,            magnitude, 1,      5,      rng),
            quarter_end_caution:       mf(p.quarter_end_caution,       magnitude, -0.5,   0.0,    rng),
            year_end_boost:            mf(p.year_end_boost,            magnitude, 0.0,    0.5,    rng),
            crash_atr_multiplier:      mf(p.crash_atr_multiplier,      magnitude, 1.5,    5.0,    rng),
            bull_trend_threshold:      mf(p.bull_trend_threshold,      magnitude, 0.0001, 0.001,  rng),
            bear_trend_threshold:      mf(p.bear_trend_threshold,      magnitude, -0.001, -0.0001, rng),
            long_ema_period:           mu(p.long_ema_period,           magnitude, 100,    200,    rng),
            atr_period:                mu(p.atr_period,                magnitude, 5,      30,     rng),
            atr_median_period:         mu(p.atr_median_period,         magnitude, 50,     200,    rng),
            buy_threshold:             mf(p.buy_threshold,             magnitude, 0.0,    5.0,    rng),
            sell_threshold:            mf(p.sell_threshold,            magnitude, -3.0,   0.0,    rng),
            short_threshold:           mf(p.short_threshold,           magnitude, -5.0,   -0.5,   rng),
        };

        let primary_interval = maybe_mutate_interval(&self.primary_interval, magnitude / 3.0, rng);
        let secondary_interval = maybe_mutate_interval(&self.secondary_interval, magnitude / 3.0, rng);

        Self { params, primary_interval, secondary_interval }
    }

    /// Convert genome to an executable strategy instance.
    pub fn to_strategy(&self) -> DualMacdStrategy {
        DualMacdStrategy { params: self.params.clone() }
    }

    /// Serialize to TOML string.
    pub fn to_toml(&self) -> String {
        toml::to_string(self).unwrap_or_else(|e| format!("# serialization error: {e}"))
    }

    /// Deserialize from TOML string.
    pub fn from_toml(s: &str) -> Result<Self, Box<dyn std::error::Error>> {
        Ok(toml::from_str(s)?)
    }
}

// ─── Mutation helpers ─────────────────────────────────────────────────────────

/// Mutate an `f64` field: add `magnitude * U(-half_range, half_range)`, then clamp.
fn mf(val: f64, magnitude: f64, min: f64, max: f64, rng: &mut impl Rng) -> f64 {
    let half = (max - min) / 2.0;
    let delta: f64 = magnitude * rng.gen_range(-half..=half);
    (val + delta).clamp(min, max)
}

/// Mutate a `usize` field: ±1 with probability `magnitude / 2`.
fn mu(val: usize, magnitude: f64, min: usize, max: usize, rng: &mut impl Rng) -> usize {
    if rng.gen::<f64>() < magnitude / 2.0 {
        let delta: i64 = if rng.gen::<bool>() { 1 } else { -1 };
        ((val as i64 + delta).max(min as i64) as usize).min(max)
    } else {
        val
    }
}

/// Possibly replace the interval string with a random one from the allowed list.
fn maybe_mutate_interval(current: &str, probability: f64, rng: &mut impl Rng) -> String {
    if rng.gen::<f64>() < probability {
        random_interval(rng).to_string()
    } else {
        current.to_string()
    }
}

fn random_interval(rng: &mut impl Rng) -> &'static str {
    ALLOWED_INTERVALS[rng.gen_range(0..ALLOWED_INTERVALS.len())]
}

// ─── Unit tests ───────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn seeded() -> StdRng { StdRng::seed_from_u64(42) }

    #[test]
    fn random_params_in_range() {
        let mut rng = seeded();
        let g = DualMacdGenome::random(&mut rng);
        let p = &g.params;

        assert!((5..=20).contains(&p.fast),          "fast={}", p.fast);
        assert!((15..=50).contains(&p.slow),         "slow={}", p.slow);
        assert!((5..=15).contains(&p.signal),        "signal={}", p.signal);
        assert!((0.0..=3.0).contains(&p.primary_crossover_weight));
        assert!((0.0..=3.0).contains(&p.primary_histogram_weight));
        assert!((0.0..=3.0).contains(&p.primary_slope_weight));
        assert!((1..=10).contains(&p.primary_slope_lookback));
        assert!(p.secondary_drop_threshold >= 0.0005 && p.secondary_drop_threshold <= 0.05);
        assert!((0.0..=3.0).contains(&p.secondary_drop_weight));
        assert!((1..=5).contains(&p.secondary_slope_lookback));
        assert!((0.0..=1.0).contains(&p.month_start_boost));
        assert!((1..=5).contains(&p.month_start_days));
        assert!(p.month_end_caution >= -1.0 && p.month_end_caution <= 0.0);
        assert!((1..=5).contains(&p.month_end_days));
        assert!(p.quarter_end_caution >= -0.5 && p.quarter_end_caution <= 0.0);
        assert!((0.0..=0.5).contains(&p.year_end_boost));
        assert!(p.crash_atr_multiplier >= 1.5 && p.crash_atr_multiplier <= 5.0);
        assert!(p.bull_trend_threshold >= 0.0001 && p.bull_trend_threshold <= 0.001);
        assert!(p.bear_trend_threshold >= -0.001 && p.bear_trend_threshold <= -0.0001);
        assert!((100..=200).contains(&p.long_ema_period));
        assert!((5..=30).contains(&p.atr_period));
        assert!((50..=200).contains(&p.atr_median_period));
        assert!((0.0..=5.0).contains(&p.buy_threshold));
        assert!(p.sell_threshold >= -3.0 && p.sell_threshold <= 0.0);
        assert!(p.short_threshold >= -5.0 && p.short_threshold <= -0.5);

        assert!(ALLOWED_INTERVALS.contains(&g.primary_interval.as_str()));
        assert!(ALLOWED_INTERVALS.contains(&g.secondary_interval.as_str()));
    }

    #[test]
    fn mutate_zero_magnitude_params_unchanged() {
        let mut rng = seeded();
        let g = DualMacdGenome::random(&mut rng);
        let mutated = g.mutate(0.0, &mut rng);
        // With magnitude=0 no mutations should fire; values stay identical.
        let p = &g.params;
        let m = &mutated.params;
        assert_eq!(p.fast,  m.fast);
        assert_eq!(p.slow,  m.slow);
        assert_eq!(p.signal, m.signal);
        // f64 fields: delta = 0 * anything = 0
        assert_eq!(p.primary_crossover_weight, m.primary_crossover_weight);
        assert_eq!(p.buy_threshold,            m.buy_threshold);
        assert_eq!(g.primary_interval,         mutated.primary_interval);
        assert_eq!(g.secondary_interval,       mutated.secondary_interval);
    }

    #[test]
    fn mutate_high_magnitude_changes_params() {
        // With magnitude=1.0 and many iterations at least one param must change.
        let mut rng = seeded();
        let g = DualMacdGenome::random(&mut rng);
        let mut changed = false;
        for _ in 0..20 {
            let mutated = g.mutate(1.0, &mut rng);
            if mutated.params.buy_threshold != g.params.buy_threshold
                || mutated.params.fast != g.params.fast
            {
                changed = true;
                break;
            }
        }
        assert!(changed, "expected at least one param to change with magnitude=1.0");
    }

    #[test]
    fn toml_roundtrip() {
        let mut rng = seeded();
        let g = DualMacdGenome::random(&mut rng);
        let s = g.to_toml();
        assert!(!s.contains("serialization error"));
        let g2 = DualMacdGenome::from_toml(&s).expect("from_toml failed");
        assert_eq!(g.params.fast,  g2.params.fast);
        assert_eq!(g.params.slow,  g2.params.slow);
        assert!((g.params.buy_threshold - g2.params.buy_threshold).abs() < 1e-9);
        assert_eq!(g.primary_interval,   g2.primary_interval);
        assert_eq!(g.secondary_interval, g2.secondary_interval);
    }
}
