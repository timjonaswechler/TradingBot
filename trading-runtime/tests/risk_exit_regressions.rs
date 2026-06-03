use domain::{Candle, EntryRiskParameters, OpenPosition, PositionSide, Timeframe};
use std::{cell::RefCell, rc::Rc};
use trading_runtime::{
    ClosedPosition, ExecutionAction, ExitKind, MarketInput, PortfolioState, RiskExitKind,
    RiskExitTriggered, RuntimeEvent, RuntimeStep, StrategyDecision, StrategyHandler,
    TradingRuntime,
};

fn ohlc_candle(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open,
        high,
        low,
        close,
        volume: 1_000.0,
        timeframe: Timeframe::minutes(1),
    }
}

fn completed_primary_step<S: StrategyHandler>(
    runtime: &mut TradingRuntime<S>,
    candle: Candle,
) -> RuntimeStep {
    runtime
        .on_market_input(MarketInput::CompletedCandle(candle))
        .expect("completed primary candle should be accepted")
}

fn position_with_entry_risk(
    side: PositionSide,
    entry_price: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> OpenPosition {
    OpenPosition {
        symbol: "BTC-USD".into(),
        side,
        entry_price,
        quantity: 2.0,
        entry_time: 1,
        entry_risk: EntryRiskParameters {
            stop_loss,
            take_profit,
        },
    }
}

#[derive(Debug, Clone)]
struct CountingStrategyHandler {
    calls: Rc<RefCell<usize>>,
}

impl StrategyHandler for CountingStrategyHandler {
    fn on_tick(
        &mut self,
        _input: trading_runtime::StrategyTickInput<'_>,
    ) -> trading_runtime::StrategyTickResult {
        *self.calls.borrow_mut() += 1;
        trading_runtime::StrategyTickResult::Decision(StrategyDecision::hold())
    }
}

fn runtime_with_open_position(
    open_position: OpenPosition,
    warmup_requirement: usize,
) -> (TradingRuntime<CountingStrategyHandler>, Rc<RefCell<usize>>) {
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(open_position);
    let strategy_calls = Rc::new(RefCell::new(0));
    let runtime = TradingRuntime::new(
        portfolio,
        warmup_requirement,
        CountingStrategyHandler {
            calls: Rc::clone(&strategy_calls),
        },
    );

    (runtime, strategy_calls)
}

fn risk_exit_event(step: &RuntimeStep) -> &RiskExitTriggered {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::RiskExitTriggered { risk_exit } => Some(risk_exit),
            _ => None,
        })
        .expect("risk exit event should be emitted")
}

fn closed_position_event(step: &RuntimeStep) -> (&ClosedPosition, ExitKind) {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::PositionClosed {
                closed_position,
                exit_kind,
            } => Some((closed_position, *exit_kind)),
            _ => None,
        })
        .expect("position closed event should be emitted")
}

fn execution_action_event(step: &RuntimeStep) -> &ExecutionAction {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::ExecutionActionPlanned { action } => Some(action),
            _ => None,
        })
        .expect("execution action should be planned")
}

fn assert_no_strategy_tick_events(step: &RuntimeStep) {
    assert!(step.events.iter().all(|event| !matches!(
        event,
        RuntimeEvent::StrategyTickStarted { .. }
            | RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::StrategyDecisionIgnored { .. }
            | RuntimeEvent::StrategyTickCompleted
    )));
}

#[test]
fn regression_long_stop_loss_uses_stop_price_not_legacy_candle_close() {
    let open_position = position_with_entry_risk(PositionSide::Long, 100.0, Some(90.0), None);
    let exit_candle = ohlc_candle(2, 100.0, 105.0, 90.0, 88.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position.clone(), 0);

    let step = completed_primary_step(&mut runtime, exit_candle.clone());

    assert_eq!(
        risk_exit_event(&step),
        &RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: 90.0,
        }
    );
    assert_eq!(
        execution_action_event(&step),
        &ExecutionAction::RiskExit {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            exit_price: 90.0,
        }
    );
    let (closed_position, exit_kind) = closed_position_event(&step);
    assert_eq!(closed_position.position, open_position);
    assert_eq!(closed_position.exit_price, 90.0);
    assert_ne!(closed_position.exit_price, exit_candle.close);
    assert_eq!(closed_position.realized_pnl, -20.0);
    assert_eq!(
        exit_kind,
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss
        }
    );
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 980.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
    assert_no_strategy_tick_events(&step);
    assert_eq!(*strategy_calls.borrow(), 0);
}

#[test]
fn regression_short_stop_loss_uses_stop_price_not_legacy_candle_close() {
    let open_position = position_with_entry_risk(PositionSide::Short, 100.0, Some(110.0), None);
    let exit_candle = ohlc_candle(2, 100.0, 110.0, 95.0, 115.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position.clone(), 0);

    let step = completed_primary_step(&mut runtime, exit_candle.clone());

    assert_eq!(
        risk_exit_event(&step),
        &RiskExitTriggered {
            side: PositionSide::Short,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: 110.0,
        }
    );
    let (closed_position, exit_kind) = closed_position_event(&step);
    assert_eq!(closed_position.position, open_position);
    assert_eq!(closed_position.exit_price, 110.0);
    assert_ne!(closed_position.exit_price, exit_candle.close);
    assert_eq!(closed_position.realized_pnl, -20.0);
    assert_eq!(
        exit_kind,
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss
        }
    );
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 980.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
    assert_no_strategy_tick_events(&step);
    assert_eq!(*strategy_calls.borrow(), 0);
}

#[test]
fn regression_take_profit_uses_target_price_not_legacy_candle_close() {
    let open_position = position_with_entry_risk(PositionSide::Long, 100.0, None, Some(120.0));
    let exit_candle = ohlc_candle(2, 100.0, 120.0, 95.0, 112.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position, 0);

    let step = completed_primary_step(&mut runtime, exit_candle.clone());

    assert_eq!(
        risk_exit_event(&step),
        &RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::TakeProfit,
            triggered: vec![RiskExitKind::TakeProfit],
            exit_price: 120.0,
        }
    );
    let (closed_position, exit_kind) = closed_position_event(&step);
    assert_eq!(closed_position.exit_price, 120.0);
    assert_ne!(closed_position.exit_price, exit_candle.close);
    assert_eq!(closed_position.realized_pnl, 40.0);
    assert_eq!(
        exit_kind,
        ExitKind::RiskExit {
            selected: RiskExitKind::TakeProfit
        }
    );
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_040.0);
    assert_no_strategy_tick_events(&step);
    assert_eq!(*strategy_calls.borrow(), 0);
}

#[test]
fn regression_both_intrabar_boundaries_select_stop_loss_and_expose_both_triggered_kinds() {
    let open_position =
        position_with_entry_risk(PositionSide::Long, 100.0, Some(90.0), Some(120.0));
    let exit_candle = ohlc_candle(2, 100.0, 120.0, 90.0, 110.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position, 0);

    let step = completed_primary_step(&mut runtime, exit_candle);

    assert_eq!(
        risk_exit_event(&step),
        &RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss, RiskExitKind::TakeProfit],
            exit_price: 90.0,
        }
    );
    let (closed_position, exit_kind) = closed_position_event(&step);
    assert_eq!(closed_position.exit_price, 90.0);
    assert_eq!(
        exit_kind,
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss
        }
    );
    assert_no_strategy_tick_events(&step);
    assert_eq!(*strategy_calls.borrow(), 0);
}

#[test]
fn regression_warmup_input_crossing_stop_loss_does_not_close_or_emit_risk_exit() {
    let open_position = position_with_entry_risk(PositionSide::Long, 100.0, Some(90.0), None);
    let warmup_candle = ohlc_candle(2, 100.0, 101.0, 80.0, 85.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position.clone(), 1);

    let step = runtime.on_warmup_input(warmup_candle);

    assert!(step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupInputAccepted { .. })));
    assert!(step.events.iter().all(|event| !matches!(
        event,
        RuntimeEvent::RiskExitTriggered { .. }
            | RuntimeEvent::ExecutionActionPlanned { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
            | RuntimeEvent::TradableCandleAccepted { .. }
            | RuntimeEvent::StrategyTickStarted { .. }
            | RuntimeEvent::StrategyTickCompleted
    )));
    assert_eq!(step.portfolio_snapshot.open_position, Some(open_position));
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 0);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_000.0);
    assert_eq!(*strategy_calls.borrow(), 0);
}

#[test]
fn regression_first_tradable_candle_after_warmup_can_close_breached_stop_before_strategy_tick() {
    let open_position = position_with_entry_risk(PositionSide::Long, 100.0, Some(90.0), None);
    let warmup_candle = ohlc_candle(2, 100.0, 101.0, 95.0, 98.0);
    let first_tradable_candle = ohlc_candle(3, 85.0, 88.0, 80.0, 84.0);
    let (mut runtime, strategy_calls) = runtime_with_open_position(open_position, 1);

    let warmup_step = runtime.on_warmup_input(warmup_candle);
    let tradable_step = completed_primary_step(&mut runtime, first_tradable_candle.clone());

    assert!(warmup_step.portfolio_snapshot.open_position.is_some());
    assert_eq!(
        risk_exit_event(&tradable_step),
        &RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: 85.0,
        }
    );
    let (closed_position, exit_kind) = closed_position_event(&tradable_step);
    assert_eq!(closed_position.exit_price, first_tradable_candle.open);
    assert_ne!(closed_position.exit_price, first_tradable_candle.close);
    assert_eq!(closed_position.realized_pnl, -30.0);
    assert_eq!(
        exit_kind,
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss
        }
    );
    assert_eq!(
        tradable_step.portfolio_snapshot.realized_cash_balance,
        970.0
    );
    assert_eq!(tradable_step.portfolio_snapshot.completed_trade_count, 1);
    assert!(tradable_step.portfolio_snapshot.open_position.is_none());
    assert_no_strategy_tick_events(&tradable_step);
    assert_eq!(*strategy_calls.borrow(), 0);
}
