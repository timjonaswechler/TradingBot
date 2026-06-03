//! Typed strategy-declared configuration for runtime strategy handling.

use crate::market_input::SecondaryTimeframeConfig;
use domain::Timeframe;
use std::{collections::HashSet, fmt};

/// Typed strategy-declared configuration returned by `strategy_config()`.
///
/// Strategy Configuration owns the strategy timeframe contract: exactly one
/// Primary Timeframe plus any Secondary-Timeframe requirements/defaults. Run
/// Configuration remains authoritative for runtime asset, source/mode, portfolio
/// inputs, and runner policies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyConfiguration {
    primary_timeframe: Option<Timeframe>,
    primary_timeframe_declarations: usize,
    minimum_warmup: usize,
    secondary_timeframes: Vec<SecondaryTimeframeConfig>,
}

impl StrategyConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn primary_timeframe(&self) -> Option<Timeframe> {
        self.primary_timeframe
    }

    pub fn minimum_warmup(&self) -> usize {
        self.minimum_warmup
    }

    pub fn secondary_timeframes(&self) -> &[SecondaryTimeframeConfig] {
        &self.secondary_timeframes
    }

    pub fn validate_timeframe_contract(&self) -> Result<(), StrategyConfigurationError> {
        let primary_timeframe = match (self.primary_timeframe, self.primary_timeframe_declarations)
        {
            (Some(primary_timeframe), 1) => primary_timeframe,
            (_, 0) => return Err(StrategyConfigurationError::MissingPrimaryTimeframe),
            (_, count) => {
                return Err(StrategyConfigurationError::MultiplePrimaryTimeframes { count })
            }
        };

        let mut seen_secondaries = HashSet::new();
        for secondary in &self.secondary_timeframes {
            if secondary.timeframe == primary_timeframe {
                return Err(StrategyConfigurationError::SecondaryMatchesPrimary {
                    timeframe: secondary.timeframe,
                });
            }
            if !seen_secondaries.insert(secondary.timeframe) {
                return Err(StrategyConfigurationError::DuplicateSecondaryTimeframe {
                    timeframe: secondary.timeframe,
                });
            }
        }

        Ok(())
    }

    pub(crate) fn with_primary(mut self, primary_timeframe: Timeframe) -> Self {
        self.primary_timeframe = Some(primary_timeframe);
        self.primary_timeframe_declarations += 1;
        self
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StrategyConfigurationError {
    MissingPrimaryTimeframe,
    MultiplePrimaryTimeframes { count: usize },
    SecondaryMatchesPrimary { timeframe: Timeframe },
    DuplicateSecondaryTimeframe { timeframe: Timeframe },
}

impl fmt::Display for StrategyConfigurationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrimaryTimeframe => write!(
                formatter,
                "strategy_config() must declare exactly one Primary Timeframe with `.with_primary(timeframe(...))`"
            ),
            Self::MultiplePrimaryTimeframes { count } => write!(
                formatter,
                "strategy_config() must declare exactly one Primary Timeframe; found {count} `.with_primary(...)` declarations"
            ),
            Self::SecondaryMatchesPrimary { timeframe } => write!(
                formatter,
                "Secondary Timeframe '{timeframe}' must not equal the Primary Timeframe"
            ),
            Self::DuplicateSecondaryTimeframe { timeframe } => write!(
                formatter,
                "strategy_config() declares duplicate Secondary Timeframe '{timeframe}'"
            ),
        }
    }
}

impl std::error::Error for StrategyConfigurationError {}
