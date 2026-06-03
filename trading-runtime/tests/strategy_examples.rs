use std::{
    fs,
    path::{Path, PathBuf},
};

use domain::Candle;
use trading_runtime::{
    MarketInput, PortfolioState, RhaiStrategy, RuntimeConfig, RuntimeEvent, StrategyDecisionIntent,
    TradingRuntime,
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
        timeframe: "1d".parse().expect("valid timeframe"),
    }
}

fn load_example(name: &str) -> RhaiStrategy {
    RhaiStrategy::load_file(strategies_dir().join(name)).expect("example strategy should load")
}

fn example_source(name: &str) -> String {
    fs::read_to_string(strategies_dir().join(name)).expect("example strategy should be readable")
}

fn produced_intent(events: &[RuntimeEvent]) -> Option<StrategyDecisionIntent> {
    events.iter().find_map(|event| match event {
        RuntimeEvent::StrategyDecisionProduced { decision } => Some(decision.intent),
        _ => None,
    })
}

#[test]
fn maintained_examples_use_current_rhai_authoring_surface() {
    for strategy in ["sma_cross.rhai", "min_loss.rhai", "trendline_break.rhai"] {
        let source = example_source(strategy);
        assert!(
            !source.contains("indicators::"),
            "{strategy} should use canonical ta::* instead of transitional indicators::*"
        );
    }

    for strategy in ["sma_cross.rhai", "min_loss.rhai"] {
        let source = example_source(strategy);
        assert!(
            source.contains("ta::cross_over("),
            "{strategy} should use ta::cross_over for SMA crossover entries"
        );
        assert!(
            source.contains("ta::cross_under("),
            "{strategy} should use ta::cross_under for SMA crossover exits"
        );
    }

    let sma_cross = example_source("sma_cross.rhai");
    assert!(sma_cross.contains("context.portfolio.is_flat()"));
    assert!(sma_cross.contains("context.portfolio.is_long()"));

    let min_loss = example_source("min_loss.rhai");
    assert!(min_loss.contains("position.is_long()"));
    assert!(min_loss.contains("context.portfolio.is_flat()"));
    assert!(min_loss.contains("context.state.float("));
    assert!(min_loss.contains("context.state.set_float("));

    let trendline_break = example_source("trendline_break.rhai");
    assert!(trendline_break.contains("context.portfolio.has_position()"));
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
        let runtime_config =
            RuntimeConfig::from_strategy_config("TEST", strategy_handler.strategy_config())
                .expect("example strategy config should resolve");
        let mut runtime = TradingRuntime::with_config(
            runtime_config,
            PortfolioState::new(10_000.0),
            0,
            strategy_handler,
        );

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
