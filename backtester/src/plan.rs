use std::{fmt::Write as _, sync::Arc};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate};
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::{Candle, Timeframe};
use trading_runtime::{
    resolve_warmup_plan, ExitKind, RhaiStrategy, RuntimeConfig, RuntimeEvent, WarmupPlan,
};

use crate::{
    run_prepared_runtime_backtest, BacktestResult, HistoricalCandleSeries, HistoricalMarketData,
    RuntimeBacktestConfig, RuntimeBacktestResult,
};

#[derive(Debug, Clone)]
pub struct PlanReport {
    pub title: Option<String>,
    pub tests: Vec<BaselinePlanTest>,
}

#[derive(Debug, Clone)]
pub struct BaselinePlanTest {
    pub name: String,
    pub symbol: String,
    pub interval: String,
    pub initial_balance: f64,
    pub result: BacktestResult,
    pub synthetic: Option<SyntheticMonteCarloReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MonteCarloProcedure {
    CandlePermutation,
    OhlcNoise,
}

#[derive(Debug, Clone)]
pub struct SyntheticMonteCarloReport {
    pub procedure: MonteCarloProcedure,
    pub iterations: Vec<MonteCarloIterationDiagnostics>,
}

#[derive(Debug, Clone)]
pub struct MonteCarloIterationDiagnostics {
    pub iteration: usize,
    pub seed: u64,
    pub final_equity: f64,
    pub max_drawdown: f64,
    pub trade_count: usize,
    pub blocked_strategy_tick_count: usize,
    pub strategy_exit_count: usize,
    pub risk_exit_count: usize,
    pub force_close_count: usize,
}

#[derive(Debug, Clone)]
struct PlanResultSpec {
    title: Option<String>,
    tests: Vec<PlanTestSpec>,
}

impl PlanResultSpec {
    fn new() -> Self {
        Self {
            title: None,
            tests: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
struct PlanTestSpec {
    name: String,
    baseline: Option<BaselinePlanSpec>,
    synthetic: Option<SyntheticPlanSpec>,
}

#[derive(Debug, Clone, PartialEq)]
struct DatasetPlanSpec {
    symbol: String,
    start_ms: i64,
    end_ms: i64,
}

#[derive(Debug, Clone, Copy)]
struct PlanTimestamp {
    timestamp_ms: i64,
}

/// Candle data request made by the Backtest Plan dataset loader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetCandleRequest {
    /// The last `count` candles before `before_ms`, returned in chronological order.
    WarmupPrefix { before_ms: i64, count: usize },
    /// Half-open candle timestamp range `[start_ms, end_ms)`, returned chronologically.
    Range { start_ms: i64, end_ms: i64 },
}

#[derive(Debug, Clone)]
struct RunConfigPlanSpec {
    balance: Option<f64>,
}

impl RunConfigPlanSpec {
    fn new() -> Self {
        Self { balance: None }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct BaselinePlanSpec {
    dataset: DatasetPlanSpec,
    balance: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct MonteCarloConfigPlanSpec {
    iterations: usize,
    base_seed: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct OhlcNoiseConfigPlanSpec {
    mutation_probability: f64,
    max_atr_change: f64,
    atr_period: usize,
}

#[derive(Debug, Clone, PartialEq)]
enum SyntheticProcedurePlanSpec {
    CandlePermutation,
    OhlcNoise(OhlcNoiseConfigPlanSpec),
}

#[derive(Debug, Clone, PartialEq)]
struct SyntheticPlanSpec {
    procedure: SyntheticProcedurePlanSpec,
    baseline: BaselinePlanSpec,
    config: MonteCarloConfigPlanSpec,
}

#[derive(Debug, Clone)]
struct ValidatedPlanResultSpec {
    title: Option<String>,
    tests: Vec<ValidatedPlanTestSpec>,
}

#[derive(Debug, Clone)]
struct ValidatedPlanTestSpec {
    name: String,
    baseline: BaselinePlanSpec,
    synthetic: Option<SyntheticPlanSpec>,
}

pub fn execute_plan<F>(
    strategy_src: &str,
    plan_src: &str,
    mut load_candles: F,
) -> Result<PlanReport>
where
    F: FnMut(&str, Timeframe, DatasetCandleRequest) -> Result<Vec<Candle>>,
{
    let plan = parse_plan(plan_src)?;
    let mut tests = Vec::with_capacity(plan.tests.len());

    for (index, test_spec) in plan.tests.into_iter().enumerate() {
        let test_identity = format!("plan test {} ('{}')", index + 1, test_spec.name);
        let baseline = test_spec.baseline;
        let dataset = baseline.dataset;
        let prepared = prepare_plan_runtime(strategy_src, &dataset, baseline.balance)
            .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        let market_data = load_warmup_aware_market_data(
            &dataset,
            &prepared.effective_config,
            &prepared.warmup_plan,
            &mut load_candles,
        )
        .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        let effective_config = prepared.effective_config.clone();
        let warmup_plan = prepared.warmup_plan.clone();
        let runtime_result = run_prepared_runtime_backtest(
            prepared.strategy,
            effective_config.clone(),
            warmup_plan.clone(),
            market_data.clone(),
            baseline.balance,
        )
        .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        if runtime_result.result.equity_curve.is_empty() {
            return Err(anyhow!(
                "{test_identity} failed: No tradable candles for {}/{} — run `just seed` first.",
                dataset.symbol,
                effective_config.primary_timeframe,
            ));
        }

        let synthetic = match test_spec.synthetic {
            Some(synthetic_spec) => Some(
                run_synthetic_monte_carlo(
                    strategy_src,
                    &effective_config,
                    &warmup_plan,
                    &market_data,
                    baseline.balance,
                    &synthetic_spec,
                )
                .map_err(|error| anyhow!("{test_identity} failed: {error}"))?,
            ),
            None => None,
        };
        let result = runtime_result.result;

        tests.push(BaselinePlanTest {
            name: test_spec.name,
            symbol: dataset.symbol,
            interval: effective_config.primary_timeframe.to_string(),
            initial_balance: baseline.balance,
            result,
            synthetic,
        });
    }

    Ok(PlanReport {
        title: plan.title,
        tests,
    })
}

struct PreparedPlanRuntime {
    strategy: RhaiStrategy,
    effective_config: RuntimeConfig,
    warmup_plan: WarmupPlan,
}

fn prepare_plan_runtime(
    strategy_src: &str,
    dataset: &DatasetPlanSpec,
    initial_balance: f64,
) -> Result<PreparedPlanRuntime> {
    let strategy = RhaiStrategy::load(strategy_src)?;
    let config = RuntimeBacktestConfig::new(dataset.symbol.clone(), initial_balance);
    let effective_config = RuntimeConfig::from_strategy_config(
        config.runtime_asset.clone(),
        strategy.strategy_config(),
    )?;
    let warmup_plan = resolve_warmup_plan(
        &effective_config,
        strategy.strategy_config(),
        strategy.ast(),
        strategy.scope(),
        config.runtime_minimum_warmup,
    );

    Ok(PreparedPlanRuntime {
        strategy,
        effective_config,
        warmup_plan,
    })
}

fn load_warmup_aware_market_data<F>(
    dataset: &DatasetPlanSpec,
    effective_config: &RuntimeConfig,
    warmup_plan: &WarmupPlan,
    load_candles: &mut F,
) -> Result<HistoricalMarketData>
where
    F: FnMut(&str, Timeframe, DatasetCandleRequest) -> Result<Vec<Candle>>,
{
    let primary_timeframe = effective_config.primary_timeframe;
    let primary_requirement = warmup_plan.requirement_for(primary_timeframe).unwrap_or(0);
    let primary_prefix = load_warmup_prefix(
        load_candles,
        &dataset.symbol,
        primary_timeframe,
        dataset.start_ms,
        primary_requirement,
        "Primary",
    )?;
    let visible_primary = load_range(
        load_candles,
        &dataset.symbol,
        primary_timeframe,
        dataset.start_ms,
        dataset.end_ms,
        "visible Primary",
    )?;

    if visible_primary.is_empty() {
        return Err(anyhow!(
            "dataset::load visible Primary window for {}/{} [{}, {}) contains no candles",
            dataset.symbol,
            primary_timeframe,
            dataset.start_ms,
            dataset.end_ms
        ));
    }
    if visible_primary[0].timestamp != dataset.start_ms {
        return Err(anyhow!(
            "dataset::load visible Primary window for {}/{} must begin at requested first tradable candle {} but first candle is {}",
            dataset.symbol,
            primary_timeframe,
            dataset.start_ms,
            visible_primary[0].timestamp
        ));
    }

    let last_visible_primary_ts = visible_primary
        .last()
        .expect("non-empty visible Primary window should have a last candle")
        .timestamp;
    let secondary_context_end = last_visible_primary_ts.checked_add(1).ok_or_else(|| {
        anyhow!(
            "dataset::load visible Primary last timestamp {} cannot be converted to a half-open Secondary context range",
            last_visible_primary_ts
        )
    })?;

    let mut primary = primary_prefix;
    primary.extend(visible_primary);

    let secondary = effective_config
        .secondary_timeframes
        .iter()
        .map(|secondary_config| {
            let timeframe = secondary_config.timeframe;
            let requirement = warmup_plan.requirement_for(timeframe).unwrap_or(0);
            let mut candles = load_warmup_prefix(
                load_candles,
                &dataset.symbol,
                timeframe,
                dataset.start_ms,
                requirement,
                "Secondary",
            )?;
            let context = load_range(
                load_candles,
                &dataset.symbol,
                timeframe,
                dataset.start_ms,
                secondary_context_end,
                "Secondary context",
            )?;
            candles.extend(context);
            Ok(HistoricalCandleSeries { timeframe, candles })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(HistoricalMarketData::with_secondary(primary, secondary))
}

fn run_synthetic_monte_carlo(
    strategy_src: &str,
    effective_config: &RuntimeConfig,
    warmup_plan: &WarmupPlan,
    source_market_data: &HistoricalMarketData,
    initial_balance: f64,
    synthetic_spec: &SyntheticPlanSpec,
) -> Result<SyntheticMonteCarloReport> {
    match &synthetic_spec.procedure {
        SyntheticProcedurePlanSpec::CandlePermutation => {
            let mut iterations = Vec::with_capacity(synthetic_spec.config.iterations);

            for iteration_index in 0..synthetic_spec.config.iterations {
                let seed = derive_monte_carlo_seed(
                    synthetic_spec.config.base_seed,
                    iteration_index,
                    0,
                    CANDLE_PERMUTATION_PROCEDURE_ID,
                );
                let synthetic_market_data = permute_market_data(source_market_data, seed)?;
                let strategy = RhaiStrategy::load(strategy_src)?;
                let runtime_result = run_prepared_runtime_backtest(
                    strategy,
                    effective_config.clone(),
                    warmup_plan.clone(),
                    synthetic_market_data,
                    initial_balance,
                )?;

                if runtime_result.result.equity_curve.is_empty() {
                    return Err(anyhow!(
                        "monte_carlo::candle_permutation iteration {} produced no tradable candles",
                        iteration_index + 1
                    ));
                }

                iterations.push(iteration_diagnostics(
                    iteration_index + 1,
                    seed,
                    &runtime_result,
                ));
            }

            Ok(SyntheticMonteCarloReport {
                procedure: MonteCarloProcedure::CandlePermutation,
                iterations,
            })
        }
        SyntheticProcedurePlanSpec::OhlcNoise(ohlc_noise_config) => {
            ensure_single_timeframe_synthetic_procedure(
                effective_config,
                "monte_carlo::ohlc_noise",
            )?;
            let mut iterations = Vec::with_capacity(synthetic_spec.config.iterations);

            for iteration_index in 0..synthetic_spec.config.iterations {
                let seed = derive_monte_carlo_seed(
                    synthetic_spec.config.base_seed,
                    iteration_index,
                    0,
                    OHLC_NOISE_PROCEDURE_ID,
                );
                let synthetic_market_data =
                    apply_ohlc_noise_to_market_data(source_market_data, ohlc_noise_config, seed)?;
                let strategy = RhaiStrategy::load(strategy_src)?;
                let runtime_result = run_prepared_runtime_backtest(
                    strategy,
                    effective_config.clone(),
                    warmup_plan.clone(),
                    synthetic_market_data,
                    initial_balance,
                )?;

                if runtime_result.result.equity_curve.is_empty() {
                    return Err(anyhow!(
                        "monte_carlo::ohlc_noise iteration {} produced no tradable candles",
                        iteration_index + 1
                    ));
                }

                iterations.push(iteration_diagnostics(
                    iteration_index + 1,
                    seed,
                    &runtime_result,
                ));
            }

            Ok(SyntheticMonteCarloReport {
                procedure: MonteCarloProcedure::OhlcNoise,
                iterations,
            })
        }
    }
}

fn iteration_diagnostics(
    iteration: usize,
    seed: u64,
    runtime_result: &RuntimeBacktestResult,
) -> MonteCarloIterationDiagnostics {
    let counters = RuntimeEventCounters::from_steps(&runtime_result.steps);
    MonteCarloIterationDiagnostics {
        iteration,
        seed,
        final_equity: runtime_result.result.metrics.final_equity,
        max_drawdown: runtime_result.result.metrics.max_drawdown,
        trade_count: runtime_result.result.metrics.trade_count,
        blocked_strategy_tick_count: counters.blocked_strategy_tick_count,
        strategy_exit_count: counters.strategy_exit_count,
        risk_exit_count: counters.risk_exit_count,
        force_close_count: counters.force_close_count,
    }
}

#[derive(Debug, Default)]
struct RuntimeEventCounters {
    blocked_strategy_tick_count: usize,
    strategy_exit_count: usize,
    risk_exit_count: usize,
    force_close_count: usize,
}

impl RuntimeEventCounters {
    fn from_steps(steps: &[trading_runtime::RuntimeStep]) -> Self {
        let mut counters = RuntimeEventCounters::default();

        for event in steps.iter().flat_map(|step| step.events.iter()) {
            match event {
                RuntimeEvent::StrategyTickBlocked { .. } => {
                    counters.blocked_strategy_tick_count += 1;
                }
                RuntimeEvent::PositionClosed { exit_kind, .. } => match exit_kind {
                    ExitKind::StrategyExit => counters.strategy_exit_count += 1,
                    ExitKind::RiskExit { .. } => counters.risk_exit_count += 1,
                    ExitKind::ForceClose => counters.force_close_count += 1,
                },
                _ => {}
            }
        }

        counters
    }
}

const CANDLE_PERMUTATION_PROCEDURE_ID: u64 = 0x4341_4e44_4c45_5031; // "CANDLEP1"
const OHLC_NOISE_PROCEDURE_ID: u64 = 0x4f48_4c43_4e4f_4931; // "OHLCNOI1"
const DEFAULT_OHLC_NOISE_ATR_PERIOD: usize = 14;
const SPLITMIX64_INCREMENT: u64 = 0x9e37_79b9_7f4a_7c15;

/// Derive a reproducible Monte Carlo seed from a declared `base_seed`, zero-based
/// `iteration_index`, zero-based `stage_index`, and stable `procedure_id`.
///
/// The helper folds each input through the SplitMix64 mixer, then each synthetic
/// procedure uses the derived seed to initialize a SplitMix64 stream. Fisher-Yates
/// shuffles consume that stream directly instead of relying on implementation-
/// default RNG behavior.
fn derive_monte_carlo_seed(
    base_seed: u64,
    iteration_index: usize,
    stage_index: usize,
    procedure_id: u64,
) -> u64 {
    let mut state = base_seed;
    state = splitmix64_mixed(state ^ procedure_id);
    state = splitmix64_mixed(state ^ iteration_index as u64);
    splitmix64_mixed(state ^ stage_index as u64)
}

fn splitmix64_mixed(value: u64) -> u64 {
    let mut state = value;
    splitmix64_next(&mut state)
}

fn splitmix64_next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(SPLITMIX64_INCREMENT);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[derive(Debug, Clone)]
struct SplitMix64 {
    state: u64,
}

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        splitmix64_next(&mut self.state)
    }

    fn next_index(&mut self, upper_exclusive: usize) -> usize {
        assert!(upper_exclusive > 0, "upper bound must be non-zero");
        let upper = upper_exclusive as u64;
        let zone = u64::MAX - (u64::MAX % upper);

        loop {
            let value = self.next_u64();
            if value < zone {
                return (value % upper) as usize;
            }
        }
    }

    fn next_unit_f64(&mut self) -> f64 {
        let mantissa = self.next_u64() >> 11;
        mantissa as f64 * (1.0 / ((1u64 << 53) as f64))
    }

    fn next_centered_f64(&mut self) -> f64 {
        self.next_unit_f64() * 2.0 - 1.0
    }
}

fn permute_market_data(source: &HistoricalMarketData, seed: u64) -> Result<HistoricalMarketData> {
    let mut rng = SplitMix64::new(seed);
    let primary = permute_candles_by_timestamp_slots(&source.primary, &mut rng, "Primary")?;
    let secondary = source
        .secondary
        .iter()
        .map(|series| {
            Ok(HistoricalCandleSeries {
                timeframe: series.timeframe,
                candles: permute_candles_by_timestamp_slots(
                    &series.candles,
                    &mut rng,
                    "Secondary",
                )?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(HistoricalMarketData::with_secondary(primary, secondary))
}

fn permute_candles_by_timestamp_slots(
    source: &[Candle],
    rng: &mut SplitMix64,
    role: &str,
) -> Result<Vec<Candle>> {
    let mut slots = source.to_vec();
    slots.sort_by_key(|candle| candle.timestamp);

    for candle in &slots {
        validate_ohlc_invariants(candle, role)?;
    }

    let mut payloads = slots.clone();
    for index in (1..payloads.len()).rev() {
        let swap_index = rng.next_index(index + 1);
        payloads.swap(index, swap_index);
    }

    let permuted = slots
        .into_iter()
        .zip(payloads)
        .map(|(slot, mut payload)| {
            payload.timestamp = slot.timestamp;
            payload.symbol = slot.symbol;
            payload.timeframe = slot.timeframe;
            payload
        })
        .collect();

    Ok(permuted)
}

fn validate_ohlc_invariants(candle: &Candle, role: &str) -> Result<()> {
    let highest_body_price = candle.open.max(candle.close);
    let lowest_body_price = candle.open.min(candle.close);

    if candle.high < highest_body_price || candle.low > lowest_body_price {
        return Err(anyhow!(
            "monte_carlo::candle_permutation {role} candle at {} violates OHLC invariants",
            candle.timestamp
        ));
    }

    Ok(())
}

fn ensure_single_timeframe_synthetic_procedure(
    effective_config: &RuntimeConfig,
    procedure_name: &str,
) -> Result<()> {
    if effective_config.secondary_timeframes.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "{procedure_name} is single-timeframe only; RuntimeConfig contains Secondary Timeframes. Use the future #93 lowest-timeframe reaggregation consistency model instead of independently mutating multiple timeframes."
        ))
    }
}

fn apply_ohlc_noise_to_market_data(
    source: &HistoricalMarketData,
    config: &OhlcNoiseConfigPlanSpec,
    seed: u64,
) -> Result<HistoricalMarketData> {
    if !source.secondary.is_empty() {
        return Err(anyhow!(
            "monte_carlo::ohlc_noise requires single-timeframe source market data; use the future #93 lowest-timeframe reaggregation consistency model for multi-timeframe runs"
        ));
    }

    Ok(HistoricalMarketData::single_timeframe(
        apply_ohlc_noise_to_candles(&source.primary, config, seed, "Primary")?,
    ))
}

fn apply_ohlc_noise_to_candles(
    source: &[Candle],
    config: &OhlcNoiseConfigPlanSpec,
    seed: u64,
    role: &str,
) -> Result<Vec<Candle>> {
    let mut candles = source.to_vec();
    candles.sort_by_key(|candle| candle.timestamp);

    for candle in &candles {
        validate_synthetic_candle_values(candle, "monte_carlo::ohlc_noise", role)?;
    }

    if config.is_effective_noop() {
        return Ok(candles);
    }

    let atr_by_index = trailing_atr_by_candle(&candles, config.atr_period)?;
    if atr_by_index.iter().all(Option::is_none) {
        return Err(anyhow!(
            "monte_carlo::ohlc_noise {role} series has no ATR-scalable candles for atr_period {}; add more history or lower the ATR period",
            config.atr_period
        ));
    }

    let mut rng = SplitMix64::new(seed);
    let mut mutated = candles.clone();

    for (index, candle) in mutated.iter_mut().enumerate() {
        let Some(atr) = atr_by_index[index] else {
            continue;
        };

        if rng.next_unit_f64() >= config.mutation_probability {
            continue;
        }

        let max_delta = atr * config.max_atr_change;
        candle.open += rng.next_centered_f64() * max_delta;
        candle.high += rng.next_centered_f64() * max_delta;
        candle.low += rng.next_centered_f64() * max_delta;
        candle.close += rng.next_centered_f64() * max_delta;
        repair_ohlc_range_to_body(candle);
        validate_synthetic_candle_values(candle, "monte_carlo::ohlc_noise", role)?;
    }

    Ok(mutated)
}

impl OhlcNoiseConfigPlanSpec {
    fn new(mutation_probability: f64, max_atr_change: f64) -> Self {
        Self {
            mutation_probability,
            max_atr_change,
            atr_period: DEFAULT_OHLC_NOISE_ATR_PERIOD,
        }
    }

    fn is_effective_noop(&self) -> bool {
        self.mutation_probability == 0.0 || self.max_atr_change == 0.0
    }
}

fn trailing_atr_by_candle(candles: &[Candle], period: usize) -> Result<Vec<Option<f64>>> {
    if period == 0 {
        return Err(anyhow!("ATR period must be greater than zero"));
    }

    let mut atr_by_index = vec![None; candles.len()];
    if candles.len() < period + 1 {
        return Ok(atr_by_index);
    }

    let true_ranges = candles
        .windows(2)
        .map(|window| {
            let previous = &window[0];
            let current = &window[1];
            (current.high - current.low)
                .max((current.high - previous.close).abs())
                .max((current.low - previous.close).abs())
        })
        .collect::<Vec<_>>();

    let mut atr = true_ranges[..period].iter().sum::<f64>() / period as f64;
    ensure_finite_non_negative_atr(atr, period)?;
    atr_by_index[period] = Some(atr);

    for (offset, true_range) in true_ranges[period..].iter().copied().enumerate() {
        atr = (atr * (period as f64 - 1.0) + true_range) / period as f64;
        ensure_finite_non_negative_atr(atr, period)?;
        atr_by_index[period + 1 + offset] = Some(atr);
    }

    Ok(atr_by_index)
}

fn ensure_finite_non_negative_atr(atr: f64, period: usize) -> Result<()> {
    if atr.is_finite() && atr >= 0.0 {
        Ok(())
    } else {
        Err(anyhow!(
            "monte_carlo::ohlc_noise computed invalid ATR value for atr_period {period}"
        ))
    }
}

fn repair_ohlc_range_to_body(candle: &mut Candle) {
    candle.high = candle.high.max(candle.open).max(candle.close);
    candle.low = candle.low.min(candle.open).min(candle.close);
}

fn validate_synthetic_candle_values(
    candle: &Candle,
    procedure_name: &str,
    role: &str,
) -> Result<()> {
    for (field, value) in [
        ("open", candle.open),
        ("high", candle.high),
        ("low", candle.low),
        ("close", candle.close),
    ] {
        if !value.is_finite() || value <= 0.0 {
            return Err(anyhow!(
                "{procedure_name} {role} candle at {} has non-finite or non-positive {field} value after mutation/repair",
                candle.timestamp
            ));
        }
    }

    let highest_body_price = candle.open.max(candle.close);
    let lowest_body_price = candle.open.min(candle.close);
    if candle.high < highest_body_price || candle.low > lowest_body_price {
        return Err(anyhow!(
            "{procedure_name} {role} candle at {} violates OHLC invariants after mutation/repair",
            candle.timestamp
        ));
    }

    Ok(())
}

fn load_warmup_prefix<F>(
    load_candles: &mut F,
    symbol: &str,
    timeframe: Timeframe,
    before_ms: i64,
    count: usize,
    role: &str,
) -> Result<Vec<Candle>>
where
    F: FnMut(&str, Timeframe, DatasetCandleRequest) -> Result<Vec<Candle>>,
{
    if count == 0 {
        return Ok(Vec::new());
    }

    let mut candles = load_candles(
        symbol,
        timeframe,
        DatasetCandleRequest::WarmupPrefix { before_ms, count },
    )?;
    sort_and_validate_identity(&mut candles, symbol, timeframe, role)?;

    if candles.len() != count {
        return Err(anyhow!(
            "dataset::load {role} warmup for {symbol}/{timeframe} requires {count} candles before {before_ms} but loader returned {}",
            candles.len()
        ));
    }
    if candles.iter().any(|candle| candle.timestamp >= before_ms) {
        return Err(anyhow!(
            "dataset::load {role} warmup for {symbol}/{timeframe} returned candles at or after visible start {before_ms}"
        ));
    }

    Ok(candles)
}

fn load_range<F>(
    load_candles: &mut F,
    symbol: &str,
    timeframe: Timeframe,
    start_ms: i64,
    end_ms: i64,
    role: &str,
) -> Result<Vec<Candle>>
where
    F: FnMut(&str, Timeframe, DatasetCandleRequest) -> Result<Vec<Candle>>,
{
    let mut candles = load_candles(
        symbol,
        timeframe,
        DatasetCandleRequest::Range { start_ms, end_ms },
    )?;
    sort_and_validate_identity(&mut candles, symbol, timeframe, role)?;

    if candles
        .iter()
        .any(|candle| candle.timestamp < start_ms || candle.timestamp >= end_ms)
    {
        return Err(anyhow!(
            "dataset::load {role} range for {symbol}/{timeframe} returned candles outside half-open window [{start_ms}, {end_ms})"
        ));
    }

    Ok(candles)
}

fn sort_and_validate_identity(
    candles: &mut Vec<Candle>,
    symbol: &str,
    timeframe: Timeframe,
    role: &str,
) -> Result<()> {
    if let Some(candle) = candles.iter().find(|candle| candle.symbol != symbol) {
        return Err(anyhow!(
            "dataset::load {role} for {symbol}/{timeframe} returned candle for unexpected symbol '{}'",
            candle.symbol
        ));
    }
    if let Some(candle) = candles.iter().find(|candle| candle.timeframe != timeframe) {
        return Err(anyhow!(
            "dataset::load {role} for {symbol}/{timeframe} returned candle for unexpected timeframe '{}'",
            candle.timeframe
        ));
    }

    candles.sort_by_key(|candle| candle.timestamp);
    Ok(())
}

pub fn render_markdown(report: &PlanReport, strategy_label: &str) -> String {
    let mut out = String::new();
    let title = report.title.as_deref().unwrap_or("Backtest plan report");

    let _ = writeln!(out, "# {title}");
    let _ = writeln!(out);
    let _ = writeln!(out, "- Strategy: `{strategy_label}`");
    let _ = writeln!(out, "- Tests: {}", report.tests.len());

    for (index, test) in report.tests.iter().enumerate() {
        let metrics = test.result.metrics;
        let _ = writeln!(out);
        let _ = writeln!(out, "## {}. {}", index + 1, test.name);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "- Symbol / interval: {} / {}",
            test.symbol, test.interval
        );
        let _ = writeln!(out, "- Initial balance: {:.2}", test.initial_balance);
        let _ = writeln!(out, "- Final equity: {:.2}", metrics.final_equity);
        let _ = writeln!(
            out,
            "- Max drawdown: {:.2} ({:.1}%)",
            metrics.max_drawdown,
            metrics.max_drawdown_pct * 100.0
        );
        let _ = writeln!(out, "- Trades: {}", metrics.trade_count);

        if let Some(synthetic) = &test.synthetic {
            render_synthetic_monte_carlo(&mut out, test, synthetic);
        }
    }

    out
}

fn render_synthetic_monte_carlo(
    out: &mut String,
    test: &BaselinePlanTest,
    synthetic: &SyntheticMonteCarloReport,
) {
    let _ = writeln!(out);
    let _ = writeln!(out, "### Baseline vs synthetic Monte Carlo comparison");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "- Procedure: {}",
        monte_carlo_procedure_label(&synthetic.procedure)
    );
    let _ = writeln!(out, "- Iterations: {}", synthetic.iterations.len());

    if synthetic.iterations.is_empty() {
        let _ = writeln!(out);
        let _ = writeln!(out, "_No synthetic iterations were produced._");
        return;
    }

    let final_equity_summary = metric_summary(
        test.result.metrics.final_equity,
        synthetic
            .iterations
            .iter()
            .map(|iteration| iteration.final_equity),
    );
    let max_drawdown_summary = metric_summary(
        test.result.metrics.max_drawdown,
        synthetic
            .iterations
            .iter()
            .map(|iteration| iteration.max_drawdown),
    );

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| Metric | Baseline | Synthetic p5 | Synthetic p50 | Synthetic p95 | Baseline percentile |"
    );
    let _ = writeln!(out, "|---|---:|---:|---:|---:|---:|");
    render_metric_summary_row(out, "Final equity", final_equity_summary);
    render_metric_summary_row(out, "Max drawdown", max_drawdown_summary);

    let _ = writeln!(out);
    let _ = writeln!(out, "#### Reduced iteration diagnostics");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "| Iteration | Seed | Final equity | Max drawdown | Trades | Blocked Strategy Ticks | Strategy Exits | Risk Exits | Force Closes |"
    );
    let _ = writeln!(out, "|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for iteration in &synthetic.iterations {
        let _ = writeln!(
            out,
            "| {} | {} | {:.2} | {:.2} | {} | {} | {} | {} | {} |",
            iteration.iteration,
            iteration.seed,
            iteration.final_equity,
            iteration.max_drawdown,
            iteration.trade_count,
            iteration.blocked_strategy_tick_count,
            iteration.strategy_exit_count,
            iteration.risk_exit_count,
            iteration.force_close_count,
        );
    }
}

fn monte_carlo_procedure_label(procedure: &MonteCarloProcedure) -> &'static str {
    match procedure {
        MonteCarloProcedure::CandlePermutation => "Candle permutation",
        MonteCarloProcedure::OhlcNoise => "ATR-scaled OHLC noise",
    }
}

#[derive(Debug, Clone, Copy)]
struct MonteCarloMetricSummary {
    baseline: f64,
    p5: f64,
    p50: f64,
    p95: f64,
    baseline_percentile: f64,
}

fn metric_summary(
    baseline: f64,
    synthetic_values: impl Iterator<Item = f64>,
) -> MonteCarloMetricSummary {
    let samples = sorted_metric_samples(synthetic_values);
    MonteCarloMetricSummary {
        baseline,
        p5: interpolated_percentile(&samples, 0.05),
        p50: interpolated_percentile(&samples, 0.50),
        p95: interpolated_percentile(&samples, 0.95),
        baseline_percentile: baseline_percentile(&samples, baseline),
    }
}

fn render_metric_summary_row(out: &mut String, label: &str, summary: MonteCarloMetricSummary) {
    let _ = writeln!(
        out,
        "| {label} | {:.2} | {:.2} | {:.2} | {:.2} | {:.1}% |",
        summary.baseline,
        summary.p5,
        summary.p50,
        summary.p95,
        summary.baseline_percentile * 100.0,
    );
}

fn sorted_metric_samples(synthetic_values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut samples = synthetic_values.collect::<Vec<_>>();
    samples.sort_by(f64::total_cmp);
    samples
}

/// Return a percentile using linear interpolation over sorted synthetic samples.
fn interpolated_percentile(sorted_samples: &[f64], percentile: f64) -> f64 {
    assert!(
        !sorted_samples.is_empty(),
        "percentile requires at least one synthetic sample"
    );
    assert!(
        (0.0..=1.0).contains(&percentile),
        "percentile must be in the inclusive range [0, 1]"
    );

    if sorted_samples.len() == 1 {
        return sorted_samples[0];
    }

    let rank = percentile * (sorted_samples.len() - 1) as f64;
    let lower_index = rank.floor() as usize;
    let upper_index = rank.ceil() as usize;
    let lower = sorted_samples[lower_index];
    let upper = sorted_samples[upper_index];

    if lower_index == upper_index {
        lower
    } else {
        lower + (upper - lower) * (rank - lower_index as f64)
    }
}

/// Return the baseline value's inverse linear position in the synthetic distribution.
fn baseline_percentile(sorted_samples: &[f64], baseline: f64) -> f64 {
    assert!(
        !sorted_samples.is_empty(),
        "baseline percentile requires at least one synthetic sample"
    );

    if sorted_samples.len() == 1 {
        return match baseline.total_cmp(&sorted_samples[0]) {
            std::cmp::Ordering::Less => 0.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Greater => 1.0,
        };
    }

    let first = sorted_samples[0];
    let last = sorted_samples[sorted_samples.len() - 1];
    if first == last {
        return match baseline.total_cmp(&first) {
            std::cmp::Ordering::Less => 0.0,
            std::cmp::Ordering::Equal => 0.5,
            std::cmp::Ordering::Greater => 1.0,
        };
    }
    if baseline <= first {
        return 0.0;
    }
    if baseline >= last {
        return 1.0;
    }

    for (lower_index, window) in sorted_samples.windows(2).enumerate() {
        let lower = window[0];
        let upper = window[1];
        if baseline <= upper {
            let local_fraction = if lower == upper {
                0.0
            } else {
                (baseline - lower) / (upper - lower)
            };
            return (lower_index as f64 + local_fraction) / (sorted_samples.len() - 1) as f64;
        }
    }

    1.0
}

fn parse_plan(plan_src: &str) -> Result<ValidatedPlanResultSpec> {
    let mut rhai = RhaiEngine::new();
    register_plan_api(&mut rhai);
    let ast = compile_plan(&rhai, plan_src)?;
    let mut scope = Scope::new();

    rhai.run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| anyhow!("plan script failed during setup: {e}"))?;

    let result: Dynamic = rhai
        .call_fn(&mut scope, &ast, "plan", ())
        .map_err(|e| anyhow!("plan() failed: {e}"))?;

    let plan_result = result.try_cast::<PlanResultSpec>().ok_or_else(|| {
        anyhow!("plan() must return a typed plan result from `plan_result::new()`")
    })?;

    validate_plan_result(plan_result)
}

fn parse_plan_time(raw: &str) -> Result<PlanTimestamp> {
    if is_date_only(raw) {
        let date = NaiveDate::parse_from_str(raw, "%Y-%m-%d")
            .map_err(|e| anyhow!("Invalid plan time '{}': {e}", raw))?;
        let datetime = date
            .and_hms_opt(0, 0, 0)
            .expect("UTC midnight should be a valid time")
            .and_utc();
        return Ok(PlanTimestamp {
            timestamp_ms: datetime.timestamp_millis(),
        });
    }

    DateTime::parse_from_rfc3339(raw)
        .map(|datetime| PlanTimestamp {
            timestamp_ms: datetime.timestamp_millis(),
        })
        .map_err(|e| {
            anyhow!(
                "Invalid plan time '{}': expected RFC3339 timestamp or date-only YYYY-MM-DD UTC date ({e})",
                raw
            )
        })
}

fn is_date_only(raw: &str) -> bool {
    raw.len() == 10
        && raw.as_bytes()[4] == b'-'
        && raw.as_bytes()[7] == b'-'
        && raw
            .bytes()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit())
}

fn compile_plan(rhai: &RhaiEngine, plan_src: &str) -> Result<AST> {
    let normalized_plan_src = normalize_reserved_constructor_names(plan_src);
    let ast = rhai
        .compile(&normalized_plan_src)
        .map_err(|e| anyhow!("plan script compile error: {e}"))?;

    let has_plan = ast
        .iter_functions()
        .any(|f| f.name == "plan" && f.params.is_empty());
    if !has_plan {
        return Err(anyhow!("plan script must define `fn plan()`"));
    }

    Ok(ast)
}

fn register_plan_api(rhai: &mut RhaiEngine) {
    rhai.register_type_with_name::<PlanResultSpec>("PlanResult");
    rhai.register_type_with_name::<PlanTestSpec>("PlanTest");
    rhai.register_type_with_name::<BaselinePlanSpec>("BaselineRun");
    rhai.register_type_with_name::<SyntheticPlanSpec>("SyntheticMonteCarlo");
    rhai.register_type_with_name::<MonteCarloConfigPlanSpec>("MonteCarloConfig");
    rhai.register_type_with_name::<OhlcNoiseConfigPlanSpec>("OhlcNoiseConfig");
    rhai.register_type_with_name::<DatasetPlanSpec>("Dataset");
    rhai.register_type_with_name::<RunConfigPlanSpec>("RunConfig");
    rhai.register_type_with_name::<PlanTimestamp>("PlanTime");
    rhai.register_fn("time", plan_time);
    rhai.register_fn("__backtester_plan_result_new", PlanResultSpec::new);
    rhai.register_fn("__backtester_plan_test_new", |name: &str| PlanTestSpec {
        name: name.to_string(),
        baseline: None,
        synthetic: None,
    });
    rhai.register_fn("__backtester_run_config_new", RunConfigPlanSpec::new);
    rhai.register_fn(
        "__backtester_monte_carlo_config_new",
        monte_carlo_config_new,
    );
    rhai.register_fn("__backtester_ohlc_noise_config_new", ohlc_noise_config_new);
    rhai.register_fn("with_title", |mut result: PlanResultSpec, title: &str| {
        result.title = Some(title.to_string());
        result
    });
    rhai.register_fn("with_test", with_test);
    rhai.register_fn("with_baseline", with_baseline);
    rhai.register_fn("with_synthetic", with_synthetic);
    rhai.register_fn("with_balance", with_balance_float);
    rhai.register_fn("with_balance", with_balance_int);
    rhai.register_fn("with_atr_period", with_atr_period);

    let mut dataset_module = Module::new();
    dataset_module.set_native_fn("load", dataset_load);
    rhai.register_static_module("dataset", Arc::new(dataset_module));

    let mut baseline_module = Module::new();
    baseline_module.set_native_fn("run", baseline_run);
    rhai.register_static_module("baseline", Arc::new(baseline_module));

    let mut monte_carlo_module = Module::new();
    monte_carlo_module.set_native_fn("candle_permutation", candle_permutation_monte_carlo);
    monte_carlo_module.set_native_fn("ohlc_noise", ohlc_noise_monte_carlo);
    rhai.register_static_module("monte_carlo", Arc::new(monte_carlo_module));
}

fn plan_time(raw: &str) -> std::result::Result<PlanTimestamp, Box<EvalAltResult>> {
    parse_plan_time(raw).map_err(|error| Box::<EvalAltResult>::from(error.to_string()))
}

fn dataset_load(
    symbol: &str,
    start: PlanTimestamp,
    end: PlanTimestamp,
) -> std::result::Result<DatasetPlanSpec, Box<EvalAltResult>> {
    if symbol.trim().is_empty() {
        return Err("dataset::load symbol must not be empty".into());
    }
    if start.timestamp_ms >= end.timestamp_ms {
        return Err(format!(
            "dataset::load start must be before end (got [{}, {}))",
            start.timestamp_ms, end.timestamp_ms
        )
        .into());
    }

    Ok(DatasetPlanSpec {
        symbol: symbol.to_string(),
        start_ms: start.timestamp_ms,
        end_ms: end.timestamp_ms,
    })
}

fn with_balance_float(
    config: RunConfigPlanSpec,
    balance: FLOAT,
) -> std::result::Result<RunConfigPlanSpec, Box<EvalAltResult>> {
    with_balance(config, balance)
}

fn with_balance_int(
    config: RunConfigPlanSpec,
    balance: INT,
) -> std::result::Result<RunConfigPlanSpec, Box<EvalAltResult>> {
    with_balance(config, balance as f64)
}

fn with_balance(
    mut config: RunConfigPlanSpec,
    balance: f64,
) -> std::result::Result<RunConfigPlanSpec, Box<EvalAltResult>> {
    if !balance.is_finite() {
        return Err("run_config.with_balance requires a finite balance".into());
    }
    config.balance = Some(balance);
    Ok(config)
}

fn with_test(
    mut result: PlanResultSpec,
    test: Dynamic,
) -> std::result::Result<PlanResultSpec, Box<EvalAltResult>> {
    let test = test.try_cast::<PlanTestSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "with_test requires a PlanTest host object from `plan_test::new(...)`",
        )
    })?;
    result.tests.push(test);
    Ok(result)
}

fn with_baseline(
    mut test: PlanTestSpec,
    baseline: Dynamic,
) -> std::result::Result<PlanTestSpec, Box<EvalAltResult>> {
    let baseline = baseline.try_cast::<BaselinePlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "with_baseline requires a BaselineRun host object from `baseline::run(...)`",
        )
    })?;
    test.baseline = Some(baseline);
    Ok(test)
}

fn with_synthetic(
    mut test: PlanTestSpec,
    synthetic: Dynamic,
) -> std::result::Result<PlanTestSpec, Box<EvalAltResult>> {
    let synthetic = synthetic.try_cast::<SyntheticPlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "with_synthetic requires a SyntheticMonteCarlo host object from `monte_carlo::*`",
        )
    })?;
    test.synthetic = Some(synthetic);
    Ok(test)
}

fn monte_carlo_config_new(
    iterations: INT,
    base_seed: INT,
) -> std::result::Result<MonteCarloConfigPlanSpec, Box<EvalAltResult>> {
    if iterations <= 0 {
        return Err("monte_carlo_config::new iterations must be greater than zero".into());
    }

    Ok(MonteCarloConfigPlanSpec {
        iterations: usize::try_from(iterations).map_err(|_| {
            Box::<EvalAltResult>::from(
                "monte_carlo_config::new iterations value does not fit this platform",
            )
        })?,
        base_seed: base_seed as u64,
    })
}

fn ohlc_noise_config_new(
    mutation_probability: Dynamic,
    max_atr_change: Dynamic,
) -> std::result::Result<OhlcNoiseConfigPlanSpec, Box<EvalAltResult>> {
    let mutation_probability = dynamic_number_to_f64(
        mutation_probability,
        "ohlc_noise_config::new mutation_probability",
    )?;
    let max_atr_change =
        dynamic_number_to_f64(max_atr_change, "ohlc_noise_config::new max_atr_change")?;

    validate_ohlc_noise_probability(mutation_probability)?;
    validate_ohlc_noise_max_atr_change(max_atr_change)?;

    Ok(OhlcNoiseConfigPlanSpec::new(
        mutation_probability,
        max_atr_change,
    ))
}

fn dynamic_number_to_f64(
    value: Dynamic,
    name: &str,
) -> std::result::Result<f64, Box<EvalAltResult>> {
    if let Some(number) = value.clone().try_cast::<FLOAT>() {
        return Ok(number);
    }
    if let Some(number) = value.try_cast::<INT>() {
        return Ok(number as f64);
    }

    Err(format!("{name} must be a number").into())
}

fn validate_ohlc_noise_probability(
    mutation_probability: f64,
) -> std::result::Result<(), Box<EvalAltResult>> {
    if mutation_probability.is_finite() && (0.0..=1.0).contains(&mutation_probability) {
        Ok(())
    } else {
        Err("ohlc_noise_config::new mutation_probability must be finite and in [0.0, 1.0]".into())
    }
}

fn validate_ohlc_noise_max_atr_change(
    max_atr_change: f64,
) -> std::result::Result<(), Box<EvalAltResult>> {
    if max_atr_change.is_finite() && max_atr_change >= 0.0 {
        Ok(())
    } else {
        Err("ohlc_noise_config::new max_atr_change must be finite and non-negative".into())
    }
}

fn with_atr_period(
    mut config: OhlcNoiseConfigPlanSpec,
    atr_period: INT,
) -> std::result::Result<OhlcNoiseConfigPlanSpec, Box<EvalAltResult>> {
    let atr_period = usize::try_from(atr_period)
        .map_err(|_| "ohlc_noise_config.with_atr_period period must be positive")?;
    if atr_period == 0 {
        return Err("ohlc_noise_config.with_atr_period period must be positive".into());
    }

    config.atr_period = atr_period;
    Ok(config)
}

fn candle_permutation_monte_carlo(
    baseline: Dynamic,
    config: Dynamic,
) -> std::result::Result<SyntheticPlanSpec, Box<EvalAltResult>> {
    let baseline = baseline.try_cast::<BaselinePlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "monte_carlo::candle_permutation requires a BaselineRun host object from `baseline::run(...)`",
        )
    })?;
    let config = config.try_cast::<MonteCarloConfigPlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "monte_carlo::candle_permutation requires a MonteCarloConfig host object from `monte_carlo_config::new(...)`",
        )
    })?;

    Ok(SyntheticPlanSpec {
        procedure: SyntheticProcedurePlanSpec::CandlePermutation,
        baseline,
        config,
    })
}

fn ohlc_noise_monte_carlo(
    baseline: Dynamic,
    config: Dynamic,
    ohlc_noise_config: Dynamic,
) -> std::result::Result<SyntheticPlanSpec, Box<EvalAltResult>> {
    let baseline = baseline.try_cast::<BaselinePlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "monte_carlo::ohlc_noise requires a BaselineRun host object from `baseline::run(...)`",
        )
    })?;
    let config = config.try_cast::<MonteCarloConfigPlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "monte_carlo::ohlc_noise requires a MonteCarloConfig host object from `monte_carlo_config::new(...)`",
        )
    })?;
    let ohlc_noise_config = ohlc_noise_config
        .try_cast::<OhlcNoiseConfigPlanSpec>()
        .ok_or_else(|| {
            Box::<EvalAltResult>::from(
                "monte_carlo::ohlc_noise requires an OhlcNoiseConfig host object from `ohlc_noise_config::new(...)`",
            )
        })?;

    Ok(SyntheticPlanSpec {
        procedure: SyntheticProcedurePlanSpec::OhlcNoise(ohlc_noise_config),
        baseline,
        config,
    })
}

fn baseline_run(
    dataset: Dynamic,
    run_config: Dynamic,
) -> std::result::Result<BaselinePlanSpec, Box<EvalAltResult>> {
    let dataset = dataset.try_cast::<DatasetPlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "baseline::run requires a Dataset host object from `dataset::load(...)`",
        )
    })?;
    let run_config = run_config.try_cast::<RunConfigPlanSpec>().ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "baseline::run requires a RunConfig host object from `run_config::new()`",
        )
    })?;
    let balance = run_config.balance.ok_or_else(|| {
        Box::<EvalAltResult>::from(
            "baseline::run run config must set a balance with `.with_balance(...)`",
        )
    })?;

    Ok(BaselinePlanSpec { dataset, balance })
}

fn validate_plan_result(result: PlanResultSpec) -> Result<ValidatedPlanResultSpec> {
    if result.tests.is_empty() {
        return Err(anyhow!(
            "typed plan result must contain at least one plan test"
        ));
    }

    let mut tests = Vec::with_capacity(result.tests.len());
    for (index, test) in result.tests.into_iter().enumerate() {
        let test_number = index + 1;
        let baseline = test.baseline.ok_or_else(|| {
            anyhow!(
                "plan test {test_number} ('{}') must attach a baseline with `with_baseline(...)`",
                test.name
            )
        })?;
        let synthetic = test.synthetic;
        if let Some(synthetic) = synthetic.as_ref() {
            if synthetic.baseline != baseline {
                return Err(anyhow!(
                    "plan test {test_number} ('{}') synthetic result must be derived from the same baseline attached with `with_baseline(...)`",
                    test.name
                ));
            }
        }
        tests.push(ValidatedPlanTestSpec {
            name: test.name,
            baseline,
            synthetic,
        });
    }

    Ok(ValidatedPlanResultSpec {
        title: result.title,
        tests,
    })
}

fn normalize_reserved_constructor_names(source: &str) -> String {
    // Rhai 1.24 reserves `new` even in module paths such as
    // `plan_result::new()`. Keep the approved plan-facing API and lower only
    // these typed constructors to private host functions before compilation.
    const REPLACEMENTS: [(&str, &str); 5] = [
        ("plan_result::new(", "__backtester_plan_result_new("),
        ("plan_test::new(", "__backtester_plan_test_new("),
        ("run_config::new(", "__backtester_run_config_new("),
        (
            "monte_carlo_config::new(",
            "__backtester_monte_carlo_config_new(",
        ),
        (
            "ohlc_noise_config::new(",
            "__backtester_ohlc_noise_config_new(",
        ),
    ];

    let mut output = String::with_capacity(source.len());
    let mut index = 0;

    while index < source.len() {
        let remaining = &source[index..];

        if let Some((from, to)) = REPLACEMENTS
            .iter()
            .find(|(from, _)| remaining.starts_with(from))
        {
            output.push_str(to);
            index += from.len();
            continue;
        }

        if remaining.starts_with("//") {
            index = copy_until_line_end(source, index, &mut output);
            continue;
        }

        if remaining.starts_with("/*") {
            index = copy_until_block_comment_end(source, index, &mut output);
            continue;
        }

        if remaining.starts_with('"') {
            index = copy_until_string_end(source, index, &mut output);
            continue;
        }

        let character = remaining
            .chars()
            .next()
            .expect("remaining source should contain a character");
        output.push(character);
        index += character.len_utf8();
    }

    output
}

fn copy_until_line_end(source: &str, start: usize, output: &mut String) -> usize {
    let end = source[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(source.len());
    output.push_str(&source[start..end]);
    end
}

fn copy_until_block_comment_end(source: &str, start: usize, output: &mut String) -> usize {
    let end = source[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
        .unwrap_or(source.len());
    output.push_str(&source[start..end]);
    end
}

fn copy_until_string_end(source: &str, start: usize, output: &mut String) -> usize {
    let mut escaped = false;
    let mut end = source.len();

    for (offset, character) in source[start..].char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' => {
                end = start + offset + character.len_utf8();
                break;
            }
            _ => {}
        }
    }

    output.push_str(&source[start..end]);
    end
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY_MS: i64 = 86_400_000;
    const JAN_1_2020: i64 = 1_577_836_800_000;

    fn day(day_of_month: i64) -> i64 {
        JAN_1_2020 + (day_of_month - 1) * DAY_MS
    }

    fn range(start_day: i64, end_day: i64) -> DatasetCandleRequest {
        DatasetCandleRequest::Range {
            start_ms: day(start_day),
            end_ms: day(end_day),
        }
    }

    fn range_ms(start_ms: i64, end_ms: i64) -> DatasetCandleRequest {
        DatasetCandleRequest::Range { start_ms, end_ms }
    }

    fn warmup_prefix(before_ms: i64, count: usize) -> DatasetCandleRequest {
        DatasetCandleRequest::WarmupPrefix { before_ms, count }
    }

    fn make_candle(ts: i64, close: f64) -> Candle {
        candle_for("AAPL", Timeframe::days(1), ts, close)
    }

    fn candle_for(symbol: &str, timeframe: Timeframe, ts: i64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            symbol: symbol.into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            timeframe,
        }
    }

    fn candles() -> Vec<Candle> {
        vec![
            make_candle(day(2), 100.0),
            make_candle(day(3), 101.0),
            make_candle(day(4), 102.0),
        ]
    }

    fn example_candles_for_request(
        symbol: &str,
        timeframe: Timeframe,
        request: DatasetCandleRequest,
    ) -> Vec<Candle> {
        let step = timeframe.duration_ms();

        match request {
            DatasetCandleRequest::WarmupPrefix { before_ms, count } => (0..count)
                .map(|index| {
                    let offset = (count - index) as i64;
                    candle_for(
                        symbol,
                        timeframe,
                        before_ms - offset * step,
                        90.0 + index as f64,
                    )
                })
                .collect(),
            DatasetCandleRequest::Range { start_ms, end_ms } => (0..4)
                .map(|index| (index, start_ms + index as i64 * step))
                .take_while(|(_, timestamp)| *timestamp < end_ms)
                .map(|(index, timestamp)| {
                    candle_for(symbol, timeframe, timestamp, 100.0 + index as f64)
                })
                .collect(),
        }
    }

    const HOLD_STRATEGY: &str = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

    const TYPED_MULTI_TEST_PLAN: &str = r#"
fn plan() {
    let aapl = dataset::load("AAPL", time("2020-01-02"), time("2020-01-05"));
    let msft = dataset::load("MSFT", time("2020-01-02"), time("2020-01-05"));
    let aapl_baseline = baseline::run(aapl, run_config::new().with_balance(10000.0));
    let msft_baseline = baseline::run(msft, run_config::new().with_balance(5000));
    let aapl_test = plan_test::new("AAPL baseline").with_baseline(aapl_baseline);
    let msft_test = plan_test::new("MSFT baseline").with_baseline(msft_baseline);

    plan_result::new()
        .with_title("Smoke test")
        .with_test(aapl_test)
        .with_test(msft_test)
}
"#;

    fn result_with_metrics(
        final_equity: f64,
        max_drawdown: f64,
        trade_count: usize,
    ) -> BacktestResult {
        BacktestResult {
            trades: Vec::new(),
            equity_curve: vec![crate::EquityPoint {
                timestamp: day(2),
                equity: final_equity,
            }],
            metrics: crate::BacktestMetrics {
                trade_count,
                wins: 0,
                losses: 0,
                win_rate: 0.0,
                total_pnl: final_equity - 10_000.0,
                max_drawdown,
                max_drawdown_pct: max_drawdown / 10_000.0,
                final_equity,
                peak_equity: final_equity + max_drawdown,
                cagr: 0.0,
                sharpe: 0.0,
                time_in_market_pct: 0.0,
                years: 0.0,
            },
            benchmark: crate::Benchmark {
                final_equity,
                cagr: 0.0,
                max_drawdown,
                max_drawdown_pct: max_drawdown / 10_000.0,
            },
            final_balance: final_equity,
        }
    }

    fn report_with_synthetic_iterations(
        iterations: Vec<MonteCarloIterationDiagnostics>,
    ) -> PlanReport {
        PlanReport {
            title: Some("Monte Carlo report".to_string()),
            tests: vec![BaselinePlanTest {
                name: "baseline vs candle permutation".to_string(),
                symbol: "AAPL".to_string(),
                interval: "1d".to_string(),
                initial_balance: 10_000.0,
                result: result_with_metrics(10_500.0, 15.0, 7),
                synthetic: Some(SyntheticMonteCarloReport {
                    procedure: MonteCarloProcedure::CandlePermutation,
                    iterations,
                }),
            }],
        }
    }

    #[test]
    fn baseline_example_plan_runs_and_renders_documented_report_shape() {
        let report = execute_plan(
            HOLD_STRATEGY,
            include_str!("../../backtest_plan/plan.rhai"),
            |symbol, timeframe, request| {
                Ok(example_candles_for_request(symbol, timeframe, request))
            },
        )
        .unwrap();

        assert_eq!(report.title.as_deref(), Some("AAPL baseline Backtest Plan"));
        assert_eq!(report.tests.len(), 1);
        let test = &report.tests[0];
        assert_eq!(test.name, "Baseline: AAPL 1d 2021");
        assert_eq!(test.symbol, "AAPL");
        assert_eq!(test.interval, "1d");
        assert!(test.synthetic.is_none());

        let markdown = render_markdown(&report, "strategies/sma_cross.rhai");
        assert!(markdown.contains("# AAPL baseline Backtest Plan"));
        assert!(markdown.contains("- Strategy: `strategies/sma_cross.rhai`"));
        assert!(markdown.contains("## 1. Baseline: AAPL 1d 2021"));
        assert!(markdown.contains("- Symbol / interval: AAPL / 1d"));
    }

    #[test]
    fn monte_carlo_example_plan_runs_and_renders_distribution_section() {
        let report = execute_plan(
            HOLD_STRATEGY,
            include_str!("../../backtest_plan/candle_permutation_monte_carlo.rhai"),
            |symbol, timeframe, request| {
                Ok(example_candles_for_request(symbol, timeframe, request))
            },
        )
        .unwrap();

        assert_eq!(report.title.as_deref(), Some("AAPL candle-path robustness"));
        assert_eq!(report.tests.len(), 1);
        let test = &report.tests[0];
        assert_eq!(test.name, "Synthetic Market Data: candle permutation");
        let synthetic = test
            .synthetic
            .as_ref()
            .expect("Monte Carlo example should attach synthetic results");
        assert_eq!(synthetic.procedure, MonteCarloProcedure::CandlePermutation);
        assert_eq!(synthetic.iterations.len(), 25);

        let markdown = render_markdown(&report, "strategies/sma_cross.rhai");
        assert!(markdown.contains("# AAPL candle-path robustness"));
        assert!(markdown.contains("### Baseline vs synthetic Monte Carlo comparison"));
        assert!(markdown.contains("- Procedure: Candle permutation"));
        assert!(markdown.contains("| Metric | Baseline | Synthetic p5 | Synthetic p50 | Synthetic p95 | Baseline percentile |"));
        assert!(markdown.contains("#### Reduced iteration diagnostics"));
    }

    #[test]
    fn monte_carlo_percentiles_use_sorted_linear_interpolation() {
        let samples = sorted_metric_samples([11_000.0, 9_000.0, 10_000.0].into_iter());

        assert_eq!(interpolated_percentile(&samples, 0.05), 9_100.0);
        assert_eq!(interpolated_percentile(&samples, 0.50), 10_000.0);
        assert_eq!(interpolated_percentile(&samples, 0.95), 10_900.0);
        assert_eq!(baseline_percentile(&samples, 10_500.0), 0.75);
    }

    #[test]
    fn render_markdown_includes_monte_carlo_summary_and_iteration_diagnostics() {
        let report = report_with_synthetic_iterations(vec![
            MonteCarloIterationDiagnostics {
                iteration: 1,
                seed: 111,
                final_equity: 9_000.0,
                max_drawdown: 10.0,
                trade_count: 1,
                blocked_strategy_tick_count: 0,
                strategy_exit_count: 1,
                risk_exit_count: 0,
                force_close_count: 0,
            },
            MonteCarloIterationDiagnostics {
                iteration: 2,
                seed: 222,
                final_equity: 10_000.0,
                max_drawdown: 20.0,
                trade_count: 2,
                blocked_strategy_tick_count: 1,
                strategy_exit_count: 3,
                risk_exit_count: 4,
                force_close_count: 5,
            },
            MonteCarloIterationDiagnostics {
                iteration: 3,
                seed: 333,
                final_equity: 11_000.0,
                max_drawdown: 30.0,
                trade_count: 3,
                blocked_strategy_tick_count: 2,
                strategy_exit_count: 0,
                risk_exit_count: 1,
                force_close_count: 0,
            },
        ]);

        let markdown = render_markdown(&report, "strategies/test.rhai");

        assert!(markdown.contains("### Baseline vs synthetic Monte Carlo comparison"));
        assert!(markdown.contains("- Procedure: Candle permutation"));
        assert!(markdown.contains("| Metric | Baseline | Synthetic p5 | Synthetic p50 | Synthetic p95 | Baseline percentile |"));
        assert!(markdown
            .contains("| Final equity | 10500.00 | 9100.00 | 10000.00 | 10900.00 | 75.0% |"));
        assert!(markdown.contains("| Max drawdown | 15.00 | 11.00 | 20.00 | 29.00 | 25.0% |"));
        assert!(markdown.contains("#### Reduced iteration diagnostics"));
        assert!(markdown.contains("| Iteration | Seed | Final equity | Max drawdown | Trades | Blocked Strategy Ticks | Strategy Exits | Risk Exits | Force Closes |"));
        assert!(markdown.contains("| 2 | 222 | 10000.00 | 20.00 | 2 | 1 | 3 | 4 | 5 |"));
    }

    #[test]
    fn typed_plan_result_renders_multiple_tests_in_insertion_order() {
        let mut requests = Vec::new();
        let report = execute_plan(
            HOLD_STRATEGY,
            TYPED_MULTI_TEST_PLAN,
            |symbol, timeframe, window| {
                requests.push((symbol.to_string(), timeframe, window));
                Ok(candles()
                    .into_iter()
                    .map(|mut candle| {
                        candle.symbol = symbol.to_string();
                        candle.timeframe = timeframe;
                        candle
                    })
                    .collect())
            },
        )
        .unwrap();

        assert_eq!(
            requests,
            vec![
                ("AAPL".to_string(), Timeframe::days(1), range(2, 5)),
                ("MSFT".to_string(), Timeframe::days(1), range(2, 5)),
            ]
        );
        assert_eq!(report.tests.len(), 2);
        assert_eq!(report.tests[0].name, "AAPL baseline");
        assert_eq!(report.tests[1].name, "MSFT baseline");

        let markdown = render_markdown(&report, "strategies/test.rhai");
        assert!(markdown.contains("# Smoke test"));
        assert!(markdown.contains("## 1. AAPL baseline"));
        assert!(markdown.contains("## 2. MSFT baseline"));
        assert!(
            markdown.find("## 1. AAPL baseline").unwrap()
                < markdown.find("## 2. MSFT baseline").unwrap()
        );
        assert!(markdown.contains("- Strategy: `strategies/test.rhai`"));
        assert!(markdown.contains("- Final equity:"));
    }

    #[test]
    fn dataset_load_accepts_rfc3339_and_date_only_half_open_window() {
        let mut requests = Vec::new();
        let report = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load(
        "AAPL",
        time("2020-01-02T01:00:00+01:00"),
        time("2020-01-04"),
    );

    plan_result::new()
        .with_test(
            plan_test::new("half-open")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |symbol, timeframe, requested_window| {
                requests.push((symbol.to_string(), timeframe, requested_window));
                Ok(vec![
                    candle_for(symbol, timeframe, day(2), 100.0),
                    candle_for(symbol, timeframe, day(3), 101.0),
                ])
            },
        )
        .unwrap();

        assert_eq!(
            requests,
            vec![("AAPL".to_string(), Timeframe::days(1), range(2, 4))]
        );
        assert_eq!(report.tests[0].result.equity_curve.len(), 2);
    }

    #[test]
    fn dataset_loader_fetches_strategy_declared_secondary_timeframes() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let strategy = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::optional(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let mut requests = Vec::new();

        execute_plan(
            strategy,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-01"), time("2020-01-02"));

    plan_result::new()
        .with_test(
            plan_test::new("secondary context")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |symbol, timeframe, requested_window| {
                requests.push((symbol.to_string(), timeframe, requested_window));
                Ok(match timeframe {
                    tf if tf == primary => vec![
                        candle_for(symbol, primary, day(1), 100.0),
                        candle_for(symbol, primary, day(1) + 60_000, 101.0),
                    ],
                    tf if tf == secondary => {
                        vec![candle_for(symbol, secondary, day(1), 200.0)]
                    }
                    _ => Vec::new(),
                })
            },
        )
        .unwrap();

        assert_eq!(
            requests,
            vec![
                ("AAPL".to_string(), primary, range(1, 2)),
                (
                    "AAPL".to_string(),
                    secondary,
                    range_ms(day(1), day(1) + 60_000 + 1),
                ),
            ]
        );
    }

    #[test]
    fn warmup_aware_dataset_loads_hidden_primary_prefix_before_visible_window() {
        let strategy = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1d"))
        .with_minimum_warmup(2)
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let plan = r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-03"), time("2020-01-05"));

    plan_result::new()
        .with_test(
            plan_test::new("warmup-aware")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#;
        let mut requests = Vec::new();

        let report = execute_plan(strategy, plan, |symbol, timeframe, request| {
            requests.push((symbol.to_string(), timeframe, request));
            Ok(match request {
                DatasetCandleRequest::WarmupPrefix { .. } => vec![
                    candle_for(symbol, timeframe, day(1), 98.0),
                    candle_for(symbol, timeframe, day(2), 99.0),
                ],
                DatasetCandleRequest::Range { .. } => vec![
                    candle_for(symbol, timeframe, day(3), 100.0),
                    candle_for(symbol, timeframe, day(4), 101.0),
                ],
            })
        })
        .unwrap();

        assert_eq!(
            requests,
            vec![
                (
                    "AAPL".to_string(),
                    Timeframe::days(1),
                    warmup_prefix(day(3), 2),
                ),
                ("AAPL".to_string(), Timeframe::days(1), range(3, 5)),
            ]
        );
        assert_eq!(
            report
                .tests
                .first()
                .unwrap()
                .result
                .equity_curve
                .iter()
                .map(|point| point.timestamp)
                .collect::<Vec<_>>(),
            vec![day(3), day(4)]
        );

        let ordinary = crate::run_runtime_backtest(
            strategy,
            HistoricalMarketData::single_timeframe(vec![
                make_candle(day(1), 98.0),
                make_candle(day(2), 99.0),
                make_candle(day(3), 100.0),
                make_candle(day(4), 101.0),
            ]),
            RuntimeBacktestConfig::new("AAPL", 10000.0),
        )
        .unwrap();
        assert_eq!(
            report.tests[0].result.equity_curve.len(),
            ordinary.result.equity_curve.len()
        );
        assert_eq!(
            report.tests[0].result.metrics.final_equity,
            ordinary.result.metrics.final_equity
        );
    }

    #[test]
    fn warmup_aware_dataset_loads_secondary_prefix_and_context_to_last_visible_primary() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let start = day(1) + 60_000;
        let last_primary = start + 60_000;
        let strategy = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(1)
        .with_secondary(secondary::optional(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let plan = r#"
fn plan() {
    let dataset = dataset::load(
        "AAPL",
        time("2020-01-01T00:01:00Z"),
        time("2020-01-01T00:03:00Z")
    );

    plan_result::new()
        .with_test(
            plan_test::new("secondary warmup")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#;
        let mut requests = Vec::new();

        execute_plan(strategy, plan, |symbol, timeframe, request| {
            requests.push((symbol.to_string(), timeframe, request));
            Ok(match (timeframe, request) {
                (tf, DatasetCandleRequest::WarmupPrefix { .. }) if tf == primary => {
                    vec![candle_for(symbol, primary, day(1), 99.0)]
                }
                (tf, DatasetCandleRequest::Range { .. }) if tf == primary => vec![
                    candle_for(symbol, primary, start, 100.0),
                    candle_for(symbol, primary, last_primary, 101.0),
                ],
                (tf, DatasetCandleRequest::WarmupPrefix { .. }) if tf == secondary => {
                    vec![candle_for(symbol, secondary, day(1), 200.0)]
                }
                (tf, DatasetCandleRequest::Range { .. }) if tf == secondary => {
                    vec![candle_for(symbol, secondary, start, 201.0)]
                }
                _ => Vec::new(),
            })
        })
        .unwrap();

        assert_eq!(
            requests,
            vec![
                ("AAPL".to_string(), primary, warmup_prefix(start, 1)),
                (
                    "AAPL".to_string(),
                    primary,
                    range_ms(start, day(1) + 180_000)
                ),
                ("AAPL".to_string(), secondary, warmup_prefix(start, 1)),
                (
                    "AAPL".to_string(),
                    secondary,
                    range_ms(start, last_primary + 1),
                ),
            ]
        );
    }

    #[test]
    fn insufficient_primary_or_secondary_warmup_history_fails_before_execution() {
        let primary_missing = execute_plan(
            r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1d"))
        .with_minimum_warmup(2)
}

fn on_tick(market, context) {
    decision::hold()
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-03"), time("2020-01-04"));

    plan_result::new()
        .with_test(
            plan_test::new("missing primary warmup")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |symbol, timeframe, request| {
                Ok(match request {
                    DatasetCandleRequest::WarmupPrefix { .. } => {
                        vec![candle_for(symbol, timeframe, day(2), 99.0)]
                    }
                    DatasetCandleRequest::Range { .. } => {
                        vec![candle_for(symbol, timeframe, day(3), 100.0)]
                    }
                })
            },
        )
        .unwrap_err();
        let msg = primary_missing.to_string();
        assert!(msg.contains("Primary warmup"));
        assert!(msg.contains("requires 2 candles before"));

        let secondary_missing = execute_plan(
            r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(1)
        .with_secondary(secondary::optional(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-01T00:01:00Z"), time("2020-01-01T00:02:00Z"));

    plan_result::new()
        .with_test(
            plan_test::new("missing secondary warmup")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |symbol, timeframe, request| {
                Ok(match (timeframe, request) {
                    (tf, DatasetCandleRequest::WarmupPrefix { .. })
                        if tf == Timeframe::minutes(1) =>
                    {
                        vec![candle_for(symbol, timeframe, day(1), 99.0)]
                    }
                    (tf, DatasetCandleRequest::Range { .. }) if tf == Timeframe::minutes(1) => {
                        vec![candle_for(symbol, timeframe, day(1) + 60_000, 100.0)]
                    }
                    (tf, DatasetCandleRequest::WarmupPrefix { .. })
                        if tf == Timeframe::hours(1) =>
                    {
                        Vec::new()
                    }
                    _ => Vec::new(),
                })
            },
        )
        .unwrap_err();
        let msg = secondary_missing.to_string();
        assert!(msg.contains("Secondary warmup"));
        assert!(msg.contains("requires 1 candles before"));
    }

    #[test]
    fn empty_visible_primary_window_fails_clearly() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-03"));

    plan_result::new()
        .with_test(
            plan_test::new("empty")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(Vec::new()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("dataset::load"));
        assert!(msg.contains("visible Primary window"));
        assert!(msg.contains("contains no candles"));
    }

    #[test]
    fn candle_permutation_monte_carlo_plan_runs_runtime_backed_iterations() {
        let report = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-05"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::candle_permutation(
        baseline,
        monte_carlo_config::new(2, 42)
    );

    plan_result::new()
        .with_test(
            plan_test::new("baseline vs candle permutation")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#,
            |symbol, timeframe, _window| {
                Ok(candles()
                    .into_iter()
                    .map(|mut candle| {
                        candle.symbol = symbol.to_string();
                        candle.timeframe = timeframe;
                        candle
                    })
                    .collect())
            },
        )
        .unwrap();

        let synthetic = report.tests[0]
            .synthetic
            .as_ref()
            .expect("test should include a synthetic Monte Carlo result");
        assert_eq!(synthetic.procedure, MonteCarloProcedure::CandlePermutation);
        assert_eq!(synthetic.iterations.len(), 2);
        assert_ne!(synthetic.iterations[0].seed, synthetic.iterations[1].seed);
        assert_eq!(synthetic.iterations[0].iteration, 1);
        assert_eq!(synthetic.iterations[0].final_equity, 10000.0);
        assert_eq!(synthetic.iterations[0].trade_count, 0);
        assert_eq!(synthetic.iterations[0].blocked_strategy_tick_count, 0);
        assert_eq!(synthetic.iterations[0].strategy_exit_count, 0);
        assert_eq!(synthetic.iterations[0].risk_exit_count, 0);
        assert_eq!(synthetic.iterations[0].force_close_count, 0);
    }

    #[test]
    fn ohlc_noise_monte_carlo_plan_runs_runtime_backed_iterations() {
        let report = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-06"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::ohlc_noise(
        baseline,
        monte_carlo_config::new(2, 42),
        ohlc_noise_config::new(0.0, 0.25).with_atr_period(2)
    );

    plan_result::new()
        .with_test(
            plan_test::new("baseline vs ohlc noise")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#,
            |symbol, timeframe, _window| {
                Ok(vec![
                    candle_for(symbol, timeframe, day(2), 100.0),
                    candle_for(symbol, timeframe, day(3), 101.0),
                    candle_for(symbol, timeframe, day(4), 102.0),
                    candle_for(symbol, timeframe, day(5), 103.0),
                ])
            },
        )
        .unwrap();

        let synthetic = report.tests[0]
            .synthetic
            .as_ref()
            .expect("test should include a synthetic Monte Carlo result");
        assert_eq!(synthetic.procedure, MonteCarloProcedure::OhlcNoise);
        assert_eq!(synthetic.iterations.len(), 2);
        assert_ne!(synthetic.iterations[0].seed, synthetic.iterations[1].seed);
        assert_eq!(synthetic.iterations[0].iteration, 1);
        assert_eq!(synthetic.iterations[0].final_equity, 10000.0);
        assert_eq!(synthetic.iterations[0].trade_count, 0);
    }

    #[test]
    fn monte_carlo_diagnostics_count_runtime_exit_events() {
        let plan = r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-05"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::candle_permutation(baseline, monte_carlo_config::new(1, 7));

    plan_result::new()
        .with_test(
            plan_test::new("diagnostics")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#;

        let strategy_exit_report = execute_plan(
            r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    let seen = context.state.get("seen", 0);
    context.state.set("seen", seen + 1);

    if seen == 0 {
        decision::open_long(1.0)
    } else {
        decision::close_long()
    }
}
"#,
            plan,
            |symbol, timeframe, _window| {
                Ok(candles()
                    .into_iter()
                    .map(|mut candle| {
                        candle.symbol = symbol.to_string();
                        candle.timeframe = timeframe;
                        candle
                    })
                    .collect())
            },
        )
        .unwrap();
        let strategy_exit_iteration = &strategy_exit_report.tests[0]
            .synthetic
            .as_ref()
            .unwrap()
            .iterations[0];
        assert_eq!(strategy_exit_iteration.strategy_exit_count, 1);
        assert_eq!(strategy_exit_iteration.risk_exit_count, 0);
        assert_eq!(strategy_exit_iteration.force_close_count, 0);

        let risk_exit_report = execute_plan(
            r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    let seen = context.state.get("seen", 0);
    context.state.set("seen", seen + 1);

    if seen == 0 {
        decision::open_long(1.0).with_take_profit(100.5)
    } else {
        decision::hold()
    }
}
"#,
            plan,
            |symbol, timeframe, _window| {
                Ok(candles()
                    .into_iter()
                    .map(|mut candle| {
                        candle.symbol = symbol.to_string();
                        candle.timeframe = timeframe;
                        candle.close = 100.0;
                        candle.open = 100.0;
                        candle.high = 101.0;
                        candle.low = 99.0;
                        candle
                    })
                    .collect())
            },
        )
        .unwrap();
        let risk_exit_iteration = &risk_exit_report.tests[0]
            .synthetic
            .as_ref()
            .unwrap()
            .iterations[0];
        assert_eq!(risk_exit_iteration.strategy_exit_count, 0);
        assert_eq!(risk_exit_iteration.risk_exit_count, 1);
        assert_eq!(risk_exit_iteration.trade_count, 1);
    }

    #[test]
    fn monte_carlo_diagnostics_count_blocked_strategy_ticks() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let report = execute_plan(
            r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    decision::open_long(1.0)
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-01"), time("2020-01-02"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::candle_permutation(baseline, monte_carlo_config::new(1, 9));

    plan_result::new()
        .with_test(
            plan_test::new("blocked diagnostics")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#,
            |_symbol, timeframe, _window| match timeframe {
                tf if tf == primary => Ok(vec![
                    candle_for("AAPL", primary, day(1), 100.0),
                    candle_for("AAPL", primary, day(1) + 60_000, 101.0),
                ]),
                tf if tf == secondary => Ok(Vec::new()),
                _ => Ok(Vec::new()),
            },
        )
        .unwrap();

        let iteration = &report.tests[0].synthetic.as_ref().unwrap().iterations[0];
        assert_eq!(iteration.blocked_strategy_tick_count, 2);
        assert_eq!(iteration.trade_count, 0);
    }

    #[test]
    fn candle_permutation_preserves_population_and_chronological_timestamp_slots() {
        let primary = Timeframe::days(1);
        let secondary = Timeframe::hours(1);
        let source = HistoricalMarketData::with_secondary(
            vec![
                candle_for("AAPL", primary, day(2), 10.0),
                candle_for("AAPL", primary, day(3), 20.0),
                candle_for("AAPL", primary, day(4), 30.0),
            ],
            [HistoricalCandleSeries {
                timeframe: secondary,
                candles: vec![
                    candle_for("AAPL", secondary, day(2), 100.0),
                    candle_for("AAPL", secondary, day(4), 200.0),
                ],
            }],
        );
        let seed = derive_monte_carlo_seed(42, 0, 0, CANDLE_PERMUTATION_PROCEDURE_ID);
        assert_eq!(
            seed,
            derive_monte_carlo_seed(42, 0, 0, CANDLE_PERMUTATION_PROCEDURE_ID)
        );
        assert_ne!(
            seed,
            derive_monte_carlo_seed(42, 1, 0, CANDLE_PERMUTATION_PROCEDURE_ID)
        );

        let permuted = permute_market_data(&source, seed).unwrap();

        assert_eq!(
            permuted
                .primary
                .iter()
                .map(|candle| candle.timestamp)
                .collect::<Vec<_>>(),
            vec![day(2), day(3), day(4)]
        );
        let mut primary_closes = permuted
            .primary
            .iter()
            .map(|candle| candle.close as i64)
            .collect::<Vec<_>>();
        primary_closes.sort();
        assert_eq!(primary_closes, vec![10, 20, 30]);
        assert!(permuted
            .primary
            .iter()
            .all(|candle| candle.symbol == "AAPL" && candle.timeframe == primary));

        let secondary = &permuted.secondary[0];
        assert_eq!(
            secondary
                .candles
                .iter()
                .map(|candle| candle.timestamp)
                .collect::<Vec<_>>(),
            vec![day(2), day(4)]
        );
        let mut secondary_closes = secondary
            .candles
            .iter()
            .map(|candle| candle.close as i64)
            .collect::<Vec<_>>();
        secondary_closes.sort();
        assert_eq!(secondary_closes, vec![100, 200]);
    }

    #[test]
    fn ohlc_noise_zero_noise_is_exact_noop_and_seed_deterministic() {
        let source = HistoricalMarketData::single_timeframe(vec![
            candle_for("AAPL", Timeframe::days(1), day(2), 100.0),
            candle_for("AAPL", Timeframe::days(1), day(3), 101.0),
            candle_for("AAPL", Timeframe::days(1), day(4), 102.0),
            candle_for("AAPL", Timeframe::days(1), day(5), 103.0),
        ]);
        let seed = derive_monte_carlo_seed(42, 0, 0, OHLC_NOISE_PROCEDURE_ID);
        assert_eq!(
            seed,
            derive_monte_carlo_seed(42, 0, 0, OHLC_NOISE_PROCEDURE_ID)
        );
        assert_ne!(
            seed,
            derive_monte_carlo_seed(42, 1, 0, OHLC_NOISE_PROCEDURE_ID)
        );

        let probability_zero = OhlcNoiseConfigPlanSpec {
            mutation_probability: 0.0,
            max_atr_change: 0.75,
            atr_period: 2,
        };
        let max_change_zero = OhlcNoiseConfigPlanSpec {
            mutation_probability: 1.0,
            max_atr_change: 0.0,
            atr_period: 2,
        };

        assert_eq!(
            apply_ohlc_noise_to_market_data(&source, &probability_zero, seed)
                .unwrap()
                .primary,
            source.primary
        );
        assert_eq!(
            apply_ohlc_noise_to_market_data(&source, &max_change_zero, seed)
                .unwrap()
                .primary,
            source.primary
        );
        assert_eq!(
            apply_ohlc_noise_to_market_data(&source, &probability_zero, seed)
                .unwrap()
                .primary,
            apply_ohlc_noise_to_market_data(&source, &probability_zero, seed)
                .unwrap()
                .primary
        );
    }

    #[test]
    fn ohlc_noise_mutates_scalable_candles_preserves_identity_and_repairs_invariants() {
        let timeframe = Timeframe::days(1);
        let source = vec![
            candle_for("AAPL", timeframe, day(1), 100.0),
            candle_for("AAPL", timeframe, day(2), 101.0),
            candle_for("AAPL", timeframe, day(3), 102.0),
            candle_for("AAPL", timeframe, day(4), 103.0),
        ];
        let config = OhlcNoiseConfigPlanSpec {
            mutation_probability: 1.0,
            max_atr_change: 0.5,
            atr_period: 2,
        };

        let seed = derive_monte_carlo_seed(7, 0, 0, OHLC_NOISE_PROCEDURE_ID);
        let mutated = apply_ohlc_noise_to_candles(&source, &config, seed, "Primary").unwrap();
        let mutated_again = apply_ohlc_noise_to_candles(&source, &config, seed, "Primary").unwrap();

        assert_eq!(mutated, mutated_again);
        assert_eq!(mutated[0], source[0]);
        assert_eq!(mutated[1], source[1]);
        assert!(mutated[2..].iter().zip(&source[2..]).any(|(left, right)| {
            left.open != right.open
                || left.high != right.high
                || left.low != right.low
                || left.close != right.close
        }));
        for (mutated, original) in mutated.iter().zip(source.iter()) {
            assert_eq!(mutated.timestamp, original.timestamp);
            assert_eq!(mutated.symbol, original.symbol);
            assert_eq!(mutated.timeframe, original.timeframe);
            assert_eq!(mutated.volume, original.volume);
            assert!(mutated.high >= mutated.open.max(mutated.close));
            assert!(mutated.low <= mutated.open.min(mutated.close));
            assert!(mutated.open >= mutated.low && mutated.open <= mutated.high);
            assert!(mutated.close >= mutated.low && mutated.close <= mutated.high);
        }
    }

    #[test]
    fn ohlc_noise_atr_is_trailing_wilder_series_without_future_lookahead() {
        let timeframe = Timeframe::days(1);
        let candles = vec![
            Candle {
                timestamp: day(1),
                symbol: "AAPL".to_string(),
                open: 10.0,
                high: 11.0,
                low: 9.0,
                close: 10.0,
                volume: 1000.0,
                timeframe,
            },
            Candle {
                timestamp: day(2),
                symbol: "AAPL".to_string(),
                open: 12.0,
                high: 14.0,
                low: 12.0,
                close: 13.0,
                volume: 1000.0,
                timeframe,
            },
            Candle {
                timestamp: day(3),
                symbol: "AAPL".to_string(),
                open: 17.0,
                high: 18.0,
                low: 17.0,
                close: 17.5,
                volume: 1000.0,
                timeframe,
            },
            Candle {
                timestamp: day(4),
                symbol: "AAPL".to_string(),
                open: 50.0,
                high: 100.0,
                low: 1.0,
                close: 50.0,
                volume: 1000.0,
                timeframe,
            },
        ];

        let atr = trailing_atr_by_candle(&candles, 2).unwrap();

        assert_eq!(atr[0], None);
        assert_eq!(atr[1], None);
        assert_eq!(atr[2], Some(4.5));
        assert_eq!(atr[3], Some(51.75));
    }

    #[test]
    fn ohlc_repair_expands_range_to_contain_mutated_body() {
        let mut candle = candle_for("AAPL", Timeframe::days(1), day(1), 10.0);
        candle.open = 12.0;
        candle.high = 9.0;
        candle.low = 11.0;
        candle.close = 10.0;

        repair_ohlc_range_to_body(&mut candle);

        assert_eq!(candle.high, 12.0);
        assert_eq!(candle.low, 10.0);
        validate_synthetic_candle_values(&candle, "monte_carlo::ohlc_noise", "Primary").unwrap();
    }

    #[test]
    fn ohlc_noise_config_validation_errors_are_clear() {
        let invalid_probability =
            ohlc_noise_config_new(Dynamic::from(1.1_f64), Dynamic::from(0.1_f64))
                .unwrap_err()
                .to_string();
        assert!(invalid_probability.contains("mutation_probability"));
        assert!(invalid_probability.contains("[0.0, 1.0]"));

        let non_finite_probability =
            ohlc_noise_config_new(Dynamic::from(f64::INFINITY), Dynamic::from(0.1_f64))
                .unwrap_err()
                .to_string();
        assert!(non_finite_probability.contains("finite"));

        let negative_change =
            ohlc_noise_config_new(Dynamic::from(0.5_f64), Dynamic::from(-0.1_f64))
                .unwrap_err()
                .to_string();
        assert!(negative_change.contains("max_atr_change"));
        assert!(negative_change.contains("non-negative"));

        let non_finite_change =
            ohlc_noise_config_new(Dynamic::from(0.5_f64), Dynamic::from(f64::NAN))
                .unwrap_err()
                .to_string();
        assert!(non_finite_change.contains("finite"));

        let invalid_period = with_atr_period(OhlcNoiseConfigPlanSpec::new(0.5, 0.1), 0)
            .unwrap_err()
            .to_string();
        assert!(invalid_period.contains("period"));
        assert!(invalid_period.contains("positive"));
    }

    #[test]
    fn ohlc_noise_requires_scalable_candles_for_effective_noise() {
        let source = vec![
            candle_for("AAPL", Timeframe::days(1), day(1), 100.0),
            candle_for("AAPL", Timeframe::days(1), day(2), 101.0),
        ];
        let config = OhlcNoiseConfigPlanSpec {
            mutation_probability: 1.0,
            max_atr_change: 0.25,
            atr_period: 2,
        };

        let err = apply_ohlc_noise_to_candles(&source, &config, 42, "Primary").unwrap_err();

        assert!(err.to_string().contains("no ATR-scalable candles"));
    }

    #[test]
    fn ohlc_noise_allows_zero_atr_scaling_without_resampling_or_clamping() {
        let timeframe = Timeframe::days(1);
        let source = (1..=3)
            .map(|day_of_month| Candle {
                timestamp: day(day_of_month),
                symbol: "AAPL".to_string(),
                open: 100.0,
                high: 100.0,
                low: 100.0,
                close: 100.0,
                volume: 1000.0,
                timeframe,
            })
            .collect::<Vec<_>>();
        let config = OhlcNoiseConfigPlanSpec {
            mutation_probability: 1.0,
            max_atr_change: 0.25,
            atr_period: 1,
        };

        let mutated = apply_ohlc_noise_to_candles(&source, &config, 42, "Primary").unwrap();

        assert_eq!(mutated, source);
    }

    #[test]
    fn ohlc_noise_rejects_non_finite_or_non_positive_output_values() {
        let mut candle = candle_for("AAPL", Timeframe::days(1), day(1), 10.0);
        candle.open = 0.0;

        let err = validate_synthetic_candle_values(&candle, "monte_carlo::ohlc_noise", "Primary")
            .unwrap_err();

        assert!(err.to_string().contains("non-finite or non-positive open"));
    }

    #[test]
    fn ohlc_noise_rejects_multi_timeframe_configs_until_reaggregation_issue_93() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let err = execute_plan(
            r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::optional(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-01"), time("2020-01-02"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::ohlc_noise(
        baseline,
        monte_carlo_config::new(1, 42),
        ohlc_noise_config::new(0.5, 0.25).with_atr_period(2)
    );

    plan_result::new()
        .with_test(
            plan_test::new("multi-timeframe noise")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#,
            |_symbol, timeframe, _window| match timeframe {
                tf if tf == primary => Ok(vec![
                    candle_for("AAPL", primary, day(1), 100.0),
                    candle_for("AAPL", primary, day(1) + 60_000, 101.0),
                ]),
                tf if tf == secondary => Ok(vec![candle_for("AAPL", secondary, day(1), 200.0)]),
                _ => Ok(Vec::new()),
            },
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("monte_carlo::ohlc_noise"));
        assert!(msg.contains("single-timeframe"));
        assert!(msg.contains("#93"));
    }

    #[test]
    fn synthetic_monte_carlo_host_object_is_opaque_to_rhai() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-05"));
    let baseline = baseline::run(dataset, run_config::new().with_balance(10000.0));
    let synthetic = monte_carlo::candle_permutation(baseline, monte_carlo_config::new(1, 42));
    let leaked = synthetic.iterations;

    plan_result::new()
        .with_test(
            plan_test::new("leak")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("plan()"));
        assert!(msg.contains("iterations"));
    }

    #[test]
    fn dataset_host_object_is_opaque_to_rhai() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-03"));
    let leaked = dataset.candles;

    plan_result::new()
        .with_test(
            plan_test::new("leak")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("plan()"));
        assert!(msg.contains("candles"));
    }

    #[test]
    fn missing_plan_function_fails_clearly() {
        let err = execute_plan(
            HOLD_STRATEGY,
            "let x = 1;",
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        assert!(err.to_string().contains("fn plan()"));
    }

    #[test]
    fn raw_map_plan_output_is_rejected_as_transitional_shape() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    #{ tests: [#{ name: "legacy", symbol: "AAPL", interval: "1d", balance: 10000.0 }] }
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        assert!(err.to_string().contains("typed plan result"));
    }

    #[test]
    fn plan_test_without_baseline_fails_with_test_identity() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    plan_result::new()
        .with_test(plan_test::new("missing baseline"))
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("plan test 1"));
        assert!(msg.contains("missing baseline"));
        assert!(msg.contains("with_baseline"));
    }

    #[test]
    fn wrong_baseline_host_object_type_fails_clearly() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    plan_result::new()
        .with_test(
            plan_test::new("wrong baseline")
                .with_baseline(plan_test::new("not a baseline"))
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("with_baseline"));
        assert!(msg.contains("BaselineRun"));
    }

    #[test]
    fn baseline_run_requires_dataset_and_explicit_balance() {
        let wrong_dataset = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    plan_result::new()
        .with_test(
            plan_test::new("wrong dataset")
                .with_baseline(baseline::run(plan_test::new("not a dataset"), run_config::new().with_balance(10000.0)))
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();
        assert!(wrong_dataset.to_string().contains("Dataset host object"));

        let missing_balance = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", time("2020-01-02"), time("2020-01-03"));

    plan_result::new()
        .with_test(
            plan_test::new("missing balance")
                .with_baseline(baseline::run(dataset, run_config::new()))
        )
}
"#,
            |_symbol, _timeframe, _window| Ok(candles()),
        )
        .unwrap_err();
        assert!(missing_balance.to_string().contains("with_balance"));
    }

    #[test]
    fn plan_execution_fails_fast_with_failing_test_identity() {
        let mut requests = Vec::new();
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let first = dataset::load("AAPL", time("2020-01-02"), time("2020-01-05"));
    let broken = dataset::load("BROKEN", time("2020-01-02"), time("2020-01-05"));
    let should_not_run = dataset::load("MSFT", time("2020-01-02"), time("2020-01-05"));
    let first_baseline = baseline::run(first, run_config::new().with_balance(10000.0));
    let broken_baseline = baseline::run(broken, run_config::new().with_balance(10000.0));
    let should_not_run_baseline = baseline::run(should_not_run, run_config::new().with_balance(10000.0));

    plan_result::new()
        .with_test(plan_test::new("first").with_baseline(first_baseline))
        .with_test(plan_test::new("broken").with_baseline(broken_baseline))
        .with_test(plan_test::new("should not run").with_baseline(should_not_run_baseline))
}
"#,
            |symbol, _timeframe, _window| {
                requests.push(symbol.to_string());
                if symbol == "BROKEN" {
                    Err(anyhow::anyhow!("loader exploded"))
                } else {
                    Ok(candles()
                        .into_iter()
                        .map(|mut candle| {
                            candle.symbol = symbol.to_string();
                            candle
                        })
                        .collect())
                }
            },
        )
        .unwrap_err();

        assert_eq!(requests, vec!["AAPL".to_string(), "BROKEN".to_string()]);
        let msg = err.to_string();
        assert!(msg.contains("plan test 2"));
        assert!(msg.contains("broken"));
        assert!(msg.contains("loader exploded"));
    }
}
