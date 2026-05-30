//! Trading runtime entrypoints.

use crate::market_input::MarketInputTimeframeRole;
use crate::{
    evaluate_risk_exit, plan_execution, BlockedSecondaryContext, ExecutionAction, ExitKind,
    ForceCloseIgnoredReason, MarketInput, MarketState, MarketView, PortfolioState, RuntimeConfig,
    RuntimeEvent, RuntimeInputError, RuntimeStep, SecondaryContextUnavailableReason,
    SecondaryReadiness, SecondaryTimeframeConfig, StrategyContext, StrategyHandler, StrategyState,
    StrategyTickInput, StrategyTickResult, WarmupPlan,
};
use shared::{Candle, PositionSide, Timeframe};
use std::collections::HashMap;

/// DB-free trading runtime core for one runtime asset.
#[derive(Debug, Clone)]
pub struct TradingRuntime<S> {
    config: RuntimeConfig,
    market_state: MarketState,
    portfolio: PortfolioState,
    warmup_progress: HashMap<Timeframe, usize>,
    warmup_plan: WarmupPlan,
    warmup_completed: bool,
    strategy_state: StrategyState,
    strategy_handler: S,
}

impl<S: StrategyHandler> TradingRuntime<S> {
    pub fn new(portfolio: PortfolioState, warmup_requirement: usize, strategy_handler: S) -> Self {
        Self::with_config(
            RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1)),
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
        let warmup_plan = WarmupPlan::same_requirement(&config, warmup_requirement);
        Self::with_warmup_plan(config, portfolio, warmup_plan, strategy_handler)
    }

    pub fn with_warmup_plan(
        config: RuntimeConfig,
        portfolio: PortfolioState,
        warmup_plan: WarmupPlan,
        strategy_handler: S,
    ) -> Self {
        let market_state = MarketState::from_config(&config);
        let warmup_progress = config
            .configured_timeframes()
            .into_iter()
            .map(|timeframe| (timeframe, 0))
            .collect();
        let warmup_completed = config
            .configured_timeframes()
            .iter()
            .all(|timeframe| warmup_plan.requirement_for(*timeframe).unwrap_or(0) == 0);

        Self {
            config,
            market_state,
            portfolio,
            warmup_progress,
            warmup_plan,
            warmup_completed,
            strategy_state: StrategyState::default(),
            strategy_handler,
        }
    }

    pub fn on_market_input(
        &mut self,
        input: MarketInput,
    ) -> Result<RuntimeStep, RuntimeInputError> {
        let role = self
            .config
            .classify_timeframe(input.candle().timeframe)
            .ok_or_else(|| RuntimeInputError::UnknownTimeframe {
                timeframe: input.candle().timeframe,
            })?;

        match (input, role) {
            (MarketInput::WarmupCandle(candle), _) => Ok(self.on_warmup_input(candle)),
            (MarketInput::CompletedCandle(candle), MarketInputTimeframeRole::Primary) => {
                if self.is_warmup_complete() {
                    Ok(self.handle_completed_primary_after_warmup(candle))
                } else {
                    Ok(self.on_completed_primary_before_warmup_complete(candle))
                }
            }
            (MarketInput::CompletedCandle(candle), MarketInputTimeframeRole::Secondary) => {
                Ok(self.on_secondary_completed_candle(candle))
            }
        }
    }

    fn on_completed_primary_before_warmup_complete(&mut self, candle: Candle) -> RuntimeStep {
        self.market_state.record_accepted_candle(candle.clone());
        self.notify_strategy_market_input_accepted(&candle);

        RuntimeStep::new(
            vec![RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            }],
            self.portfolio.snapshot(candle.close),
        )
    }

    fn on_secondary_completed_candle(&mut self, candle: Candle) -> RuntimeStep {
        self.market_state.record_accepted_candle(candle.clone());
        self.notify_strategy_market_input_accepted(&candle);

        RuntimeStep::new(
            vec![RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            }],
            self.portfolio.snapshot(candle.close),
        )
    }

    pub fn on_warmup_input(&mut self, candle: Candle) -> RuntimeStep {
        self.market_state.record_accepted_candle(candle.clone());
        self.notify_strategy_market_input_accepted(&candle);
        let timeframe = candle.timeframe;
        let current_warmup_input_count = self.advance_warmup_progress(timeframe);

        let mut events = vec![RuntimeEvent::WarmupInputAccepted {
            candle: candle.clone(),
        }];

        events.push(RuntimeEvent::WarmupAdvanced {
            timeframe,
            current_warmup_input_count,
            required_warmup_inputs: self.required_warmup_inputs(timeframe),
        });

        if !self.warmup_completed && self.is_warmup_complete() {
            self.warmup_completed = true;
            events.push(RuntimeEvent::WarmupCompleted {
                completed_timeframes: self.config.configured_timeframes(),
                required_warmup_inputs: self.warmup_requirement(),
            });
        }

        RuntimeStep::new(events, self.portfolio.snapshot(candle.close))
    }

    fn handle_completed_primary_after_warmup(&mut self, candle: Candle) -> RuntimeStep {
        self.market_state.record_accepted_candle(candle.clone());
        self.notify_strategy_market_input_accepted(&candle);

        let mut events = vec![RuntimeEvent::MarketInputAccepted {
            candle: candle.clone(),
        }];

        events.push(RuntimeEvent::TradableCandleAccepted {
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
            events.push(RuntimeEvent::TradableCandleCompleted);

            return RuntimeStep::new(events, self.portfolio.snapshot(candle.close));
        }

        let blocked_contexts = self.evaluate_secondary_readiness(&candle, &mut events);
        if !blocked_contexts.is_empty() {
            events.push(RuntimeEvent::StrategyTickBlocked {
                candle: candle.clone(),
                blocked_contexts,
            });
            events.push(RuntimeEvent::TradableCandleCompleted);

            return RuntimeStep::new(events, self.portfolio.snapshot(candle.close));
        }

        events.push(RuntimeEvent::StrategyTickStarted {
            candle: candle.clone(),
        });

        let portfolio_before_decision = self.portfolio.snapshot(candle.close);
        let tick_input = StrategyTickInput {
            market: MarketView::new(
                &self.market_state,
                self.config.primary_timeframe,
                self.config.secondary_configs(),
                &candle,
            ),
            context: StrategyContext {
                portfolio: &portfolio_before_decision,
                state: &mut self.strategy_state,
            },
            primary_candle: &candle,
        };
        let decision = match self.strategy_handler.on_tick(tick_input) {
            StrategyTickResult::Decision(decision) => decision,
            StrategyTickResult::Error(error) => {
                events.push(RuntimeEvent::StrategyError {
                    candle: candle.clone(),
                    error,
                });
                events.push(RuntimeEvent::StrategyTickCompleted);
                events.push(RuntimeEvent::TradableCandleCompleted);

                return RuntimeStep::new(events, self.portfolio.snapshot(candle.close));
            }
        };
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

        events.push(RuntimeEvent::StrategyTickCompleted);
        events.push(RuntimeEvent::TradableCandleCompleted);

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
        self.warmup_plan.effective_requirement()
    }

    pub fn warmup_plan(&self) -> &WarmupPlan {
        &self.warmup_plan
    }

    fn required_warmup_inputs(&self, timeframe: Timeframe) -> usize {
        self.warmup_plan.requirement_for(timeframe).unwrap_or(0)
    }

    fn notify_strategy_market_input_accepted(&mut self, candle: &Candle) {
        self.strategy_handler.on_market_input_accepted(
            &self.market_state,
            candle,
            self.config.primary_timeframe,
        );
    }

    fn advance_warmup_progress(&mut self, timeframe: Timeframe) -> usize {
        let count = self.warmup_progress.entry(timeframe).or_insert(0);
        *count += 1;
        *count
    }

    fn is_warmup_complete(&self) -> bool {
        self.config.configured_timeframes().iter().all(|timeframe| {
            self.warmup_progress.get(timeframe).copied().unwrap_or(0)
                >= self.required_warmup_inputs(*timeframe)
        })
    }

    fn evaluate_secondary_readiness(
        &self,
        primary_candle: &Candle,
        events: &mut Vec<RuntimeEvent>,
    ) -> Vec<BlockedSecondaryContext> {
        let mut blocked_contexts = Vec::new();

        for secondary in self.config.secondary_configs() {
            if let Some(reason) = self.secondary_unavailable_reason(primary_candle, secondary) {
                match secondary.readiness {
                    SecondaryReadiness::Required => {
                        blocked_contexts.push(BlockedSecondaryContext {
                            timeframe: secondary.timeframe,
                            reason,
                        });
                    }
                    SecondaryReadiness::Optional => {
                        events.push(RuntimeEvent::SecondaryContextUnavailable {
                            candle: primary_candle.clone(),
                            timeframe: secondary.timeframe,
                            readiness: secondary.readiness,
                            reason,
                        });
                    }
                }
            }
        }

        blocked_contexts
    }

    fn secondary_unavailable_reason(
        &self,
        primary_candle: &Candle,
        secondary: &SecondaryTimeframeConfig,
    ) -> Option<SecondaryContextUnavailableReason> {
        let Some(latest_secondary) = self
            .market_state
            .latest_completed_candle(secondary.timeframe)
        else {
            return Some(SecondaryContextUnavailableReason::Missing);
        };
        let duration_ms = secondary.timeframe.duration_ms();
        let allowed_until = latest_secondary.timestamp.saturating_add(
            duration_ms.saturating_mul(i64::from(secondary.max_missing_candles) + 1),
        );

        (primary_candle.timestamp > allowed_until)
            .then_some(SecondaryContextUnavailableReason::Stale)
    }

    /// Inspect a configured timeframe's chronological Market State history.
    pub fn market_history(&self, timeframe: Timeframe) -> Option<&[Candle]> {
        self.market_state.history(timeframe)
    }
}
