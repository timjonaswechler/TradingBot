use shared::{Candle, Position, PositionSide, Timeframe};
use std::{
    hint::black_box,
    time::{Duration, Instant},
};
use trading_runtime::{MarketInput, PortfolioState, RhaiStrategy, RuntimeConfig, TradingRuntime};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StrategyProfile {
    HoldBaseline,
    AuthoringSurface,
}

impl StrategyProfile {
    fn label(self) -> &'static str {
        match self {
            Self::HoldBaseline => "hold_baseline",
            Self::AuthoringSurface => "ta_state_portfolio",
        }
    }
}

#[derive(Debug, Clone)]
struct AuthoringScenario {
    name: &'static str,
    comparison_group: &'static str,
    profile: StrategyProfile,
    history_per_timeframe: usize,
    secondary_timeframes: Vec<Timeframe>,
    pre_measure_primary_ticks: usize,
    measured_primary_ticks: usize,
    initial_position: Option<PositionSide>,
}

#[derive(Debug)]
struct AuthoringMeasurement {
    scenario_name: &'static str,
    comparison_group: &'static str,
    profile: StrategyProfile,
    configured_timeframes: usize,
    history_per_timeframe: usize,
    pre_measure_primary_ticks: usize,
    visible_candles_on_first_measured_tick: usize,
    measured_primary_ticks: usize,
    initial_position: Option<PositionSide>,
    elapsed: Duration,
}

impl AuthoringMeasurement {
    fn mean_per_tick(&self) -> Duration {
        self.elapsed / self.measured_primary_ticks as u32
    }
}

#[ignore = "performance smoke; run with --release --ignored --nocapture when measuring Rhai authoring surface cost"]
#[test]
fn measure_rhai_authoring_surface_smoke() {
    let measurements: Vec<_> = authoring_scenarios()
        .into_iter()
        .map(measure_authoring_scenario)
        .collect();

    println!(
        "| scenario | group | profile | timeframes | history/timeframe | pre-measure ticks | visible candles on first measured tick | initial position | measured ticks | elapsed | mean/tick | delta/tick vs group baseline | ratio vs baseline |"
    );
    println!("|---|---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|");

    for measurement in &measurements {
        let baseline = measurements
            .iter()
            .find(|candidate| {
                candidate.comparison_group == measurement.comparison_group
                    && candidate.profile == StrategyProfile::HoldBaseline
            })
            .expect("every measurement group should include a hold baseline");
        let mean = measurement.mean_per_tick();
        let baseline_mean = baseline.mean_per_tick();
        let delta_nanos = mean.as_nanos() as i128 - baseline_mean.as_nanos() as i128;
        let ratio = if baseline_mean.as_nanos() == 0 {
            0.0
        } else {
            mean.as_nanos() as f64 / baseline_mean.as_nanos() as f64
        };

        println!(
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {:.2}x |",
            measurement.scenario_name,
            measurement.comparison_group,
            measurement.profile.label(),
            measurement.configured_timeframes,
            measurement.history_per_timeframe,
            measurement.pre_measure_primary_ticks,
            measurement.visible_candles_on_first_measured_tick,
            position_label(measurement.initial_position),
            measurement.measured_primary_ticks,
            format_duration(measurement.elapsed),
            format_duration(mean),
            format_signed_duration(delta_nanos),
            ratio,
        );
    }

    assert!(measurements
        .iter()
        .any(|measurement| measurement.profile == StrategyProfile::HoldBaseline));
    assert!(measurements
        .iter()
        .any(|measurement| measurement.profile == StrategyProfile::AuthoringSurface));
    assert!(measurements.iter().any(|measurement| {
        measurement.profile == StrategyProfile::AuthoringSurface
            && measurement.configured_timeframes > 1
            && measurement.initial_position.is_some()
    }));
    assert_eq!(measurements.len(), 4);
}

fn authoring_scenarios() -> Vec<AuthoringScenario> {
    vec![
        AuthoringScenario {
            name: "primary_hold_baseline_500",
            comparison_group: "primary_flat_500",
            profile: StrategyProfile::HoldBaseline,
            history_per_timeframe: 500,
            secondary_timeframes: vec![],
            pre_measure_primary_ticks: 200,
            measured_primary_ticks: 5_000,
            initial_position: None,
        },
        AuthoringScenario {
            name: "primary_ta_state_portfolio_500",
            comparison_group: "primary_flat_500",
            profile: StrategyProfile::AuthoringSurface,
            history_per_timeframe: 500,
            secondary_timeframes: vec![],
            pre_measure_primary_ticks: 200,
            measured_primary_ticks: 5_000,
            initial_position: None,
        },
        AuthoringScenario {
            name: "multi_hold_baseline_500_each_long_position",
            comparison_group: "multi_long_position_500_each",
            profile: StrategyProfile::HoldBaseline,
            history_per_timeframe: 500,
            secondary_timeframes: vec![Timeframe::hours(1), Timeframe::days(1)],
            pre_measure_primary_ticks: 200,
            measured_primary_ticks: 2_000,
            initial_position: Some(PositionSide::Long),
        },
        AuthoringScenario {
            name: "multi_ta_state_portfolio_position_500_each",
            comparison_group: "multi_long_position_500_each",
            profile: StrategyProfile::AuthoringSurface,
            history_per_timeframe: 500,
            secondary_timeframes: vec![Timeframe::hours(1), Timeframe::days(1)],
            pre_measure_primary_ticks: 200,
            measured_primary_ticks: 2_000,
            initial_position: Some(PositionSide::Long),
        },
    ]
}

fn measure_authoring_scenario(scenario: AuthoringScenario) -> AuthoringMeasurement {
    let primary_timeframe = Timeframe::minutes(1);
    let max_timeframe_duration = std::iter::once(primary_timeframe)
        .chain(scenario.secondary_timeframes.iter().copied())
        .map(Timeframe::duration_ms)
        .max()
        .expect("at least primary timeframe should exist");
    let measurement_start_timestamp =
        (scenario.history_per_timeframe as i64 + 10) * max_timeframe_duration;

    let strategy = RhaiStrategy::load(&strategy_source(
        scenario.profile,
        &scenario.secondary_timeframes,
    ))
    .expect("measurement strategy should load");
    let runtime_config = RuntimeConfig::from_strategy_config("BTC-USD", strategy.strategy_config())
        .expect("measurement runtime config should resolve");
    let mut portfolio = PortfolioState::new(10_000.0);
    if let Some(side) = scenario.initial_position {
        portfolio.open_position = Some(measured_position(
            side,
            measurement_start_timestamp - primary_timeframe.duration_ms(),
        ));
    }
    let mut runtime = TradingRuntime::with_config(runtime_config, portfolio, 0, strategy);

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

    run_primary_ticks(
        &mut runtime,
        primary_timeframe,
        measurement_start_timestamp,
        scenario.pre_measure_primary_ticks,
    );

    let measured_start_timestamp = measurement_start_timestamp
        + scenario.pre_measure_primary_ticks as i64 * primary_timeframe.duration_ms();
    let visible_candles_on_first_measured_tick = scenario.history_per_timeframe
        + scenario.pre_measure_primary_ticks
        + 1
        + scenario.secondary_timeframes.len() * scenario.history_per_timeframe;

    let started = Instant::now();
    run_primary_ticks(
        &mut runtime,
        primary_timeframe,
        measured_start_timestamp,
        scenario.measured_primary_ticks,
    );
    let elapsed = started.elapsed();

    AuthoringMeasurement {
        scenario_name: scenario.name,
        comparison_group: scenario.comparison_group,
        profile: scenario.profile,
        configured_timeframes: 1 + scenario.secondary_timeframes.len(),
        history_per_timeframe: scenario.history_per_timeframe,
        pre_measure_primary_ticks: scenario.pre_measure_primary_ticks,
        visible_candles_on_first_measured_tick,
        measured_primary_ticks: scenario.measured_primary_ticks,
        initial_position: scenario.initial_position,
        elapsed,
    }
}

fn run_primary_ticks<S: trading_runtime::StrategyHandler>(
    runtime: &mut TradingRuntime<S>,
    primary_timeframe: Timeframe,
    first_timestamp: i64,
    ticks: usize,
) {
    for tick_index in 0..ticks {
        let timestamp = first_timestamp + tick_index as i64 * primary_timeframe.duration_ms();
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(
                primary_timeframe,
                timestamp,
            )))
            .expect("primary measurement tick should be accepted");
        black_box(step.events.len());
        black_box(step.portfolio_snapshot.current_equity);
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

fn measured_position(side: PositionSide, entry_time: i64) -> Position {
    Position {
        symbol: "BTC-USD".to_string(),
        side,
        entry_price: 100.0,
        size: 1.0,
        entry_time,
        stop_loss: Some(50.0),
        take_profit: Some(250.0),
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

fn strategy_source(profile: StrategyProfile, secondary_timeframes: &[Timeframe]) -> String {
    let mut source = strategy_config_source(secondary_timeframes);
    match profile {
        StrategyProfile::HoldBaseline => source.push_str(
            r#"
fn on_tick(market, context) {
    decision::hold()
}
"#,
        ),
        StrategyProfile::AuthoringSurface => {
            source.push_str(&authoring_surface_on_tick_source(secondary_timeframes));
        }
    }
    source
}

fn strategy_config_source(secondary_timeframes: &[Timeframe]) -> String {
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
"#,
    );
    source
}

fn authoring_surface_on_tick_source(secondary_timeframes: &[Timeframe]) -> String {
    let mut source = String::from(
        r#"
fn on_tick(market, context) {
    let candles = market.candles();
    let fast = ta::sma(candles, 20);
    let slow = ta::sma(candles, 50);
    let fast_prev = ta::sma(candles, 20, 1);
    let slow_prev = ta::sma(candles, 50, 1);

    let seen = context.state.int("seen", 0);
    context.state.set_int("seen", seen + 1);

    let flat = context.portfolio.is_flat();
    let has_position = context.portfolio.has_position();
    let portfolio_long = context.portfolio.is_long();
    let portfolio_short = context.portfolio.is_short();
    let position = context.portfolio.position;
    let entry_risk_present = false;
    let known_position_side = !has_position;
    if position != () {
        entry_risk_present = position.has_stop_loss() || position.has_take_profit();
        known_position_side = position.is_long() || position.is_short();
    }

    let indicators_ready = fast != () && slow != () && fast_prev != () && slow_prev != ();
    let crossed_over = false;
    let crossed_under = false;
    if indicators_ready {
        crossed_over = ta::cross_over(fast_prev, slow_prev, fast, slow);
        crossed_under = ta::cross_under(fast_prev, slow_prev, fast, slow);
    }

    let secondary_ready = true;
    let secondary_bias = 0.0;
"#,
    );

    for (index, timeframe) in secondary_timeframes.iter().enumerate() {
        source.push_str(&format!(
            r#"    let secondary_{index} = market.candles(timeframe("{timeframe}"));
    if secondary_{index} == () {{
        secondary_ready = false;
    }} else {{
        let secondary_sma_{index} = ta::sma(secondary_{index}, 20);
        if secondary_sma_{index} == () {{
            secondary_ready = false;
        }} else {{
            secondary_bias = secondary_bias + secondary_sma_{index};
        }}
    }}
"#
        ));
    }

    source.push_str(
        r#"
    if !secondary_ready || !known_position_side || secondary_bias < 0.0 {
        return decision::hold().with_reason("authoring guards");
    }

    if crossed_over && flat && !has_position && seen < 0 {
        return decision::open_long(1.0).with_reason("fast crossed above slow");
    }

    if crossed_under && portfolio_long && entry_risk_present && seen < 0 {
        return decision::close_long().with_reason("fast crossed below slow");
    }

    if portfolio_short && entry_risk_present && seen < 0 {
        return decision::close_short().with_reason("short protection check");
    }

    decision::hold()
}
"#,
    );

    source
}

fn position_label(position: Option<PositionSide>) -> &'static str {
    match position {
        Some(PositionSide::Long) => "long",
        Some(PositionSide::Short) => "short",
        None => "flat",
    }
}

fn format_signed_duration(nanos: i128) -> String {
    if nanos == 0 {
        return "0ns".to_string();
    }

    let sign = if nanos < 0 { "-" } else { "+" };
    let abs_nanos = u64::try_from(nanos.unsigned_abs()).unwrap_or(u64::MAX);
    format!("{sign}{}", format_duration(Duration::from_nanos(abs_nanos)))
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
