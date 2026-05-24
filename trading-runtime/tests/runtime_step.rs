use trading_runtime::{RuntimeEvent, RuntimeStep, StrategyDecision};

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
fn runtime_step_returns_ordered_events_and_current_portfolio_snapshot() {
    let candle = candle();
    let snapshot = snapshot();
    let step = RuntimeStep::new(
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
        ],
        snapshot.clone(),
    );

    assert_eq!(
        step.events,
        vec![
            RuntimeEvent::MarketInputAccepted {
                candle: candle.clone(),
            },
            RuntimeEvent::TradableTickStarted { candle },
            RuntimeEvent::StrategyDecisionProduced {
                decision: StrategyDecision::hold(),
            },
        ]
    );
    assert_eq!(step.portfolio_snapshot, snapshot);
}
