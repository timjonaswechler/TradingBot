//! Typed strategy-declared configuration for runtime strategy handling.

use crate::market_input::SecondaryTimeframeConfig;

/// Typed strategy-declared configuration returned by `strategy_config()`.
///
/// Strategy Configuration may declare only strategy requirements/defaults such as
/// minimum warmup and Secondary-Timeframe requirements. Run Configuration remains
/// authoritative for runtime asset, Primary Timeframe, and conflicts.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyConfiguration {
    minimum_warmup: usize,
    secondary_timeframes: Vec<SecondaryTimeframeConfig>,
}

impl StrategyConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn minimum_warmup(&self) -> usize {
        self.minimum_warmup
    }

    pub fn secondary_timeframes(&self) -> &[SecondaryTimeframeConfig] {
        &self.secondary_timeframes
    }

    pub(crate) fn with_minimum_warmup(mut self, minimum_warmup: usize) -> Self {
        self.minimum_warmup = minimum_warmup;
        self
    }

    pub(crate) fn with_secondary(mut self, secondary: SecondaryTimeframeConfig) -> Self {
        self.secondary_timeframes.push(secondary);
        self
    }
}
