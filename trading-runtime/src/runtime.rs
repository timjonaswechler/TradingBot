//! Trading runtime entrypoints.

use crate::{
    plan_execution, ExecutionAction, ExitKind, ForceCloseIgnoredReason, PortfolioState,
    RuntimeEvent, RuntimeStep, StrategyHandler,
};
use shared::{Candle, PositionSide};

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

        if self.primary_candle_count <= self.effective_warmup_bars {
            events.push(RuntimeEvent::WarmupAdvanced {
                current_primary_candle_count: self.primary_candle_count,
                required_warmup_candles: self.effective_warmup_bars,
            });

            if self.primary_candle_count == self.effective_warmup_bars {
                events.push(RuntimeEvent::WarmupCompleted {
                    completed_primary_candle_count: self.primary_candle_count,
                });
            }

            return RuntimeStep::new(events, self.portfolio.snapshot(candle.close));
        }

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
        let planned = plan_execution(&decision, current_side, candle.close);
        events.push(RuntimeEvent::ExecutionActionPlanned {
            action: planned.action.clone(),
        });

        if let Some(reason) = planned.ignored_reason {
            events.push(RuntimeEvent::StrategyDecisionIgnored { decision, reason });
        }

        match planned.action {
            ExecutionAction::OpenLong {
                quantity,
                stop_loss,
                take_profit,
            } => {
                self.portfolio
                    .open_long_from_flat(&candle, quantity, stop_loss, take_profit)
                    .expect("planned open long should be executable");
                let position = self
                    .portfolio
                    .open_position
                    .clone()
                    .expect("opened long position should exist");
                events.push(RuntimeEvent::PositionOpened { position });
                events.push(RuntimeEvent::PortfolioUpdated {
                    snapshot: self.portfolio.snapshot(candle.close),
                });
            }
            ExecutionAction::CloseLong => {
                let closed_position = self
                    .portfolio
                    .close_long(&candle)
                    .expect("planned close long should be executable");
                events.push(RuntimeEvent::PositionClosed {
                    closed_position,
                    exit_kind: ExitKind::StrategyExit,
                });
                events.push(RuntimeEvent::PortfolioUpdated {
                    snapshot: self.portfolio.snapshot(candle.close),
                });
            }
            ExecutionAction::OpenShort {
                quantity,
                stop_loss,
                take_profit,
            } => {
                self.portfolio
                    .open_short_from_flat(&candle, quantity, stop_loss, take_profit)
                    .expect("planned open short should be executable");
                let position = self
                    .portfolio
                    .open_position
                    .clone()
                    .expect("opened short position should exist");
                events.push(RuntimeEvent::PositionOpened { position });
                events.push(RuntimeEvent::PortfolioUpdated {
                    snapshot: self.portfolio.snapshot(candle.close),
                });
            }
            ExecutionAction::CloseShort => {
                let closed_position = self
                    .portfolio
                    .close_short(&candle)
                    .expect("planned close short should be executable");
                events.push(RuntimeEvent::PositionClosed {
                    closed_position,
                    exit_kind: ExitKind::StrategyExit,
                });
                events.push(RuntimeEvent::PortfolioUpdated {
                    snapshot: self.portfolio.snapshot(candle.close),
                });
            }
            ExecutionAction::Noop | ExecutionAction::ForceClose => {}
        }

        events.push(RuntimeEvent::TradableTickCompleted);

        RuntimeStep::new(events, self.portfolio.snapshot(candle.close))
    }

    pub fn force_close(&mut self, mark_candle: Candle, reason: impl Into<String>) -> RuntimeStep {
        let mut events = vec![RuntimeEvent::ForceCloseRequested {
            candle: mark_candle.clone(),
            reason: reason.into(),
        }];

        let closed_position = match self
            .portfolio
            .open_position
            .as_ref()
            .map(|position| position.side)
        {
            Some(PositionSide::Long) => Some(
                self.portfolio
                    .close_long(&mark_candle)
                    .expect("open long position should be force-closeable"),
            ),
            Some(PositionSide::Short) => Some(
                self.portfolio
                    .close_short(&mark_candle)
                    .expect("open short position should be force-closeable"),
            ),
            None => None,
        };

        if let Some(closed_position) = closed_position {
            events.push(RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::ForceClose,
            });
            events.push(RuntimeEvent::PositionClosed {
                closed_position,
                exit_kind: ExitKind::ForceClose,
            });
            events.push(RuntimeEvent::PortfolioUpdated {
                snapshot: self.portfolio.snapshot(mark_candle.close),
            });
        } else {
            events.push(RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            });
            events.push(RuntimeEvent::ForceCloseIgnored {
                reason: ForceCloseIgnoredReason::NoOpenPosition,
            });
        }

        events.push(RuntimeEvent::ForceCloseCompleted);

        RuntimeStep::new(events, self.portfolio.snapshot(mark_candle.close))
    }

    pub fn effective_warmup_bars(&self) -> usize {
        self.effective_warmup_bars
    }
}
