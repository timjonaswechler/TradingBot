use rand::Rng;

use crate::strategy::Strategy;
use crate::strategy::macd_enhanced::{MacdEnhanced, MacdEnhancedParams};

/// Trait den jedes Strategie-Genom implementieren muss.
pub trait Genome: Clone + Send + Sync + std::fmt::Debug {
    /// Eindeutiger Strategiename (für Logging und Dateinamen).
    fn strategy_name(&self) -> &'static str;

    /// Erzeugt eine zufällige Variante (feste Kernparameter bleiben erhalten).
    fn random_like(&self, rng: &mut impl Rng) -> Self;

    /// Gibt eine leicht mutierte Kopie zurück.
    /// `magnitude` ∈ [0.0, 1.0]: 0 = unverändert, 1 = vollständig zufällig.
    fn mutate(&self, magnitude: f64, rng: &mut impl Rng) -> Self;

    /// Konvertiert das Genom in eine ausführbare Strategie.
    fn to_strategy(&self) -> Box<dyn Strategy>;

    /// Serialisiert die tunable Parameter als TOML-String.
    fn to_toml(&self) -> String;
}

// ── MacdEnhancedGenome ────────────────────────────────────────────────────────

/// Wrapper um MacdEnhancedParams der das Genome-Trait implementiert.
/// fast/slow/signal_period sind fest und werden bei Mutation nicht verändert.
#[derive(Clone, Debug)]
pub struct MacdEnhancedGenome(pub MacdEnhancedParams);

impl MacdEnhancedGenome {
    /// Lädt die beiden Gewinner aus einer vorherigen Optimierung.
    /// Gibt `None` zurück wenn die Datei nicht existiert oder nicht parsebar ist.
    pub fn load_prev_winners(path: &str) -> Option<(Self, Self)> {
        let content = std::fs::read_to_string(path).ok()?;
        let doc: toml::Value = content.parse().ok()?;

        let a = doc.get("winner_a")?.as_table()?;
        let b = doc.get("winner_b")?.as_table()?;

        let params_a: MacdEnhancedParams = toml::Value::Table(a.clone()).try_into().ok()?;
        let params_b: MacdEnhancedParams = toml::Value::Table(b.clone()).try_into().ok()?;

        Some((Self(params_a), Self(params_b)))
    }

    /// Erstellt ein Genom mit zufälligen tunable Parametern.
    /// fast/slow/signal bleiben aus `base` erhalten.
    pub fn new_random(base: &MacdEnhancedParams, rng: &mut impl Rng) -> Self {
        Self(MacdEnhancedParams {
            fast_period:   base.fast_period,
            slow_period:   base.slow_period,
            signal_period: base.signal_period,
            crossover_weight:           rng.gen_range(0.0_f64..3.0),
            zero_line_weight:           rng.gen_range(0.0_f64..3.0),
            zero_line_deadband:         rng.gen_range(0.0_f64..0.01),
            histogram_strength_weight:  rng.gen_range(0.0_f64..3.0),
            histogram_min_threshold:    rng.gen_range(0.0_f64..0.005),
            histogram_momentum_weight:  rng.gen_range(0.0_f64..3.0),
            histogram_lookback:         rng.gen_range(1_usize..=5),
            histogram_reversal_weight:  rng.gen_range(0.0_f64..3.0),
            reversal_confirm_bars:      rng.gen_range(1_usize..=3),
            macd_slope_weight:          rng.gen_range(0.0_f64..3.0),
            slope_lookback:             rng.gen_range(1_usize..=10),
            ema_fast_slope_weight:      rng.gen_range(0.0_f64..3.0),
            ema_slow_slope_weight:      rng.gen_range(0.0_f64..3.0),
            trend_filter_weight:        rng.gen_range(0.0_f64..3.0),
            trend_filter_min_slope:     rng.gen_range(0.0_f64..0.005),
            ema_separation_weight:      rng.gen_range(0.0_f64..3.0),
            regime_period:              rng.gen_range(20_usize..=200),
            regime_weight:              rng.gen_range(0.0_f64..5.0),
            regime_deadband:            rng.gen_range(0.0_f64..0.05),
            exit_fast_period:           rng.gen_range(2_usize..=10),
            exit_slow_period:           rng.gen_range(5_usize..=20),
            exit_signal_period:         rng.gen_range(2_usize..=6),
            exit_weight:                rng.gen_range(0.0_f64..5.0),
            buy_threshold:              rng.gen_range(0.1_f64..5.0),
            sell_threshold:             rng.gen_range(0.1_f64..5.0),
        })
    }
}

impl Genome for MacdEnhancedGenome {
    fn strategy_name(&self) -> &'static str { "macd_enhanced" }

    fn random_like(&self, rng: &mut impl Rng) -> Self {
        Self::new_random(&self.0, rng)
    }

    fn mutate(&self, magnitude: f64, rng: &mut impl Rng) -> Self {
        let p = &self.0;
        Self(MacdEnhancedParams {
            // Fest — unveränderlich
            fast_period:   p.fast_period,
            slow_period:   p.slow_period,
            signal_period: p.signal_period,
            // Tunable — werden mutiert
            crossover_weight:           mf(p.crossover_weight,          magnitude, 0.0, 3.0,   rng),
            zero_line_weight:           mf(p.zero_line_weight,           magnitude, 0.0, 3.0,   rng),
            zero_line_deadband:         mf(p.zero_line_deadband,         magnitude, 0.0, 0.01,  rng),
            histogram_strength_weight:  mf(p.histogram_strength_weight,  magnitude, 0.0, 3.0,   rng),
            histogram_min_threshold:    mf(p.histogram_min_threshold,    magnitude, 0.0, 0.005, rng),
            histogram_momentum_weight:  mf(p.histogram_momentum_weight,  magnitude, 0.0, 3.0,   rng),
            histogram_lookback:         mi(p.histogram_lookback,         magnitude, 1,   5,     rng),
            histogram_reversal_weight:  mf(p.histogram_reversal_weight,  magnitude, 0.0, 3.0,   rng),
            reversal_confirm_bars:      mi(p.reversal_confirm_bars,      magnitude, 1,   3,     rng),
            macd_slope_weight:          mf(p.macd_slope_weight,          magnitude, 0.0, 3.0,   rng),
            slope_lookback:             mi(p.slope_lookback,             magnitude, 1,   10,    rng),
            ema_fast_slope_weight:      mf(p.ema_fast_slope_weight,      magnitude, 0.0, 3.0,   rng),
            ema_slow_slope_weight:      mf(p.ema_slow_slope_weight,      magnitude, 0.0, 3.0,   rng),
            trend_filter_weight:        mf(p.trend_filter_weight,        magnitude, 0.0, 3.0,   rng),
            trend_filter_min_slope:     mf(p.trend_filter_min_slope,     magnitude, 0.0, 0.005, rng),
            ema_separation_weight:      mf(p.ema_separation_weight,      magnitude, 0.0, 3.0,    rng),
            regime_period:              mi(p.regime_period,              magnitude, 20,  200,    rng),
            regime_weight:              mf(p.regime_weight,              magnitude, 0.0, 5.0,    rng),
            regime_deadband:            mf(p.regime_deadband,            magnitude, 0.0, 0.05,   rng),
            exit_fast_period:           mi(p.exit_fast_period,           magnitude, 2,   10,     rng),
            exit_slow_period:           mi(p.exit_slow_period,           magnitude, 5,   20,     rng),
            exit_signal_period:         mi(p.exit_signal_period,         magnitude, 2,   6,      rng),
            exit_weight:                mf(p.exit_weight,                magnitude, 0.0, 5.0,    rng),
            buy_threshold:              mf(p.buy_threshold,              magnitude, 0.1, 5.0,    rng),
            sell_threshold:             mf(p.sell_threshold,             magnitude, 0.1, 5.0,    rng),
        })
    }

    fn to_strategy(&self) -> Box<dyn Strategy> {
        Box::new(MacdEnhanced::new(self.0.clone()))
    }

    fn to_toml(&self) -> String {
        let p = &self.0;
        format!(
            "# Feste Parameter (nicht vom Optimizer geändert)\n\
             fast_period   = {}\n\
             slow_period   = {}\n\
             signal_period = {}\n\n\
             # K1: Kreuzung\n\
             crossover_weight          = {:.4}\n\n\
             # K2: Nulllinien-Lage\n\
             zero_line_weight          = {:.4}\n\
             zero_line_deadband        = {:.6}\n\n\
             # K3: Histogramm-Stärke\n\
             histogram_strength_weight = {:.4}\n\
             histogram_min_threshold   = {:.6}\n\n\
             # K4: Histogramm-Momentum\n\
             histogram_momentum_weight = {:.4}\n\
             histogram_lookback        = {}\n\n\
             # K5: Histogramm-Wendepunkt\n\
             histogram_reversal_weight = {:.4}\n\
             reversal_confirm_bars     = {}\n\n\
             # K6: MACD-Steigung\n\
             macd_slope_weight         = {:.4}\n\
             slope_lookback            = {}\n\n\
             # K7: EMA-Fast Steigung\n\
             ema_fast_slope_weight     = {:.4}\n\n\
             # K8: EMA-Slow Steigung\n\
             ema_slow_slope_weight     = {:.4}\n\n\
             # K9: Gleichlauf-Filter\n\
             trend_filter_weight       = {:.4}\n\
             trend_filter_min_slope    = {:.6}\n\n\
             # K10: EMA-Abstand\n\
             ema_separation_weight     = {:.4}\n\n\
             # K11: Trend-Regime (langer EMA)\n\
             regime_period             = {}\n\
             regime_weight             = {:.4}\n\
             regime_deadband           = {:.6}\n\n\
             # Exit-MACD (schneller MACD für frühzeitigen Exit)\n\
             exit_fast_period          = {}\n\
             exit_slow_period          = {}\n\
             exit_signal_period        = {}\n\
             exit_weight               = {:.4}\n\n\
             # Entscheidungs-Schwellen\n\
             buy_threshold             = {:.4}\n\
             sell_threshold            = {:.4}\n",
            p.fast_period, p.slow_period, p.signal_period,
            p.crossover_weight,
            p.zero_line_weight, p.zero_line_deadband,
            p.histogram_strength_weight, p.histogram_min_threshold,
            p.histogram_momentum_weight, p.histogram_lookback,
            p.histogram_reversal_weight, p.reversal_confirm_bars,
            p.macd_slope_weight, p.slope_lookback,
            p.ema_fast_slope_weight,
            p.ema_slow_slope_weight,
            p.trend_filter_weight, p.trend_filter_min_slope,
            p.ema_separation_weight,
            p.regime_period, p.regime_weight, p.regime_deadband,
            p.exit_fast_period, p.exit_slow_period, p.exit_signal_period, p.exit_weight,
            p.buy_threshold, p.sell_threshold,
        )
    }
}

// ── RsiGenome (Stub für spätere Implementierung) ──────────────────────────────

/// Platzhalter — wird implementiert wenn RSI-Optimierung benötigt wird.
/// Struktur analog zu MacdEnhancedGenome.
#[derive(Clone, Debug)]
pub struct RsiGenome {
    pub period:               usize,
    pub oversold_threshold:   f64,
    pub overbought_threshold: f64,
    pub slope_weight:         f64,
    pub slope_lookback:       usize,
    pub zone_filter_weight:   f64,
    pub buy_threshold:        f64,
    pub sell_threshold:       f64,
}

impl RsiGenome {
    pub fn new_random(period: usize, rng: &mut impl Rng) -> Self {
        Self {
            period,
            oversold_threshold:   rng.gen_range(15.0_f64..40.0),
            overbought_threshold: rng.gen_range(60.0_f64..85.0),
            slope_weight:         rng.gen_range(0.0_f64..2.0),
            slope_lookback:       rng.gen_range(1_usize..=8),
            zone_filter_weight:   rng.gen_range(0.0_f64..2.0),
            buy_threshold:        rng.gen_range(0.1_f64..3.0),
            sell_threshold:       rng.gen_range(0.1_f64..3.0),
        }
    }
}

impl Genome for RsiGenome {
    fn strategy_name(&self) -> &'static str { "rsi" }

    fn random_like(&self, rng: &mut impl Rng) -> Self {
        Self::new_random(self.period, rng)
    }

    fn mutate(&self, magnitude: f64, rng: &mut impl Rng) -> Self {
        Self {
            period:               self.period, // fest
            oversold_threshold:   mf(self.oversold_threshold,   magnitude, 15.0, 45.0, rng),
            overbought_threshold: mf(self.overbought_threshold,  magnitude, 55.0, 85.0, rng),
            slope_weight:         mf(self.slope_weight,          magnitude,  0.0,  2.0, rng),
            slope_lookback:       mi(self.slope_lookback,        magnitude,    1,    8, rng),
            zone_filter_weight:   mf(self.zone_filter_weight,    magnitude,  0.0,  2.0, rng),
            buy_threshold:        mf(self.buy_threshold,         magnitude,  0.1,  3.0, rng),
            sell_threshold:       mf(self.sell_threshold,        magnitude,  0.1,  3.0, rng),
        }
    }

    fn to_strategy(&self) -> Box<dyn Strategy> {
        // RSI Enhanced noch nicht implementiert — fällt auf Standard-RSI zurück
        Box::new(crate::strategy::rsi::Rsi {
            period:     self.period,
            oversold:   self.oversold_threshold,
            overbought: self.overbought_threshold,
        })
    }

    fn to_toml(&self) -> String {
        format!(
            "period               = {}\n\
             oversold_threshold   = {:.2}\n\
             overbought_threshold = {:.2}\n\
             slope_weight         = {:.4}\n\
             slope_lookback       = {}\n\
             zone_filter_weight   = {:.4}\n\
             buy_threshold        = {:.4}\n\
             sell_threshold       = {:.4}\n",
            self.period,
            self.oversold_threshold, self.overbought_threshold,
            self.slope_weight, self.slope_lookback,
            self.zone_filter_weight,
            self.buy_threshold, self.sell_threshold,
        )
    }
}

// ── Mutations-Hilfsfunktionen ─────────────────────────────────────────────────

/// Mutiert einen f64-Wert: addiert Normalverteilungs-Rauschen proportional
/// zur Parameterbreite (max−min), clamped auf [min, max].
fn mf(val: f64, magnitude: f64, min: f64, max: f64, rng: &mut impl Rng) -> f64 {
    let range = max - min;
    let sigma = magnitude * range * 0.3; // 30% der Range bei magnitude=1
    let delta: f64 = rng.gen::<f64>() * 2.0 * sigma - sigma; // uniform [-sigma, +sigma]
    (val + delta).clamp(min, max)
}

/// Mutiert einen usize-Wert: mit Wahrscheinlichkeit `magnitude` ±1.
fn mi(val: usize, magnitude: f64, min: usize, max: usize, rng: &mut impl Rng) -> usize {
    if rng.gen::<f64>() < magnitude {
        let delta: i32 = if rng.gen::<bool>() { 1 } else { -1 };
        ((val as i32 + delta).max(min as i32) as usize).min(max)
    } else {
        val
    }
}
