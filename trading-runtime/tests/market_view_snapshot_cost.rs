use domain::{Candle, Timeframe};
use std::{hint::black_box, time::Duration, time::Instant};
use trading_runtime::{MarketInput, PortfolioState, RhaiStrategy, RuntimeConfig, TradingRuntime};

#[derive(Debug, Clone)]
struct SnapshotScenario {
    name: &'static str,
    history_per_timeframe: usize,
    secondary_timeframes: Vec<Timeframe>,
    measured_primary_ticks: usize,
}

#[derive(Debug)]
struct SnapshotMeasurement {
    scenario_name: &'static str,
    configured_timeframes: usize,
    history_per_timeframe: usize,
    visible_candles_on_first_tick: usize,
    measured_primary_ticks: usize,
    elapsed: Duration,
}

impl SnapshotMeasurement {
    fn mean_per_tick(&self) -> Duration {
        self.elapsed / self.measured_primary_ticks as u32
    }
}

#[ignore = "smoke benchmark; run with --ignored --nocapture when checking Rhai Market View snapshot cost"]
#[test]
fn measure_rhai_market_view_snapshot_cost() {
    let measurements: Vec<_> = snapshot_scenarios()
        .into_iter()
        .map(measure_snapshot_scenario)
        .collect();

    println!(
        "| scenario | timeframes | history/timeframe | visible candles on first tick | measured ticks | elapsed | mean/tick |"
    );
    println!("|---|---:|---:|---:|---:|---:|---:|");
    for measurement in &measurements {
        println!(
            "| {} | {} | {} | {} | {} | {} | {} |",
            measurement.scenario_name,
            measurement.configured_timeframes,
            measurement.history_per_timeframe,
            measurement.visible_candles_on_first_tick,
            measurement.measured_primary_ticks,
            format_duration(measurement.elapsed),
            format_duration(measurement.mean_per_tick()),
        );
    }

    assert_eq!(measurements.len(), 4);
}

fn snapshot_scenarios() -> Vec<SnapshotScenario> {
    vec![
        SnapshotScenario {
            name: "typical_primary_only_500",
            history_per_timeframe: 500,
            secondary_timeframes: vec![],
            measured_primary_ticks: 2_000,
        },
        SnapshotScenario {
            name: "long_primary_only_10k",
            history_per_timeframe: 10_000,
            secondary_timeframes: vec![],
            measured_primary_ticks: 500,
        },
        SnapshotScenario {
            name: "typical_multi_timeframe_500_each",
            history_per_timeframe: 500,
            secondary_timeframes: vec![Timeframe::hours(1), Timeframe::days(1)],
            measured_primary_ticks: 2_000,
        },
        SnapshotScenario {
            name: "long_multi_timeframe_10k_each",
            history_per_timeframe: 10_000,
            secondary_timeframes: vec![Timeframe::hours(1), Timeframe::days(1)],
            measured_primary_ticks: 250,
        },
    ]
}

fn measure_snapshot_scenario(scenario: SnapshotScenario) -> SnapshotMeasurement {
    let primary_timeframe = Timeframe::minutes(1);
    let strategy = RhaiStrategy::load(&strategy_source(&scenario.secondary_timeframes))
        .expect("measurement strategy should load");
    let runtime_config = RuntimeConfig::from_strategy_config("BTC-USD", strategy.strategy_config())
        .expect("measurement runtime config should resolve");
    let mut runtime =
        TradingRuntime::with_config(runtime_config, PortfolioState::new(10_000.0), 0, strategy);

    let max_timeframe_duration = std::iter::once(primary_timeframe)
        .chain(scenario.secondary_timeframes.iter().copied())
        .map(Timeframe::duration_ms)
        .max()
        .expect("at least primary timeframe should exist");
    let measurement_start_timestamp =
        (scenario.history_per_timeframe as i64 + 10) * max_timeframe_duration;

    seed_history(
        &mut runtime,
        primary_timeframe,
        scenario.history_per_timeframe,
        measurement_start_timestamp,
    );
    for timeframe in scenario.secondary_timeframes.iter().copied() {
        seed_history(
            &mut runtime,
            timeframe,
            scenario.history_per_timeframe,
            measurement_start_timestamp,
        );
    }

    let visible_candles_on_first_tick = scenario.history_per_timeframe
        + 1
        + scenario.secondary_timeframes.len() * scenario.history_per_timeframe;

    let started = Instant::now();
    for tick_index in 0..scenario.measured_primary_ticks {
        let timestamp =
            measurement_start_timestamp + tick_index as i64 * primary_timeframe.duration_ms();
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(
                primary_timeframe,
                timestamp,
            )))
            .expect("primary measurement tick should be accepted");
        black_box(step.events.len());
        black_box(step.portfolio_snapshot.current_equity);
    }
    let elapsed = started.elapsed();

    SnapshotMeasurement {
        scenario_name: scenario.name,
        configured_timeframes: 1 + scenario.secondary_timeframes.len(),
        history_per_timeframe: scenario.history_per_timeframe,
        visible_candles_on_first_tick,
        measured_primary_ticks: scenario.measured_primary_ticks,
        elapsed,
    }
}

fn seed_history<S: trading_runtime::StrategyHandler>(
    runtime: &mut TradingRuntime<S>,
    timeframe: Timeframe,
    history_len: usize,
    measurement_start_timestamp: i64,
) {
    for index in 0..history_len {
        let timestamp =
            measurement_start_timestamp - (history_len - index) as i64 * timeframe.duration_ms();
        runtime
            .on_market_input(MarketInput::WarmupCandle(candle(timeframe, timestamp)))
            .expect("warmup candle should be accepted");
    }
}

fn candle(timeframe: Timeframe, timestamp: i64) -> Candle {
    let close = 100.0 + (timestamp.rem_euclid(10_000) as f64 / 10_000.0);
    Candle {
        timestamp,
        symbol: "BTC-USD".to_string(),
        open: close - 0.25,
        high: close + 0.5,
        low: close - 0.5,
        close,
        volume: 1_000.0,
        timeframe,
    }
}

fn strategy_source(secondary_timeframes: &[Timeframe]) -> String {
    let mut source = String::from(
        r#"
fn strategy_config() {
    let config = strategy_config::new()
        .with_primary(timeframe("1m"));
"#,
    );

    for timeframe in secondary_timeframes {
        source.push_str(&format!(
            r#"    config = config.with_secondary(secondary::required(timeframe("{timeframe}")).with_max_missing_candles(1000));
"#
        ));
    }

    source.push_str(
        r#"    config
}

fn on_tick(market, context) {
    let visible_candles = market.candles().len();
"#,
    );

    for timeframe in secondary_timeframes {
        source.push_str(&format!(
            r#"    visible_candles += market.candles(timeframe("{timeframe}")).len();
"#
        ));
    }

    source.push_str(
        r#"    if visible_candles < 0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#,
    );

    source
}

fn format_duration(duration: Duration) -> String {
    let nanos = duration.as_nanos();
    if nanos >= 1_000_000_000 {
        format!("{:.3}s", nanos as f64 / 1_000_000_000.0)
    } else if nanos >= 1_000_000 {
        format!("{:.3}ms", nanos as f64 / 1_000_000.0)
    } else if nanos >= 1_000 {
        format!("{:.3}µs", nanos as f64 / 1_000.0)
    } else {
        format!("{nanos}ns")
    }
}
