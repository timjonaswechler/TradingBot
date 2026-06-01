//! Runtime market-input boundary types.

use crate::strategy_config::{StrategyConfiguration, StrategyConfigurationError};
use shared::{Candle, Timeframe};

/// Whether a configured Secondary Timeframe is required for Strategy Ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryReadiness {
    Required,
    Optional,
}

/// Runtime configuration for one Secondary Timeframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecondaryTimeframeConfig {
    pub timeframe: Timeframe,
    pub readiness: SecondaryReadiness,
    pub max_missing_candles: u32,
}

impl SecondaryTimeframeConfig {
    pub fn required(timeframe: Timeframe, max_missing_candles: u32) -> Self {
        Self {
            timeframe,
            readiness: SecondaryReadiness::Required,
            max_missing_candles,
        }
    }

    pub fn optional(timeframe: Timeframe, max_missing_candles: u32) -> Self {
        Self {
            timeframe,
            readiness: SecondaryReadiness::Optional,
            max_missing_candles,
        }
    }
}

/// Runtime-owned configuration for one runtime asset's market input boundary.
///
/// Live/backtest production callers should derive this from the Runtime Asset
/// plus a validated [`StrategyConfiguration`] via [`RuntimeConfig::from_strategy_config`].
/// Direct constructors remain low-level conveniences for runtime internals,
/// tests, and fixtures that intentionally bypass Rhai strategy loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub runtime_asset: String,
    pub primary_timeframe: Timeframe,
    pub secondary_timeframes: Vec<SecondaryTimeframeConfig>,
}

impl RuntimeConfig {
    /// Build a runtime configuration from the runner-owned Runtime Asset and
    /// the strategy-owned timeframe contract.
    pub fn from_strategy_config<A>(
        runtime_asset: A,
        strategy_config: &StrategyConfiguration,
    ) -> Result<Self, StrategyConfigurationError>
    where
        A: Into<String>,
    {
        strategy_config.validate_timeframe_contract()?;
        Ok(Self::with_secondary_configs(
            runtime_asset,
            strategy_config
                .primary_timeframe()
                .expect("validated Strategy Configuration should declare Primary Timeframe"),
            strategy_config.secondary_timeframes().to_vec(),
        ))
    }

    /// Low-level constructor for explicit runtime fixtures or internals.
    ///
    /// Prefer [`RuntimeConfig::from_strategy_config`] in live/backtest code so
    /// Primary and Secondary Timeframes have a single strategy-owned source of
    /// truth.
    pub fn with_secondary_configs<A, I>(
        runtime_asset: A,
        primary_timeframe: Timeframe,
        secondary_timeframes: I,
    ) -> Self
    where
        A: Into<String>,
        I: IntoIterator<Item = SecondaryTimeframeConfig>,
    {
        Self {
            runtime_asset: runtime_asset.into(),
            primary_timeframe,
            secondary_timeframes: secondary_timeframes.into_iter().collect(),
        }
    }

    /// Low-level single-timeframe convenience constructor for tests/fixtures.
    ///
    /// Prefer [`RuntimeConfig::from_strategy_config`] in live/backtest code.
    pub fn single_timeframe(
        runtime_asset: impl Into<String>,
        primary_timeframe: Timeframe,
    ) -> Self {
        Self::with_secondary_configs(
            runtime_asset,
            primary_timeframe,
            std::iter::empty::<SecondaryTimeframeConfig>(),
        )
    }

    pub(crate) fn classify_timeframe(
        &self,
        timeframe: Timeframe,
    ) -> Option<MarketInputTimeframeRole> {
        if timeframe == self.primary_timeframe {
            Some(MarketInputTimeframeRole::Primary)
        } else if self
            .secondary_timeframes
            .iter()
            .any(|secondary| secondary.timeframe == timeframe)
        {
            Some(MarketInputTimeframeRole::Secondary)
        } else {
            None
        }
    }

    pub(crate) fn configured_timeframes(&self) -> Vec<Timeframe> {
        let mut timeframes = Vec::with_capacity(1 + self.secondary_timeframes.len());
        timeframes.push(self.primary_timeframe);
        timeframes.extend(
            self.secondary_timeframes
                .iter()
                .map(|secondary| secondary.timeframe),
        );
        timeframes
    }

    pub(crate) fn secondary_configs(&self) -> &[SecondaryTimeframeConfig] {
        &self.secondary_timeframes
    }
}

/// Market input accepted by the runtime boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum MarketInput {
    WarmupCandle(Candle),
    CompletedCandle(Candle),
}

impl MarketInput {
    pub(crate) fn candle(&self) -> &Candle {
        match self {
            Self::WarmupCandle(candle) | Self::CompletedCandle(candle) => candle,
        }
    }
}

/// Runtime-boundary errors for invalid runner/runtime market input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeInputError {
    UnknownTimeframe { timeframe: Timeframe },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarketInputTimeframeRole {
    Primary,
    Secondary,
}
