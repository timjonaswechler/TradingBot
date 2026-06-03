use domain::{Candle, Timeframe};
use std::{cell::RefCell, rc::Rc};
use trading_runtime::{
    ExecutionAction, MarketInput, PortfolioState, RuntimeEvent, StrategyDecision, StrategyError,
    StrategyHandler, StrategyTickInput, StrategyTickResult, TradingRuntime,
};

fn candle(timestamp: i64, close: f64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1_000.0,
        timeframe: Timeframe::minutes(1),
    }
}

#[derive(Clone)]
struct InspectingStrategyHandler {
    seen_primary_timestamp: Rc<RefCell<Option<i64>>>,
    seen_market_latest_timestamp: Rc<RefCell<Option<i64>>>,
    seen_equity: Rc<RefCell<Option<f64>>>,
}

impl StrategyHandler for InspectingStrategyHandler {
    fn on_tick(&mut self, input: StrategyTickInput<'_>) -> StrategyTickResult {
        *self.seen_primary_timestamp.borrow_mut() = Some(input.primary_candle.timestamp);
        *self.seen_market_latest_timestamp.borrow_mut() = input
            .market
            .latest_candle(input.market.primary_timeframe())
            .map(|candle| candle.timestamp);
        *self.seen_equity.borrow_mut() = Some(input.context.portfolio.current_equity);
        let _state_handle = &input.context.state;

        StrategyTickResult::Decision(StrategyDecision::hold())
    }
}

#[test]
fn fake_strategy_receives_primary_candle_market_view_and_portfolio_context() {
    let seen_primary_timestamp = Rc::new(RefCell::new(None));
    let seen_market_latest_timestamp = Rc::new(RefCell::new(None));
    let seen_equity = Rc::new(RefCell::new(None));
    let first = candle(1, 100.0);
    let second = candle(2, 125.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        InspectingStrategyHandler {
            seen_primary_timestamp: Rc::clone(&seen_primary_timestamp),
            seen_market_latest_timestamp: Rc::clone(&seen_market_latest_timestamp),
            seen_equity: Rc::clone(&seen_equity),
        },
    );

    runtime
        .on_market_input(MarketInput::CompletedCandle(first))
        .unwrap();
    let step = runtime
        .on_market_input(MarketInput::CompletedCandle(second.clone()))
        .unwrap();

    assert_eq!(*seen_primary_timestamp.borrow(), Some(second.timestamp));
    assert_eq!(
        *seen_market_latest_timestamp.borrow(),
        Some(second.timestamp)
    );
    assert_eq!(*seen_equity.borrow(), Some(1_000.0));
    assert!(step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::StrategyDecisionProduced { decision }
            if *decision == StrategyDecision::hold()
    )));
}

#[derive(Clone)]
struct ErrorStrategyHandler;

impl StrategyHandler for ErrorStrategyHandler {
    fn on_tick(&mut self, _input: StrategyTickInput<'_>) -> StrategyTickResult {
        StrategyTickResult::Error(StrategyError::new("strategy exploded"))
    }
}

#[test]
fn fake_strategy_error_emits_diagnostic_and_does_not_plan_execution() {
    let primary = candle(1, 100.0);
    let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, ErrorStrategyHandler);

    let step = runtime
        .on_market_input(MarketInput::CompletedCandle(primary.clone()))
        .unwrap();

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
            RuntimeEvent::StrategyError {
                candle: primary,
                error: StrategyError::new("strategy exploded"),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert!(!step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::OpenLong { .. }
                    | ExecutionAction::CloseLong
                    | ExecutionAction::OpenShort { .. }
                    | ExecutionAction::CloseShort
                    | ExecutionAction::Noop
                    | ExecutionAction::RiskExit { .. }
                    | ExecutionAction::ForceClose,
            }
    )));
    assert_eq!(
        step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(100.0)
    );
}
