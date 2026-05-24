use trading_runtime::{
    ExecutionAction, PortfolioState, PredeterminedStrategyHandler, RuntimeEvent, StrategyDecision,
    TradingRuntime,
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
