use trading_runtime::{
    BlockedSecondaryContext, ExitKind, RiskExitKind, RuntimeEvent, RuntimeStep,
    SecondaryContextUnavailableReason, StrategyDecision,
};

fn candle() -> shared::Candle {
    shared::Candle {
        timestamp: 1,
        symbol: "BTC-USD".into(),
        open: 100.0,
        high: 101.0,
        low: 99.0,
        close: 100.0,
        volume: 1_000.0,
        timeframe: "1m".into(),
    }
}

fn snapshot() -> trading_runtime::RuntimePortfolioSnapshot {
    trading_runtime::PortfolioState::new(1_000.0).snapshot(100.0)
}

#[test]
fn runtime_step_returns_ordered_strategy_tick_events_and_current_portfolio_snapshot() {
    let candle = candle();
    let snapshot = snapshot();
    let step = RuntimeStep::new(
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
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ],
        snapshot.clone(),
    );

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
                decision: StrategyDecision::hold(),
            },
            RuntimeEvent::StrategyTickCompleted,
            RuntimeEvent::TradableCandleCompleted,
        ]
    );
    assert_eq!(step.portfolio_snapshot, snapshot);
}

#[test]
fn blocked_strategy_tick_can_complete_tradable_candle_without_strategy_output() {
    let candle = candle();
    let snapshot = snapshot();
    let step = RuntimeStep::new(
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableCandleAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::StrategyTickBlocked {
                candle: candle.clone(),
                blocked_contexts: vec![BlockedSecondaryContext {
                    timeframe: "1h".into(),
                    reason: SecondaryContextUnavailableReason::Missing,
                }],
            },
            RuntimeEvent::TradableCandleCompleted,
        ],
        snapshot,
    );

    assert!(step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::StrategyTickBlocked {
            blocked_contexts,
            ..
        } if blocked_contexts == &vec![BlockedSecondaryContext {
            timeframe: "1h".into(),
            reason: SecondaryContextUnavailableReason::Missing,
        }]
    )));
    assert!(!step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::StrategyDecisionProduced { .. } | RuntimeEvent::ExecutionActionPlanned { .. }
    )));
}

#[test]
fn exit_kind_type_surface_represents_risk_exit_selection() {
    assert_eq!(
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        },
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        }
    );
    assert_ne!(
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        },
        ExitKind::RiskExit {
            selected: RiskExitKind::TakeProfit,
        }
    );
}
