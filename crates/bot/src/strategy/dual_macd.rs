// STUB — replace when merging with the real DualMacd implementation.
use super::{DualStrategy, Signal};
use crate::market_data::Candle;

/// All 24 tunable parameters of the DualMacd strategy.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DualMacdParams {
    // MACD core
    pub fast:   usize,
    pub slow:   usize,
    pub signal: usize,

    // Primary timeframe weights
    pub primary_crossover_weight: f64,
    pub primary_histogram_weight: f64,
    pub primary_slope_weight:     f64,
    pub primary_slope_lookback:   usize,

    // Secondary timeframe
    pub secondary_drop_threshold: f64,
    pub secondary_drop_weight:    f64,
    pub secondary_slope_lookback: usize,

    // Calendar effects
    pub month_start_boost:   f64,
    pub month_start_days:    usize,
    pub month_end_caution:   f64,
    pub month_end_days:      usize,
    pub quarter_end_caution: f64,
    pub year_end_boost:      f64,

    // Trend / volatility filters
    pub crash_atr_multiplier: f64,
    pub bull_trend_threshold: f64,
    pub bear_trend_threshold: f64,
    pub long_ema_period:      usize,
    pub atr_period:           usize,
    pub atr_median_period:    usize,

    // Decision thresholds
    pub buy_threshold:   f64,
    pub sell_threshold:  f64,
    pub short_threshold: f64,
}

impl Default for DualMacdParams {
    fn default() -> Self {
        Self {
            fast:   12,
            slow:   26,
            signal: 9,
            primary_crossover_weight: 1.0,
            primary_histogram_weight: 1.0,
            primary_slope_weight:     1.0,
            primary_slope_lookback:   3,
            secondary_drop_threshold: 0.01,
            secondary_drop_weight:    1.0,
            secondary_slope_lookback: 2,
            month_start_boost:        0.2,
            month_start_days:         2,
            month_end_caution:        -0.2,
            month_end_days:           2,
            quarter_end_caution:      -0.1,
            year_end_boost:           0.1,
            crash_atr_multiplier:     3.0,
            bull_trend_threshold:     0.0005,
            bear_trend_threshold:     -0.0005,
            long_ema_period:          150,
            atr_period:               14,
            atr_median_period:        100,
            buy_threshold:            1.0,
            sell_threshold:           -1.0,
            short_threshold:          -2.5,
        }
    }
}

/// STUB — always returns `Hold`.
pub struct DualMacdStrategy {
    pub params: DualMacdParams,
}

impl DualStrategy for DualMacdStrategy {
    fn name(&self) -> &str { "dual_macd" }
    fn required_history(&self) -> usize { 300 }
    fn signal(&self, _primary: &[Candle], _secondary: &[Candle]) -> Signal {
        Signal::Hold
    }
}
