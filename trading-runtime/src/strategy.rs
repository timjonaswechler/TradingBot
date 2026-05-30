//! Strategy tick boundary abstractions for the trading runtime.

use crate::{
    secondary_context::secondary_context_unavailable_reason, MarketState, RuntimePortfolioSnapshot,
    SecondaryTimeframeConfig, StrategyDecision,
};
use shared::{Candle, Timeframe};
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

/// Strategy-facing read-only view over runtime-owned Market State for one tick.
#[derive(Debug, Clone, Copy)]
pub struct MarketView<'a> {
    market_state: &'a MarketState,
    primary_timeframe: Timeframe,
    secondary_timeframes: &'a [SecondaryTimeframeConfig],
    primary_candle: &'a Candle,
}

impl<'a> MarketView<'a> {
    pub(crate) fn new(
        market_state: &'a MarketState,
        primary_timeframe: Timeframe,
        secondary_timeframes: &'a [SecondaryTimeframeConfig],
        primary_candle: &'a Candle,
    ) -> Self {
        Self {
            market_state,
            primary_timeframe,
            secondary_timeframes,
            primary_candle,
        }
    }

    /// Configured Primary Timeframe for this Strategy Tick.
    pub fn primary_timeframe(&self) -> Timeframe {
        self.primary_timeframe
    }

    /// The current Primary Strategy Tick candle.
    pub fn primary_candle(&self) -> &'a Candle {
        self.primary_candle
    }

    /// All timeframes configured for this Market View.
    pub fn configured_timeframes(&self) -> impl Iterator<Item = Timeframe> + '_ {
        std::iter::once(self.primary_timeframe).chain(
            self.secondary_timeframes
                .iter()
                .map(|secondary| secondary.timeframe),
        )
    }

    /// Chronological Primary Timeframe history visible to this Strategy Tick.
    pub fn primary_history(&self) -> &'a [Candle] {
        self.market_state
            .history(self.primary_timeframe)
            .expect("primary timeframe history should be configured")
    }

    /// Visible candle history for a configured timeframe.
    ///
    /// Primary history is always visible. Secondary history is visible only when
    /// the configured Secondary context is currently available/fresh for this
    /// Strategy Tick. Unavailable optional Secondary context returns `Ok(None)`;
    /// unconfigured timeframe access returns a strategy-facing error.
    pub fn visible_history(
        &self,
        timeframe: Timeframe,
    ) -> Result<Option<&'a [Candle]>, MarketViewTimeframeError> {
        if timeframe == self.primary_timeframe {
            return Ok(Some(self.primary_history()));
        }

        let secondary = self
            .secondary_timeframes
            .iter()
            .find(|secondary| secondary.timeframe == timeframe)
            .ok_or(MarketViewTimeframeError::UnconfiguredTimeframe { timeframe })?;

        if secondary_context_unavailable_reason(self.market_state, self.primary_candle, secondary)
            .is_some()
        {
            Ok(None)
        } else {
            Ok(self.market_state.history(timeframe))
        }
    }

    /// Latest candle currently visible for a configured timeframe.
    pub fn visible_latest_candle(
        &self,
        timeframe: Timeframe,
    ) -> Result<Option<&'a Candle>, MarketViewTimeframeError> {
        Ok(self
            .visible_history(timeframe)?
            .and_then(|history| history.last()))
    }

    /// Latest candle currently held by Market State for a configured timeframe.
    pub fn latest_candle(&self, timeframe: Timeframe) -> Option<&'a Candle> {
        self.market_state
            .history(timeframe)
            .and_then(|history| history.last())
    }
}

/// Strategy-facing Market View timeframe access error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MarketViewTimeframeError {
    UnconfiguredTimeframe { timeframe: Timeframe },
}

impl std::fmt::Display for MarketViewTimeframeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnconfiguredTimeframe { timeframe } => {
                write!(formatter, "unconfigured timeframe `{timeframe}`")
            }
        }
    }
}

impl std::error::Error for MarketViewTimeframeError {}

/// Primitive value stored in session-local Strategy State.
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyStateValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
}

/// Session-local Strategy State handle.
///
/// Strategy State is runtime-owned memory for one running strategy. Clones share
/// the same underlying session-local storage so strategy-facing adapters can pass
/// lightweight handles into script runtimes without losing mutations.
#[derive(Debug, Clone, Default)]
pub struct StrategyState {
    values: Arc<RwLock<HashMap<String, StrategyStateValue>>>,
}

impl StrategyState {
    pub fn get(&self, key: &str) -> Option<StrategyStateValue> {
        self.values
            .read()
            .expect("strategy state lock should not be poisoned")
            .get(key)
            .cloned()
    }

    pub fn set(&self, key: impl Into<String>, value: StrategyStateValue) {
        self.values
            .write()
            .expect("strategy state lock should not be poisoned")
            .insert(key.into(), value);
    }

    pub fn is_empty(&self) -> bool {
        self.values
            .read()
            .expect("strategy state lock should not be poisoned")
            .is_empty()
    }
}

impl PartialEq for StrategyState {
    fn eq(&self, other: &Self) -> bool {
        *self
            .values
            .read()
            .expect("strategy state lock should not be poisoned")
            == *other
                .values
                .read()
                .expect("strategy state lock should not be poisoned")
    }
}

/// Grouped strategy-facing runtime context for one Strategy Tick.
pub struct StrategyContext<'a> {
    pub portfolio: &'a RuntimePortfolioSnapshot,
    pub state: &'a mut StrategyState,
}

/// Runtime-owned input passed to Strategy Handling for one Strategy Tick.
pub struct StrategyTickInput<'a> {
    pub market: MarketView<'a>,
    pub context: StrategyContext<'a>,
    pub primary_candle: &'a Candle,
}

/// Tick-time error returned by Strategy Handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StrategyError {
    pub message: String,
}

impl StrategyError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// Result of evaluating a Strategy Tick.
#[derive(Debug, Clone, PartialEq)]
pub enum StrategyTickResult {
    Decision(StrategyDecision),
    Error(StrategyError),
}

impl From<StrategyDecision> for StrategyTickResult {
    fn from(decision: StrategyDecision) -> Self {
        Self::Decision(decision)
    }
}

/// Strategy Tick evaluator used by the runtime.
pub trait StrategyHandler {
    /// Notify Strategy Handling that market input has been accepted into Runtime Market State.
    ///
    /// Rhai-backed handlers use this to update runtime-owned Compute State such as
    /// anchored/structure-aware outputs. Test handlers can keep the default no-op.
    fn on_market_input_accepted(
        &mut self,
        _market_state: &MarketState,
        _candle: &Candle,
        _primary_timeframe: Timeframe,
    ) {
    }

    fn on_tick(&mut self, input: StrategyTickInput<'_>) -> StrategyTickResult;
}

/// Test-oriented strategy handler that returns predetermined decisions.
#[derive(Debug, Clone)]
pub struct PredeterminedStrategyHandler {
    decisions: VecDeque<StrategyDecision>,
}

impl PredeterminedStrategyHandler {
    pub fn from_decisions(decisions: impl IntoIterator<Item = StrategyDecision>) -> Self {
        Self {
            decisions: decisions.into_iter().collect(),
        }
    }
}

impl StrategyHandler for PredeterminedStrategyHandler {
    fn on_tick(&mut self, _input: StrategyTickInput<'_>) -> StrategyTickResult {
        StrategyTickResult::Decision(
            self.decisions
                .pop_front()
                .unwrap_or_else(StrategyDecision::hold),
        )
    }
}
