use shared::Candle;
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use trading_runtime::{
    ExecutionAction, MarketInput, PortfolioState, RuntimeConfig, RuntimeEvent, RuntimeInputError,
    RuntimePortfolioSnapshot, StrategyDecision, StrategyHandler, TradingRuntime,
};

fn candle(timestamp: i64, timeframe: &str, close: f64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1_000.0,
        timeframe: timeframe.into(),
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
fn completed_primary_market_input_routes_to_existing_tradable_candle_behavior() {
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
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: primary.clone(),
            },
            RuntimeEvent::TradableTickStarted {
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
            RuntimeEvent::TradableTickCompleted,
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
        step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: primary_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
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
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: secondary_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
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
    assert_eq!(*calls.borrow(), 0);
    assert_eq!(
        secondary_step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(secondary.close)
    );

    let primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
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

    assert_eq!(
        error,
        RuntimeInputError::UnknownTimeframe {
            timeframe: "5m".into(),
        }
    );
    assert_eq!(*calls.borrow(), 0);

    let primary_step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary))
        .unwrap();

    assert_eq!(*calls.borrow(), 1);
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
        current_warmup_input_count: 1,
        required_warmup_inputs: 2,
    }));
    assert!(!warmup_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
}
