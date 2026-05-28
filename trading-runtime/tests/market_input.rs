use shared::{Candle, Position, PositionSide};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use trading_runtime::{
    ExecutionAction, MarketInput, PortfolioState, RuntimeConfig, RuntimeEvent, RuntimeInputError,
    RuntimePortfolioSnapshot, StrategyDecision, StrategyHandler, TradingRuntime,
};

fn candle(timestamp: i64, timeframe: &str, close: f64) -> Candle {
    ohlc_candle(timestamp, timeframe, close, close, close, close)
}

fn ohlc_candle(
    timestamp: i64,
    timeframe: &str,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open,
        high,
        low,
        close,
        volume: 1_000.0,
        timeframe: timeframe.into(),
    }
}

fn position_with_entry_risk(
    side: PositionSide,
    entry_price: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> Position {
    Position {
        symbol: "BTC-USD".into(),
        side,
        entry_price,
        size: 2.0,
        entry_time: 0,
        stop_loss,
        take_profit,
    }
}

fn runtime_config() -> RuntimeConfig {
    RuntimeConfig::new("BTC-USD", "1m", ["1h"])
}

#[derive(Clone)]
struct CountingStrategyHandler {
    calls: Rc<RefCell<usize>>,
    decisions: VecDeque<StrategyDecision>,
}

impl CountingStrategyHandler {
    fn from_decisions(
        calls: Rc<RefCell<usize>>,
        decisions: impl IntoIterator<Item = StrategyDecision>,
    ) -> Self {
        Self {
            calls,
            decisions: decisions.into_iter().collect(),
        }
    }
}

impl StrategyHandler for CountingStrategyHandler {
    fn next_decision(
        &mut self,
        _candle: &Candle,
        _portfolio: &RuntimePortfolioSnapshot,
    ) -> StrategyDecision {
        *self.calls.borrow_mut() += 1;
        self.decisions
            .pop_front()
            .unwrap_or_else(StrategyDecision::hold)
    }
}

#[test]
fn completed_primary_market_input_emits_tradable_candle_and_strategy_tick_events() {
    let primary = candle(1, "1m", 100.0);
    let decision = StrategyDecision::open_long(2.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        0,
        CountingStrategyHandler::from_decisions(Rc::clone(&calls), [decision.clone()]),
    );

    let step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
    assert_eq!(runtime.market_history("1m"), Some(&[primary.clone()][..]));
    assert_eq!(runtime.market_history("1h"), Some(&[][..]));
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: primary.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: primary.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: primary.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced { decision },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::OpenLong {
                    quantity: 2.0,
                    stop_loss: None,
                    take_profit: None,
                },
            },
            RuntimeEvent::PositionOpened {
                position: step.portfolio_snapshot.open_position.clone().unwrap(),
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: step.portfolio_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert!(step.portfolio_snapshot.open_position.is_some());
}

#[test]
fn warmup_primary_market_input_routes_to_existing_warmup_behavior() {
    let primary_warmup = candle(1, "1m", 100.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        2,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::open_long(2.0)],
        ),
    );

    let step = runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        runtime.market_history("1m"),
        Some(&[primary_warmup.clone()][..])
    );
    assert_eq!(runtime.market_history("1h"), Some(&[][..]));
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: primary_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1m".into(),
                current_warmup_input_count: 1,
                required_warmup_inputs: 2,
            },
        ]
    );
    assert_eq!(
        step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(primary_warmup.close)
    );
}

#[test]
fn warmup_secondary_market_input_is_accepted_without_strategy_or_portfolio_transition() {
    let secondary_warmup = candle(1, "1h", 100.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        2,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::open_long(2.0)],
        ),
    );

    let step = runtime
        .on_market_input(MarketInput::WarmupCandle(secondary_warmup.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(runtime.market_history("1m"), Some(&[][..]));
    assert_eq!(
        runtime.market_history("1h"),
        Some(&[secondary_warmup.clone()][..])
    );
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: secondary_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1h".into(),
                current_warmup_input_count: 1,
                required_warmup_inputs: 2,
            },
        ]
    );
    assert_eq!(
        step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(secondary_warmup.close)
    );
}

#[test]
fn completed_secondary_market_input_is_accepted_without_strategy_or_portfolio_transition() {
    let secondary = candle(1, "1h", 100.0);
    let primary = candle(2, "1m", 101.0);
    let decision = StrategyDecision::open_long(2.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        0,
        CountingStrategyHandler::from_decisions(Rc::clone(&calls), [decision.clone()]),
    );

    let secondary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(secondary.clone()))
        .unwrap();

    assert_eq!(
        secondary_step.events,
        vec![RuntimeEvent::MarketInputAccepted {
            candle: secondary.clone(),
        }]
    );
    assert_eq!(runtime.market_history("1m"), Some(&[][..]));
    assert_eq!(runtime.market_history("1h"), Some(&[secondary.clone()][..]));
    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        secondary_step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(secondary.close)
    );

    let primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
    assert_eq!(runtime.market_history("1m"), Some(&[primary][..]));
    assert_eq!(runtime.market_history("1h"), Some(&[secondary][..]));
    assert_eq!(
        primary_step
            .portfolio_snapshot
            .open_position
            .as_ref()
            .map(|position| position.size),
        Some(2.0)
    );
}

#[test]
fn interleaved_completed_primary_and_secondary_inputs_only_evaluate_primary_inputs() {
    let first_secondary = candle(1, "1h", 100.0);
    let first_primary = candle(2, "1m", 101.0);
    let second_secondary = candle(3, "1h", 102.0);
    let second_primary = candle(4, "1m", 103.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        0,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [
                StrategyDecision::open_long(2.0),
                StrategyDecision::close_long(),
            ],
        ),
    );

    let first_secondary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(first_secondary.clone()))
        .unwrap();
    let first_primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(first_primary.clone()))
        .unwrap();
    let second_secondary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(second_secondary.clone()))
        .unwrap();
    let second_primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(second_primary.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 2);
    assert_eq!(
        first_secondary_step.events,
        vec![RuntimeEvent::MarketInputAccepted {
            candle: first_secondary.clone(),
        }]
    );
    assert_eq!(
        second_secondary_step.events,
        vec![RuntimeEvent::MarketInputAccepted {
            candle: second_secondary.clone(),
        }]
    );
    assert!(!first_secondary_step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::TradableCandleAccepted { .. }
            | RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::ExecutionActionPlanned { .. }
            | RuntimeEvent::PositionOpened { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
    )));
    assert!(!second_secondary_step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::TradableCandleAccepted { .. }
            | RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::ExecutionActionPlanned { .. }
            | RuntimeEvent::PositionOpened { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
    )));
    assert!(first_primary_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    assert!(second_primary_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    assert!(second_primary_step
        .portfolio_snapshot
        .open_position
        .is_none());
    assert_eq!(
        second_primary_step.portfolio_snapshot.completed_trade_count,
        1
    );
    assert_eq!(
        runtime.market_history("1m"),
        Some(&[first_primary, second_primary][..])
    );
    assert_eq!(
        runtime.market_history("1h"),
        Some(&[first_secondary, second_secondary][..])
    );
}

#[test]
fn completed_secondary_candle_that_crosses_entry_risk_does_not_trigger_risk_exit() {
    let secondary_crossing_stop = ohlc_candle(1, "1h", 100.0, 101.0, 85.0, 88.0);
    let primary_crossing_stop = ohlc_candle(2, "1m", 100.0, 101.0, 85.0, 88.0);
    let calls = Rc::new(RefCell::new(0));
    let mut portfolio = PortfolioState::new(1_000.0);
    let open_position =
        position_with_entry_risk(PositionSide::Long, 100.0, Some(90.0), Some(120.0));
    portfolio.open_position = Some(open_position.clone());
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        portfolio,
        0,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::close_long()],
        ),
    );

    let secondary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(
            secondary_crossing_stop.clone(),
        ))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        secondary_step.events,
        vec![RuntimeEvent::MarketInputAccepted {
            candle: secondary_crossing_stop.clone(),
        }]
    );
    assert_eq!(
        secondary_step.portfolio_snapshot.open_position,
        Some(open_position)
    );
    assert_eq!(secondary_step.portfolio_snapshot.completed_trade_count, 0);
    assert_eq!(runtime.market_history("1m"), Some(&[][..]));
    assert_eq!(
        runtime.market_history("1h"),
        Some(&[secondary_crossing_stop][..])
    );

    let primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary_crossing_stop.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert!(primary_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::RiskExitTriggered { .. })));
    assert!(primary_step.portfolio_snapshot.open_position.is_none());
    assert_eq!(primary_step.portfolio_snapshot.completed_trade_count, 1);
    assert_eq!(
        runtime.market_history("1m"),
        Some(&[primary_crossing_stop][..])
    );
}

#[test]
fn multi_timeframe_warmup_does_not_complete_when_primary_is_ready_but_secondary_is_not() {
    let first_primary = candle(1, "1m", 100.0);
    let second_primary = candle(2, "1m", 101.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        2,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::open_long(2.0)],
        ),
    );

    runtime
        .on_market_input(MarketInput::WarmupCandle(first_primary.clone()))
        .unwrap();
    let second_step = runtime
        .on_market_input(MarketInput::WarmupCandle(second_primary.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        runtime.market_history("1m"),
        Some(&[first_primary, second_primary][..])
    );
    assert_eq!(runtime.market_history("1h"), Some(&[][..]));
    assert!(!second_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
}

#[test]
fn completed_primary_before_all_timeframe_warmups_are_satisfied_is_stored_without_trading() {
    let primary_warmup_1 = candle(1, "1m", 100.0);
    let primary_warmup_2 = candle(2, "1m", 101.0);
    let early_completed_primary = ohlc_candle(3, "1m", 100.0, 105.0, 90.0, 99.0);
    let calls = Rc::new(RefCell::new(0));
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(position_with_entry_risk(
        PositionSide::Long,
        100.0,
        Some(90.0),
        None,
    ));
    let open_position = portfolio.open_position.clone();
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        portfolio,
        2,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::close_long()],
        ),
    );

    runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup_1.clone()))
        .unwrap();
    runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup_2.clone()))
        .unwrap();
    let early_step = runtime
        .on_market_input(MarketInput::CompletedCandle(
            early_completed_primary.clone(),
        ))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        runtime.market_history("1m"),
        Some(
            &[
                primary_warmup_1,
                primary_warmup_2,
                early_completed_primary.clone()
            ][..]
        )
    );
    assert_eq!(runtime.market_history("1h"), Some(&[][..]));
    assert_eq!(
        early_step.events,
        vec![RuntimeEvent::MarketInputAccepted {
            candle: early_completed_primary.clone(),
        }]
    );
    assert_eq!(early_step.portfolio_snapshot.open_position, open_position);
    assert_eq!(early_step.portfolio_snapshot.completed_trade_count, 0);
    assert!(!early_step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::TradableCandleAccepted { .. }
            | RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::RiskExitTriggered { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
    )));
}

#[test]
fn first_completed_primary_after_all_timeframe_warmups_are_satisfied_runs_strategy() {
    let primary_warmup_1 = candle(1, "1m", 100.0);
    let primary_warmup_2 = candle(2, "1m", 101.0);
    let secondary_warmup_1 = candle(3, "1h", 100.0);
    let secondary_warmup_2 = candle(4, "1h", 101.0);
    let first_completed_primary = candle(5, "1m", 102.0);
    let decision = StrategyDecision::open_long(2.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        2,
        CountingStrategyHandler::from_decisions(Rc::clone(&calls), [decision]),
    );

    runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup_1.clone()))
        .unwrap();
    runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup_2.clone()))
        .unwrap();
    runtime
        .on_market_input(MarketInput::WarmupCandle(secondary_warmup_1.clone()))
        .unwrap();
    let completion_step = runtime
        .on_market_input(MarketInput::WarmupCandle(secondary_warmup_2.clone()))
        .unwrap();

    assert!(completion_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
    assert_eq!(*calls.borrow(), 0);

    let active_step = runtime
        .on_market_input(MarketInput::CompletedCandle(
            first_completed_primary.clone(),
        ))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
    assert!(active_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    assert!(active_step.portfolio_snapshot.open_position.is_some());
    assert_eq!(
        runtime.market_history("1m"),
        Some(&[primary_warmup_1, primary_warmup_2, first_completed_primary][..])
    );
    assert_eq!(
        runtime.market_history("1h"),
        Some(&[secondary_warmup_1, secondary_warmup_2][..])
    );
}

#[test]
fn unknown_timeframe_returns_input_error_without_consuming_strategy_decisions() {
    let unknown = candle(1, "5m", 100.0);
    let primary = candle(2, "1m", 101.0);
    let decision = StrategyDecision::open_long(2.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        0,
        CountingStrategyHandler::from_decisions(Rc::clone(&calls), [decision]),
    );

    let error = runtime
        .on_market_input(MarketInput::CompletedCandle(unknown))
        .unwrap_err();

    assert_eq!(runtime.market_history("1m"), Some(&[][..]));
    assert_eq!(runtime.market_history("1h"), Some(&[][..]));
    assert_eq!(
        error,
        RuntimeInputError::UnknownTimeframe {
            timeframe: "5m".into(),
        }
    );
    assert_eq!(*calls.borrow(), 0);

    let primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
    assert_eq!(runtime.market_history("1m"), Some(&[primary][..]));
    assert!(primary_step.portfolio_snapshot.open_position.is_some());
}

#[test]
fn unknown_timeframe_returns_input_error_without_advancing_warmup() {
    let unknown = candle(1, "5m", 100.0);
    let primary_warmup = candle(2, "1m", 101.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        2,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::open_long(2.0)],
        ),
    );

    let error = runtime
        .on_market_input(MarketInput::WarmupCandle(unknown))
        .unwrap_err();

    assert_eq!(
        error,
        RuntimeInputError::UnknownTimeframe {
            timeframe: "5m".into(),
        }
    );

    let warmup_step = runtime
        .on_market_input(MarketInput::WarmupCandle(primary_warmup))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert!(warmup_step.events.contains(&RuntimeEvent::WarmupAdvanced {
        timeframe: "1m".into(),
        current_warmup_input_count: 1,
        required_warmup_inputs: 2,
    }));
    assert!(!warmup_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
}

#[test]
fn completed_primary_is_stored_even_when_risk_exit_prevents_strategy_tick() {
    let exit_candle = ohlc_candle(2, "1m", 100.0, 105.0, 90.0, 99.0);
    let calls = Rc::new(RefCell::new(0));
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(position_with_entry_risk(
        PositionSide::Long,
        100.0,
        Some(90.0),
        None,
    ));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        portfolio,
        0,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&calls),
            [StrategyDecision::close_long()],
        ),
    );

    let step = runtime
        .on_market_input(MarketInput::CompletedCandle(exit_candle.clone()))
        .unwrap();

    assert_eq!(*calls.borrow(), 0);
    assert_eq!(runtime.market_history("1m"), Some(&[exit_candle][..]));
    assert!(step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::RiskExitTriggered { .. })));
    assert!(!step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
}

#[test]
fn unknown_timeframe_error_leaves_existing_market_state_histories_unchanged() {
    let primary = candle(1, "1m", 100.0);
    let secondary = candle(2, "1h", 101.0);
    let unknown = candle(3, "5m", 102.0);
    let calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::with_config(
        runtime_config(),
        PortfolioState::new(1_000.0),
        0,
        CountingStrategyHandler::from_decisions(Rc::clone(&calls), [StrategyDecision::hold()]),
    );

    runtime
        .on_market_input(MarketInput::CompletedCandle(primary.clone()))
        .unwrap();
    runtime
        .on_market_input(MarketInput::CompletedCandle(secondary.clone()))
        .unwrap();
    let primary_before = runtime.market_history("1m").unwrap().to_vec();
    let secondary_before = runtime.market_history("1h").unwrap().to_vec();

    let error = runtime
        .on_market_input(MarketInput::CompletedCandle(unknown))
        .unwrap_err();

    assert_eq!(
        error,
        RuntimeInputError::UnknownTimeframe {
            timeframe: "5m".into(),
        }
    );
    assert_eq!(runtime.market_history("1m"), Some(&primary_before[..]));
    assert_eq!(runtime.market_history("1h"), Some(&secondary_before[..]));
    assert_eq!(runtime.market_history("5m"), None);
}
