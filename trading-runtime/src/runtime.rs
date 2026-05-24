//! Trading runtime entrypoints.

use crate::{plan_execution, PortfolioState, RuntimeEvent, RuntimeStep, StrategyHandler};
use shared::Candle;

/// DB-free trading runtime core for one runtime asset.
#[derive(Debug, Clone)]
pub struct TradingRuntime<S> {
    portfolio: PortfolioState,
    primary_candle_count: usize,
    effective_warmup_bars: usize,
    strategy_handler: S,
}

impl<S: StrategyHandler> TradingRuntime<S> {
    pub fn new(
        portfolio: PortfolioState,
        effective_warmup_bars: usize,
        strategy_handler: S,
    ) -> Self {
        Self {
            portfolio,
            primary_candle_count: 0,
            effective_warmup_bars,
            strategy_handler,
        }
    }

    pub fn on_primary_candle(&mut self, candle: Candle) -> RuntimeStep {
        self.primary_candle_count += 1;

        let mut events = vec![RuntimeEvent::MarketInputAccepted {
            candle: candle.clone(),
        }];

        events.push(RuntimeEvent::TradableTickStarted {
            candle: candle.clone(),
        });

        let portfolio_before_decision = self.portfolio.snapshot(candle.close);
        let decision = self
            .strategy_handler
            .next_decision(&candle, &portfolio_before_decision);
        events.push(RuntimeEvent::StrategyDecisionProduced {
            decision: decision.clone(),
        });

        let current_side = self
            .portfolio
            .open_position
            .as_ref()
            .map(|position| position.side);
        let planned = plan_execution(&decision, current_side);
        events.push(RuntimeEvent::ExecutionActionPlanned {
            action: planned.action,
        });

        events.push(RuntimeEvent::TradableTickCompleted);

        RuntimeStep::new(events, self.portfolio.snapshot(candle.close))
    }

    pub fn effective_warmup_bars(&self) -> usize {
        self.effective_warmup_bars
    }
}
