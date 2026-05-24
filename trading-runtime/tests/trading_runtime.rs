use shared::{Position, PositionSide};
use trading_runtime::{
    ClosedPosition, ExecutionAction, ForceCloseIgnoredReason, IgnoredDecisionReason, PortfolioState,
    PredeterminedStrategyHandler, RuntimeEvent, RuntimePortfolioSnapshot, StrategyDecision,
    StrategyDecisionIntent, TradingRuntime,
};

fn candle(timestamp: i64, close: f64) -> shared::Candle {
    shared::Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1_000.0,
        timeframe: "1m".into(),
    }
}

fn position(side: PositionSide, entry_time: i64, entry_price: f64, size: f64) -> Position {
    Position {
        symbol: "BTC-USD".into(),
        side,
        entry_price,
        size,
        entry_time,
        stop_loss: None,
        take_profit: None,
    }
}

fn assert_ignored_step(
    step: trading_runtime::RuntimeStep,
    candle: shared::Candle,
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
            RuntimeEvent::TradableTickStarted { candle },
            RuntimeEvent::StrategyDecisionProduced {
                decision: decision.clone(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            },
            RuntimeEvent::StrategyDecisionIgnored { decision, reason },
            RuntimeEvent::TradableTickCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
}

#[test]
fn primary_candle_with_no_warmup_and_hold_while_flat_emits_tradable_noop_step() {
    let candle = candle(1, 100.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]),
    );

    let step = runtime.on_primary_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableTickStarted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyDecisionProduced {
                decision: StrategyDecision::hold(),
            },
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            },
            RuntimeEvent::TradableTickCompleted,
        ]
    );
    assert_eq!(
        step.portfolio_snapshot,
        PortfolioState::new(1_000.0).snapshot(candle.close)
    );
}

#[test]
fn positive_warmup_advances_market_progress_without_calling_strategy_until_complete() {
    let first_warmup = candle(1, 100.0);
    let second_warmup = candle(2, 101.0);
    let first_tradable = candle(3, 102.0);
    let decision = StrategyDecision::open_long(2.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        2,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );

    let first_step = runtime.on_primary_candle(first_warmup.clone());
    let second_step = runtime.on_primary_candle(second_warmup.clone());
    let tradable_step = runtime.on_primary_candle(first_tradable.clone());

    assert_eq!(
        first_step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: first_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                current_primary_candle_count: 1,
                required_warmup_candles: 2,
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
            RuntimeEvent::MarketInputAccepted {
                candle: second_warmup.clone(),
            },
            RuntimeEvent::WarmupAdvanced {
                current_primary_candle_count: 2,
                required_warmup_candles: 2,
            },
            RuntimeEvent::WarmupCompleted {
                completed_primary_candle_count: 2,
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
            RuntimeEvent::TradableTickStarted {
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
            RuntimeEvent::TradableTickCompleted,
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
fn zero_warmup_makes_first_primary_candle_tradable_without_warmup_completed() {
    let candle = candle(1, 100.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::hold()]),
    );

    let step = runtime.on_primary_candle(candle);

    assert!(!step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. })));
    assert!(step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::TradableTickStarted { .. })));
}

#[test]
fn primary_candle_opens_long_from_flat_and_updates_portfolio_snapshot() {
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

    let step = runtime.on_primary_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableTickStarted {
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
            RuntimeEvent::TradableTickCompleted,
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
fn primary_candle_opens_short_from_flat_and_updates_portfolio_snapshot() {
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

    let step = runtime.on_primary_candle(candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableTickStarted {
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
            RuntimeEvent::TradableTickCompleted,
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
fn primary_candle_closes_long_position_and_realizes_pnl() {
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
    runtime.on_primary_candle(entry_candle.clone());
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

    let step = runtime.on_primary_candle(exit_candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::TradableTickStarted {
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
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::TradableTickCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, expected_snapshot);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn primary_candle_ignores_invalid_opening_quantities_without_portfolio_transition() {
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

        let step = runtime.on_primary_candle(candle.clone());

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
fn primary_candle_ignores_close_decision_while_flat_without_portfolio_transition() {
    let candle = candle(1, 100.0);
    let decision = StrategyDecision::close_long();
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([decision.clone()]),
    );

    let step = runtime.on_primary_candle(candle.clone());

    assert_ignored_step(
        step,
        candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingLongPosition,
        PortfolioState::new(1_000.0).snapshot(candle.close),
    );
}

#[test]
fn primary_candle_ignores_close_long_while_short_without_portfolio_transition() {
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
    runtime.on_primary_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_primary_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingLongPosition,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn primary_candle_ignores_close_short_while_long_without_portfolio_transition() {
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
    runtime.on_primary_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_primary_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::NoMatchingShortPosition,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn primary_candle_ignores_open_long_while_already_long_without_portfolio_transition() {
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
    runtime.on_primary_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_long_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_primary_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::PositionAlreadyOpen,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn primary_candle_ignores_open_short_while_already_short_without_portfolio_transition() {
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
    runtime.on_primary_candle(entry_candle.clone());
    let mut expected_portfolio = PortfolioState::new(1_000.0);
    expected_portfolio
        .open_short_from_flat(&entry_candle, 2.0, None, None)
        .unwrap();

    let step = runtime.on_primary_candle(invalid_candle.clone());

    assert_ignored_step(
        step,
        invalid_candle.clone(),
        decision,
        IgnoredDecisionReason::PositionAlreadyOpen,
        expected_portfolio.snapshot(invalid_candle.close),
    );
}

#[test]
fn primary_candle_closes_short_position_and_realizes_pnl() {
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
    runtime.on_primary_candle(entry_candle.clone());
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

    let step = runtime.on_primary_candle(exit_candle.clone());

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: exit_candle.clone(),
            },
            RuntimeEvent::TradableTickStarted {
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
            },
            RuntimeEvent::PortfolioUpdated {
                snapshot: expected_snapshot.clone(),
            },
            RuntimeEvent::TradableTickCompleted,
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
    runtime.on_primary_candle(entry_candle.clone());
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
    runtime.on_primary_candle(entry_candle.clone());
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
