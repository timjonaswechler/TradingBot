//! Minimal strategy handler abstractions for runtime tests.

use crate::{RuntimePortfolioSnapshot, StrategyDecision};
use shared::Candle;
use std::collections::VecDeque;

/// Strategy-decision source used by the runtime.
pub trait StrategyHandler {
    fn next_decision(
        &mut self,
        candle: &Candle,
        portfolio: &RuntimePortfolioSnapshot,
    ) -> StrategyDecision;
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
    fn next_decision(
        &mut self,
        _candle: &Candle,
        _portfolio: &RuntimePortfolioSnapshot,
    ) -> StrategyDecision {
        self.decisions
            .pop_front()
            .unwrap_or_else(StrategyDecision::hold)
    }
}
