use std::path::{Path, PathBuf};

use shared::Candle;
use trading_runtime::{
    MarketInput, PortfolioState, RhaiStrategy, RuntimeEvent, StrategyDecisionIntent, TradingRuntime,
};

fn strategies_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../strategies")
}

fn candle(close: f64, timestamp: i64) -> Candle {
    Candle {
        timestamp,
        symbol: "TEST".to_string(),
        open: close - 0.5,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1_000.0,
        timeframe: "1m".parse().expect("valid timeframe"),
    }
}

fn load_example(name: &str) -> RhaiStrategy {
    RhaiStrategy::load_file(strategies_dir().join(name)).expect("example strategy should load")
}

fn produced_intent(events: &[RuntimeEvent]) -> Option<StrategyDecisionIntent> {
    events.iter().find_map(|event| match event {
        RuntimeEvent::StrategyDecisionProduced { decision } => Some(decision.intent),
        _ => None,
    })
}

#[test]
fn typed_strategy_examples_load() {
    for strategy in ["sma_cross.rhai", "min_loss.rhai", "trendline_break.rhai"] {
        load_example(strategy);
    }
}

#[test]
fn typed_strategy_examples_run_one_runtime_tick() {
    for strategy in ["sma_cross.rhai", "min_loss.rhai", "trendline_break.rhai"] {
        let strategy_handler = load_example(strategy);
        let mut runtime = TradingRuntime::new(PortfolioState::new(10_000.0), 0, strategy_handler);

        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0, 1)))
            .expect("primary candle should be accepted");

        assert_eq!(
            produced_intent(&step.events),
            Some(StrategyDecisionIntent::Hold),
            "{strategy} should produce HOLD on its first example tick"
        );
    }
}
