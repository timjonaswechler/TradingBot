use shared::{Candle, Position, PositionSide};
use std::{cell::RefCell, collections::VecDeque, rc::Rc};
use trading_runtime::{
    ClosedPosition, ExecutionAction, ExitKind, ForceCloseIgnoredReason, IgnoredDecisionReason,
    PortfolioState, PredeterminedStrategyHandler, RiskExitKind, RiskExitTriggered, RuntimeEvent,
    RuntimePortfolioSnapshot, StrategyDecision, StrategyDecisionIntent, StrategyHandler,
    TradingRuntime,
};

fn candle(timestamp: i64, close: f64) -> Candle {
    ohlc_candle(timestamp, close, close, close, close)
}

fn ohlc_candle(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open,
        high,
        low,
        close,
        volume: 1_000.0,
        timeframe: "1m".into(),
    }
}

fn position(side: PositionSide, entry_time: i64, entry_price: f64, size: f64) -> Position {
    position_with_entry_risk(side, entry_time, entry_price, size, None, None)
}

fn position_with_entry_risk(
    side: PositionSide,
    entry_time: i64,
    entry_price: f64,
    size: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> Position {
    Position {
        symbol: "BTC-USD".into(),
        side,
        entry_price,
        size,
        entry_time,
        stop_loss,
        take_profit,
    }
}

fn assert_ignored_step(
    step: trading_runtime::RuntimeStep,
    candle: Candle,
    decision: StrategyDecision,
    reason: IgnoredDecisionReason,
    expected_snapshot: RuntimePortfolioSnapshot,
) {
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted { candle },
            RuntimeEvent::StrategyDecisionProduced {
                decision: decision.clone(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            },
            RuntimeEvent::StrategyDecisionIgnored { decision, reason },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
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

fn assert_risk_exit_step(
    open_position: Position,
    exit_candle: Candle,
    risk_exit: RiskExitTriggered,
    expected_realized_pnl: f64,
    expected_realized_cash_balance: f64,
) {
    let expected_closed = ClosedPosition {
        position: open_position.clone(),
        exit_price: risk_exit.exit_price,
        exit_time: exit_candle.timestamp,
        realized_pnl: expected_realized_pnl,
    };
    let expected_snapshot = RuntimePortfolioSnapshot {
        realized_cash_balance: expected_realized_cash_balance,
        open_position: None,
        completed_trade_count: 1,
        current_equity: expected_realized_cash_balance,
    };
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(open_position);
    let strategy_calls = Rc::new(RefCell::new(0));
    let mut runtime = TradingRuntime::new(
        portfolio,
        0,
        CountingStrategyHandler::from_decisions(
            Rc::clone(&strategy_calls),
            [
                StrategyDecision::close_long(),
                StrategyDecision::close_short(),
            ],
        ),
    );

    let step = runtime.on_tradable_candle(exit_candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: exit_candle,
            },
            RuntimeEvent::RiskExitTriggered {
                risk_exit: risk_exit.clone(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::RiskExit {
                    side: risk_exit.side,
                    selected: risk_exit.selected,
                    exit_price: risk_exit.exit_price,
                },
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::RiskExit {
                    selected: risk_exit.selected,
                },
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(*strategy_calls.borrow(), 0);
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert!(step.portfolio_snapshot.open_position.is_none());
    assert_eq!(
        step.portfolio_snapshot.current_equity,
        step.portfolio_snapshot.realized_cash_balance
    );
}

#[test]
fn tradable_candle_with_long_stop_loss_risk_exit_closes_before_strategy_tick() {
    assert_risk_exit_step(
        position_with_entry_risk(PositionSide::Long, 1, 100.0, 2.0, Some(90.0), None),
        ohlc_candle(2, 100.0, 105.0, 90.0, 99.0),
        RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: 90.0,
        },
        -20.0,
        980.0,
    );
}

#[test]
fn tradable_candle_with_long_take_profit_risk_exit_closes_at_selected_price() {
    assert_risk_exit_step(
        position_with_entry_risk(PositionSide::Long, 1, 100.0, 2.0, None, Some(120.0)),
        ohlc_candle(2, 100.0, 120.0, 95.0, 110.0),
        RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::TakeProfit,
            triggered: vec![RiskExitKind::TakeProfit],
            exit_price: 120.0,
        },
        40.0,
        1_040.0,
    );
}

#[test]
fn tradable_candle_with_short_stop_loss_risk_exit_closes_at_selected_price() {
    assert_risk_exit_step(
        position_with_entry_risk(PositionSide::Short, 1, 100.0, 2.0, Some(110.0), None),
        ohlc_candle(2, 100.0, 110.0, 95.0, 105.0),
        RiskExitTriggered {
            side: PositionSide::Short,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: 110.0,
        },
        -20.0,
        980.0,
    );
}

#[test]
fn tradable_candle_with_short_take_profit_risk_exit_closes_at_selected_price() {
    assert_risk_exit_step(
        position_with_entry_risk(PositionSide::Short, 1, 100.0, 2.0, None, Some(80.0)),
        ohlc_candle(2, 100.0, 105.0, 80.0, 90.0),
        RiskExitTriggered {
            side: PositionSide::Short,
            selected: RiskExitKind::TakeProfit,
            triggered: vec![RiskExitKind::TakeProfit],
            exit_price: 80.0,
        },
        40.0,
        1_040.0,
    );
}

#[test]
fn tradable_candle_with_both_intrabar_boundaries_selects_stop_loss_and_reports_both() {
    assert_risk_exit_step(
        position_with_entry_risk(PositionSide::Long, 1, 100.0, 2.0, Some(90.0), Some(120.0)),
        ohlc_candle(2, 100.0, 120.0, 90.0, 110.0),
        RiskExitTriggered {
            side: PositionSide::Long,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss, RiskExitKind::TakeProfit],
            exit_price: 90.0,
        },
        -20.0,
        980.0,
    );
}

#[test]
fn tradable_candle_with_no_warmup_and_hold_while_flat_emits_strategy_tick_noop_step() {
    let candle = candle(1, 100.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]),
    );

    let step = runtime.on_tradable_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced {
                decision: StrategyDecision::hold(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(
        step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(candle.close)
    );
}

#[test]
fn runtime_exposes_warmup_requirement_for_runner_fetching() {
    let runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        2,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]),
    );

    assert_eq!(runtime.warmup_requirement(), 2);
}

#[test]
fn warmup_input_advances_market_progress_without_calling_strategy_until_complete() {
    let first_warmup = candle(1, 100.0);
    let second_warmup = candle(2, 101.0);
    let first_tradable = candle(3, 102.0);
    let decision = StrategyDecision::open_long(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        2,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );

    let first_step = runtime.on_warmup_input(first_warmup.clone());
    let second_step = runtime.on_warmup_input(second_warmup.clone());
    let tradable_step = runtime.on_tradable_candle(first_tradable.clone());

    assert_eq!(
        first_step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: first_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1m".into(),
                current_warmup_input_count: 1,
                required_warmup_inputs: 2,
            },
        ]
    );
    assert_eq!(
        first_step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(first_warmup.close)
    );
    assert_eq!(
        second_step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: second_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1m".into(),
                current_warmup_input_count: 2,
                required_warmup_inputs: 2,
            },
            RuntimeEvent::WarmupCompleted {
                completed_timeframes: vec!["1m".into()],
                required_warmup_inputs: 2,
            },
        ]
    );
    assert_eq!(
        second_step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(second_warmup.close)
    );

    assert_eq!(
        tradable_step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: first_tradable.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: first_tradable.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: first_tradable.clone(),
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
                position: position(
                    PositionSide::Long,
                    first_tradable.timestamp,
                    first_tradable.close,
                    2.0,
                ),
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: tradable_step.portfolio_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(
        tradable_step
            .portfolio_snapshot
            .open_position
            .as_ref()
            .map(|p| p.side),
        Some(PositionSide::Long)
    );
}

#[test]
fn warmup_input_crossing_stop_loss_on_initial_open_position_does_not_trade() {
    let warmup = candle(1, 80.0);
    let open_position =
        position_with_entry_risk(PositionSide::Long, 0, 100.0, 2.0, Some(90.0), None);
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(open_position.clone());
    let mut runtime = TradingRuntime::new(
        portfolio,
        1,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::close_long()]),
    );

    let step = runtime.on_warmup_input(warmup.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1m".into(),
                current_warmup_input_count: 1,
                required_warmup_inputs: 1,
            },
            RuntimeEvent::WarmupCompleted {
                completed_timeframes: vec!["1m".into()],
                required_warmup_inputs: 1,
            },
        ]
    );
    assert_eq!(step.portfolio_snapshot.open_position, Some(open_position));
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 0);
    assert!(!step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::StrategyDecisionProduced { .. }
            | RuntimeEvent::ExecutionActionPlanned { .. }
            | RuntimeEvent::RiskExitTriggered { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
            | RuntimeEvent::TradableCandleAccepted { .. }
    )));
}

#[test]
fn zero_warmup_makes_first_tradable_candle_tradable_without_warmup_completed() {
    let candle = candle(1, 100.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]),
    );

    let step = runtime.on_tradable_candle(candle);

    assert!(!step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
    assert!(step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::TradableCandleAccepted { .. })));
}

#[test]
fn tradable_candle_opens_long_from_flat_and_updates_portfolio_snapshot() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::open_long(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );
    let expected_position = position(PositionSide::Long, candle.timestamp, candle.close, 2.0);
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&candle, 2.0, None, None)
        .unwrap();
    let expected_snapshot = expected_portfolio.snapshot(candle.close);

    let step = runtime.on_tradable_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: candle.clone(),
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
                position: expected_position,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_000.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 0);
    assert_eq!(
        step.portfolio_snapshot
            .open_position
            .as_ref()
            .map(|p| p.side),
        Some(PositionSide::Long)
    );
}

#[test]
fn tradable_candle_opens_short_from_flat_and_updates_portfolio_snapshot() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::open_short(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );
    let expected_position = position(PositionSide::Short, candle.timestamp, candle.close, 2.0);
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&candle, 2.0, None, None)
        .unwrap();
    let expected_snapshot = expected_portfolio.snapshot(candle.close);

    let step = runtime.on_tradable_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced { decision },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::OpenShort {
                    quantity: 2.0,
                    stop_loss: None,
                    take_profit: None,
                },
            },
            RuntimeEvent::PositionOpened {
                position: expected_position,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_000.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 0);
    assert_eq!(
        step.portfolio_snapshot
            .open_position
            .as_ref()
            .map(|p| p.side),
        Some(PositionSide::Short)
    );
}

#[test]
fn tradable_candle_opens_long_with_valid_entry_risk_boundaries() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::open_long(2.0).with_entry_risk(Some(90.0), Some(120.0));
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );
    let expected_position = position_with_entry_risk(
        PositionSide::Long,
        candle.timestamp,
        candle.close,
        2.0,
        Some(90.0),
        Some(120.0),
    );

    let step = runtime.on_tradable_candle(candle.clone());

    assert!(step.events.contains(&RuntimeEvent::ExecutionActionPlanned {
        action: ExecutionAction::OpenLong {
            quantity: 2.0,
            stop_loss: Some(90.0),
            take_profit: Some(120.0),
        },
    }));
    assert!(step.events.contains(&RuntimeEvent::PositionOpened {
        position: expected_position.clone(),
    }));
    assert_eq!(
        step.portfolio_snapshot.open_position,
        Some(expected_position)
    );
}

#[test]
fn tradable_candle_opens_short_with_valid_entry_risk_boundaries() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::open_short(2.0).with_entry_risk(Some(110.0), Some(80.0));
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );
    let expected_position = position_with_entry_risk(
        PositionSide::Short,
        candle.timestamp,
        candle.close,
        2.0,
        Some(110.0),
        Some(80.0),
    );

    let step = runtime.on_tradable_candle(candle.clone());

    assert!(step.events.contains(&RuntimeEvent::ExecutionActionPlanned {
        action: ExecutionAction::OpenShort {
            quantity: 2.0,
            stop_loss: Some(110.0),
            take_profit: Some(80.0),
        },
    }));
    assert!(step.events.contains(&RuntimeEvent::PositionOpened {
        position: expected_position.clone(),
    }));
    assert_eq!(
        step.portfolio_snapshot.open_position,
        Some(expected_position)
    );
}

#[test]
fn tradable_candle_opens_with_one_valid_entry_risk_boundary() {
    for (decision, expected_side, expected_stop_loss, expected_take_profit) in [
        (
            StrategyDecision::open_long(2.0).with_entry_risk(Some(90.0), None),
            PositionSide::Long,
            Some(90.0),
            None,
        ),
        (
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(80.0)),
            PositionSide::Short,
            None,
            Some(80.0),
        ),
    ] {
        let candle = candle(1, 100.0);
        let mut runtime = TradingRuntime::new(
            PortfolioState::new(1_000.0),
            0,
            PredeterminedStrategyHandler::from_decisions([decision]),
        );

        let step = runtime.on_tradable_candle(candle.clone());

        assert_eq!(
            step.portfolio_snapshot.open_position,
            Some(position_with_entry_risk(
                expected_side,
                candle.timestamp,
                candle.close,
                2.0,
                expected_stop_loss,
                expected_take_profit,
            ))
        );
    }
}

#[test]
fn tradable_candle_ignores_invalid_entry_risk_without_portfolio_transition() {
    let invalid_decisions = [
        StrategyDecision::open_long(2.0).with_entry_risk(Some(f64::INFINITY), None),
        StrategyDecision::open_long(2.0).with_entry_risk(Some(0.0), None),
        StrategyDecision::open_long(2.0).with_entry_risk(Some(-1.0), None),
        StrategyDecision::open_long(2.0).with_entry_risk(Some(100.0), None),
        StrategyDecision::open_long(2.0).with_entry_risk(Some(101.0), None),
        StrategyDecision::open_long(2.0).with_entry_risk(None, Some(f64::INFINITY)),
        StrategyDecision::open_long(2.0).with_entry_risk(None, Some(0.0)),
        StrategyDecision::open_long(2.0).with_entry_risk(None, Some(-1.0)),
        StrategyDecision::open_long(2.0).with_entry_risk(None, Some(100.0)),
        StrategyDecision::open_long(2.0).with_entry_risk(None, Some(99.0)),
        StrategyDecision::open_short(2.0).with_entry_risk(Some(f64::INFINITY), None),
        StrategyDecision::open_short(2.0).with_entry_risk(Some(0.0), None),
        StrategyDecision::open_short(2.0).with_entry_risk(Some(-1.0), None),
        StrategyDecision::open_short(2.0).with_entry_risk(Some(100.0), None),
        StrategyDecision::open_short(2.0).with_entry_risk(Some(99.0), None),
        StrategyDecision::open_short(2.0).with_entry_risk(None, Some(f64::INFINITY)),
        StrategyDecision::open_short(2.0).with_entry_risk(None, Some(0.0)),
        StrategyDecision::open_short(2.0).with_entry_risk(None, Some(-1.0)),
        StrategyDecision::open_short(2.0).with_entry_risk(None, Some(100.0)),
        StrategyDecision::open_short(2.0).with_entry_risk(None, Some(101.0)),
    ];

    for decision in invalid_decisions {
        let candle = candle(1, 100.0);
        let mut runtime = TradingRuntime::new(
            PortfolioState::new(1_000.0),
            0,
            PredeterminedStrategyHandler::from_decisions([decision.clone()]),
        );

        let step = runtime.on_tradable_candle(candle.clone());

        assert_ignored_step(
            step,
            candle.clone(),
            decision,
            IgnoredDecisionReason::InvalidEntryRisk,
            PortfolioState::new(1_000.0).snapshot(candle.close),
        );
    }
}

#[test]
fn invalid_quantity_wins_before_invalid_entry_risk_while_flat() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::open_long(0.0).with_entry_risk(Some(100.0), Some(99.0));
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );

    let step = runtime.on_tradable_candle(candle.clone());

    assert_ignored_step(
        step,
        candle.clone(),
        decision,
        IgnoredDecisionReason::InvalidQuantity,
        PortfolioState::new(1_000.0).snapshot(candle.close),
    );
}

#[test]
fn position_already_open_wins_before_invalid_quantity_or_entry_risk() {
    let entry_candle = candle(1, 100.0);
    let invalid_candle = candle(2, 105.0);
    let decision = StrategyDecision::open_short(0.0).with_entry_risk(Some(0.0), Some(150.0));
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_long(2.0),
            decision.clone(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_tradable_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::PositionAlreadyOpen,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn tradable_candle_closes_long_position_and_realizes_pnl() {
    let entry_candle = candle(1, 100.0);
    let exit_candle = candle(2, 115.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_long(2.0),
            StrategyDecision::close_long(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let opened_position = position(
        PositionSide::Long,
        entry_candle.timestamp,
        entry_candle.close,
        2.0,
    );
    let expected_closed = ClosedPosition {
        position: opened_position,
        exit_price: exit_candle.close,
        exit_time: exit_candle.timestamp,
        realized_pnl: 30.0,
    };
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();
    expected_portfolio.close_long(&exit_candle).unwrap();
    let expected_snapshot = expected_portfolio.snapshot(exit_candle.close);

    let step = runtime.on_tradable_candle(exit_candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced {
                decision: StrategyDecision::close_long(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::CloseLong,
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::StrategyExit,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn tradable_candle_ignores_invalid_opening_quantities_without_portfolio_transition() {
    for decision in [
        StrategyDecision::new(StrategyDecisionIntent::OpenLong),
        StrategyDecision::open_long(0.0),
        StrategyDecision::open_long(-1.0),
        StrategyDecision::open_long(f64::INFINITY),
    ] {
        let candle = candle(1, 100.0);
        let mut runtime = TradingRuntime::new(
            PortfolioState::new(1_000.0),
            0,
            PredeterminedStrategyHandler::from_decisions([decision.clone()]),
        );

        let step = runtime.on_tradable_candle(candle.clone());

        assert_ignored_step(
            step,
            candle.clone(),
            decision,
            IgnoredDecisionReason::InvalidQuantity,
            PortfolioState::new(1_000.0).snapshot(candle.close),
        );
    }
}

#[test]
fn tradable_candle_ignores_close_decision_while_flat_without_portfolio_transition() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::close_long();
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );

    let step = runtime.on_tradable_candle(candle.clone());

    assert_ignored_step(
        step,
        candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingLongPosition,
        PortfolioState::new(1_000.0).snapshot(candle.close),
    );
}

#[test]
fn tradable_candle_ignores_close_long_while_short_without_portfolio_transition() {
    let entry_candle = candle(1, 100.0);
    let invalid_candle = candle(2, 95.0);
    let decision = StrategyDecision::close_long();
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_short(2.0),
            decision.clone(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_tradable_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingLongPosition,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn tradable_candle_ignores_close_short_while_long_without_portfolio_transition() {
    let entry_candle = candle(1, 100.0);
    let invalid_candle = candle(2, 105.0);
    let decision = StrategyDecision::close_short();
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_long(2.0),
            decision.clone(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_tradable_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingShortPosition,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn tradable_candle_ignores_open_long_while_already_long_without_portfolio_transition() {
    let entry_candle = candle(1, 100.0);
    let invalid_candle = candle(2, 105.0);
    let decision = StrategyDecision::open_long(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_long(2.0),
            decision.clone(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_tradable_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::PositionAlreadyOpen,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn tradable_candle_ignores_open_short_while_already_short_without_portfolio_transition() {
    let entry_candle = candle(1, 100.0);
    let invalid_candle = candle(2, 95.0);
    let decision = StrategyDecision::open_short(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_short(2.0),
            decision.clone(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_tradable_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::PositionAlreadyOpen,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn tradable_candle_closes_short_position_and_realizes_pnl() {
    let entry_candle = candle(1, 100.0);
    let exit_candle = candle(2, 85.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_short(2.0),
            StrategyDecision::close_short(),
        ]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let opened_position = position(
        PositionSide::Short,
        entry_candle.timestamp,
        entry_candle.close,
        2.0,
    );
    let expected_closed = ClosedPosition {
        position: opened_position,
        exit_price: exit_candle.close,
        exit_time: exit_candle.timestamp,
        realized_pnl: 30.0,
    };
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();
    expected_portfolio.close_short(&exit_candle).unwrap();
    let expected_snapshot = expected_portfolio.snapshot(exit_candle.close);

    let step = runtime.on_tradable_candle(exit_candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::StrategyTickStarted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced {
                decision: StrategyDecision::close_short(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::CloseShort,
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::StrategyExit,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn force_close_closes_open_long_position_with_ordered_events() {
    let entry_candle = candle(1, 100.0);
    let mark_candle = candle(2, 115.0);
    let reason = "shutdown liquidation";
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::open_long(2.0)]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let opened_position = position(
        PositionSide::Long,
        entry_candle.timestamp,
        entry_candle.close,
        2.0,
    );
    let expected_closed = ClosedPosition {
        position: opened_position,
        exit_price: mark_candle.close,
        exit_time: mark_candle.timestamp,
        realized_pnl: 30.0,
    };
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();
    expected_portfolio.close_long(&mark_candle).unwrap();
    let expected_snapshot = expected_portfolio.snapshot(mark_candle.close);

    let step = runtime.force_close(mark_candle.clone(), reason);

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::ForceCloseRequested {
                candle: mark_candle.clone(),
                reason: reason.into(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::ForceClose,
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::ForceClose,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::ForceCloseCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn force_close_closes_open_short_position_with_ordered_events() {
    let entry_candle = candle(1, 100.0);
    let mark_candle = candle(2, 85.0);
    let reason = "shutdown liquidation";
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::open_short(2.0)]),
    );
    runtime.on_tradable_candle(entry_candle.clone());
    let opened_position = position(
        PositionSide::Short,
        entry_candle.timestamp,
        entry_candle.close,
        2.0,
    );
    let expected_closed = ClosedPosition {
        position: opened_position,
        exit_price: mark_candle.close,
        exit_time: mark_candle.timestamp,
        realized_pnl: 30.0,
    };
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();
    expected_portfolio.close_short(&mark_candle).unwrap();
    let expected_snapshot = expected_portfolio.snapshot(mark_candle.close);

    let step = runtime.force_close(mark_candle.clone(), reason);

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::ForceCloseRequested {
                candle: mark_candle.clone(),
                reason: reason.into(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::ForceClose,
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::ForceClose,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::ForceCloseCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn force_close_while_flat_emits_ignored_noop_step() {
    let mark_candle = candle(1, 100.0);
    let reason = "shutdown liquidation";
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([]),
    );
    let expected_snapshot = PortfolioState::new(1_000.0).snapshot(mark_candle.close);

    let step = runtime.force_close(mark_candle.clone(), reason);

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::ForceCloseRequested {
                candle: mark_candle.clone(),
                reason: reason.into(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            },
            RuntimeEvent::ForceCloseIgnored {
                reason: ForceCloseIgnoredReason::NoOpenPosition,
            },
            RuntimeEvent::ForceCloseCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
}

#[test]
fn force_close_works_while_runtime_is_still_in_warmup() {
    let restored_entry_candle = candle(1, 100.0);
    let warmup_candle = candle(2, 105.0);
    let mark_candle = candle(3, 115.0);
    let reason = "shutdown liquidation";
    let mut initial_portfolio = PortfolioState::new(1_000.0);
    initial_portfolio
        .open_long_from_flat(&restored_entry_candle, 2.0, None, None)
        .unwrap();
    let mut runtime = TradingRuntime::new(
        initial_portfolio,
        3,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::close_long()]),
    );
    let warmup_step = runtime.on_warmup_input(warmup_candle.clone());
    let expected_closed = ClosedPosition {
        position: position(
            PositionSide::Long,
            restored_entry_candle.timestamp,
            restored_entry_candle.close,
            2.0,
        ),
        exit_price: mark_candle.close,
        exit_time: mark_candle.timestamp,
        realized_pnl: 30.0,
    };
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&restored_entry_candle, 2.0, None, None)
        .unwrap();
    expected_portfolio.close_long(&mark_candle).unwrap();
    let expected_snapshot = expected_portfolio.snapshot(mark_candle.close);

    let step = runtime.force_close(mark_candle.clone(), reason);

    assert_eq!(
        warmup_step.events,
        vec![
            RuntimeEvent::WarmupInputAccepted {
                candle: warmup_candle.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                timeframe: "1m".into(),
                current_warmup_input_count: 1,
                required_warmup_inputs: 3,
            },
        ]
    );
    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::ForceCloseRequested {
                candle: mark_candle.clone(),
                reason: reason.into(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::ForceClose,
            },
            RuntimeEvent::PositionClosed {
                closed_position: expected_closed,
                exit_kind: ExitKind::ForceClose,
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::ForceCloseCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
}
