//! Strategy tick boundary abstractions for the trading runtime.

use crate::{MarketState, RuntimePortfolioSnapshot, StrategyDecision};
use shared::{Candle, Timeframe};
use std::collections::VecDeque;

/// Strategy-facing read-only view over runtime-owned Market State for one tick.
#[derive(Debug, Clone, Copy)]
pub struct MarketView<'a> {
    market_state: &'a MarketState,
    primary_timeframe: Timeframe,
    primary_candle: &'a Candle,
}

impl<'a> MarketView<'a> {
    pub(crate) fn new(
        market_state: &'a MarketState,
        primary_timeframe: Timeframe,
        primary_candle: &'a Candle,
    ) -> Self {
        Self {
            market_state,
            primary_timeframe,
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

    /// Latest candle currently held by Market State for a configured timeframe.
    pub fn latest_candle(&self, timeframe: Timeframe) -> Option<&'a Candle> {
        self.market_state
            .history(timeframe)
            .and_then(|history| history.last())
    }
}

/// Session-local Strategy State handle.
///
/// This issue only establishes the typed boundary. Concrete state value APIs are
/// intentionally left to later Strategy Context/State work.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyState;

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
