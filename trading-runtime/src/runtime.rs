//! Trading runtime entrypoints.

use crate::market_input::MarketInputTimeframeRole;
use crate::{
    evaluate_risk_exit, plan_execution, ExecutionAction, ExitKind, ForceCloseIgnoredReason,
    MarketInput, PortfolioState, RuntimeConfig, RuntimeEvent, RuntimeInputError, RuntimeStep,
    StrategyHandler,
};
use shared::{Candle, PositionSide};

/// DB-free trading runtime core for one runtime asset.
#[derive(Debug, Clone)]
pub struct TradingRuntime<S> {
    config: RuntimeConfig,
    portfolio: PortfolioState,
    warmup_input_count: usize,
    warmup_requirement: usize,
    strategy_handler: S,
}

impl<S: StrategyHandler> TradingRuntime<S> {
    pub fn new(portfolio: PortfolioState, warmup_requirement: usize, strategy_handler: S) -> Self {
        Self::with_config(
            RuntimeConfig::single_timeframe("BTC-USD", "1m"),
            portfolio,
            warmup_requirement,
            strategy_handler,
        )
    }

    pub fn with_config(
        config: RuntimeConfig,
        portfolio: PortfolioState,
        warmup_requirement: usize,
        strategy_handler: S,
    ) -> Self {
        Self {
            config,
            portfolio,
            warmup_input_count: 0,
            warmup_requirement,
            strategy_handler,
        }
    }

    pub fn on_market_input(
        &mut self,
        input: MarketInput,
    ) -> Result<RuntimeStep, RuntimeInputError> {
        let role = self
            .config
            .classify_timeframe(&input.candle().timeframe)
            .ok_or_else(|| RuntimeInputError::UnknownTimeframe {
                timeframe: input.candle().timeframe.clone(),
            })?;

        match (input, role) {
            (MarketInput::WarmupCandle(candle), _) => Ok(self.on_warmup_input(candle)),
            (MarketInput::CompletedCandle(candle), MarketInputTimeframeRole::Primary) => {
                Ok(self.on_tradable_candle(candle))
            }
            (MarketInput::CompletedCandle(candle), MarketInputTimeframeRole::Secondary) => {
                Ok(self.on_secondary_completed_candle(candle))
            }
        }
    }

    fn on_secondary_completed_candle(&mut self, candle: Candle) -> RuntimeStep {
        RuntimeStep::new(
            vec![RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            }],
            self.portfolio.snapshot(candle.close),
        )
    }

    pub fn on_warmup_input(&mut self, candle: Candle) -> RuntimeStep {
        self.warmup_input_count += 1;

        let mut events = vec![RuntimeEvent::WarmupInputAccepted {
            candle: candle.clone(),
        }];

        events.push(RuntimeEvent::WarmupAdvanced {
            current_warmup_input_count: self.warmup_input_count,
            required_warmup_inputs: self.warmup_requirement,
        });

        if self.warmup_input_count == self.warmup_requirement {
            events.push(RuntimeEvent::WarmupCompleted {
                completed_warmup_input_count: self.warmup_input_count,
            });
        }

        RuntimeStep::new(events, self.portfolio.snapshot(candle.close))
    }

    pub fn on_tradable_candle(&mut self, candle: Candle) -> RuntimeStep {
        let mut events = vec![RuntimeEvent::MarketInputAccepted {
            candle: candle.clone(),
        }];

        events.push(RuntimeEvent::TradableTickStarted {
            candle: candle.clone(),
        });

        if let Some(risk_exit) = self
            .portfolio
            .open_position
            .as_ref()
            .and_then(|position| evaluate_risk_exit(position, &candle))
        {
            events.push(RuntimeEvent::RiskExitTriggered {
                risk_exit: risk_exit.clone(),
            });
            events.push(RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::RiskExit {
                    side: risk_exit.side,
                    selected: risk_exit.selected,
                    exit_price: risk_exit.exit_price,
                },
            });

            let closed_position = match risk_exit.side {
                PositionSide::Long => self
                    .portfolio
                    .close_long_at_price(&candle, risk_exit.exit_price)
                    .expect("planned long risk exit should be executable"),
                PositionSide::Short => self
                    .portfolio
                    .close_short_at_price(&candle, risk_exit.exit_price)
                    .expect("planned short risk exit should be executable"),
            };
            events.push(RuntimeEvent::PositionClosed {
                closed_position,
                exit_kind: ExitKind::RiskExit {
                    selected: risk_exit.selected,
                },
            });
            events.push(RuntimeEvent::PortfolioUpdated {
                snapshot: self.portfolio.snapshot(candle.close),
            });
            events.push(RuntimeEvent::TradableTickCompleted);

            return RuntimeStep::new(events, self.portfolio.snapshot(candle.close));
        }

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
            ExecutionAction::Noop
            | ExecutionAction::RiskExit { .. }
            | ExecutionAction::ForceClose => {}
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

    pub fn warmup_requirement(&self) -> usize {
        self.warmup_requirement
    }
}
