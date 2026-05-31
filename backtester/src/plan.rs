use std::{fmt::Write as _, sync::Arc};

use anyhow::{anyhow, Result};
use chrono::{DateTime, NaiveDate};
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::{Candle, Timeframe};
use trading_runtime::{resolve_warmup_plan, RhaiStrategy, RuntimeConfig, WarmupPlan};

use crate::{
    run_prepared_runtime_backtest, BacktestResult, HistoricalCandleSeries, HistoricalMarketData,
    RuntimeBacktestConfig,
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
}

#[derive(Debug, Clone)]
struct DatasetPlanSpec {
    symbol: String,
    primary_timeframe: Timeframe,
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

#[derive(Debug, Clone)]
struct BaselinePlanSpec {
    dataset: DatasetPlanSpec,
    balance: f64,
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
        let primary_timeframe = dataset.primary_timeframe;
        let prepared = prepare_plan_runtime(strategy_src, &dataset, baseline.balance)
            .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        let market_data = load_warmup_aware_market_data(
            &dataset,
            &prepared.effective_config,
            &prepared.warmup_plan,
            &mut load_candles,
        )
        .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        let runtime_result = run_prepared_runtime_backtest(
            prepared.strategy,
            prepared.effective_config,
            prepared.warmup_plan,
            market_data,
            baseline.balance,
        )
        .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        if runtime_result.result.equity_curve.is_empty() {
            return Err(anyhow!(
                "{test_identity} failed: No tradable candles for {}/{} — run `just seed` first.",
                dataset.symbol,
                primary_timeframe,
            ));
        }
        let result = runtime_result.result;

        tests.push(BaselinePlanTest {
            name: test_spec.name,
            symbol: dataset.symbol,
            interval: primary_timeframe.to_string(),
            initial_balance: baseline.balance,
            result,
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
    let config = RuntimeBacktestConfig::new(
        dataset.symbol.clone(),
        dataset.primary_timeframe,
        initial_balance,
    );
    let effective_config = config
        .runtime_config
        .merge_strategy_config(strategy.strategy_config());
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
    }

    out
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

fn parse_timeframe(raw: &str) -> Result<Timeframe> {
    raw.parse()
        .map_err(|e| anyhow!("Invalid plan timeframe '{}': {e}", raw))
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
    rhai.register_type_with_name::<DatasetPlanSpec>("Dataset");
    rhai.register_type_with_name::<RunConfigPlanSpec>("RunConfig");
    rhai.register_type_with_name::<PlanTimestamp>("PlanTime");
    rhai.register_type_with_name::<Timeframe>("Timeframe");

    rhai.register_fn("timeframe", plan_timeframe);
    rhai.register_fn("time", plan_time);
    rhai.register_fn("__backtester_plan_result_new", PlanResultSpec::new);
    rhai.register_fn("__backtester_plan_test_new", |name: &str| PlanTestSpec {
        name: name.to_string(),
        baseline: None,
    });
    rhai.register_fn("__backtester_run_config_new", RunConfigPlanSpec::new);
    rhai.register_fn("with_title", |mut result: PlanResultSpec, title: &str| {
        result.title = Some(title.to_string());
        result
    });
    rhai.register_fn("with_test", with_test);
    rhai.register_fn("with_baseline", with_baseline);
    rhai.register_fn("with_balance", with_balance_float);
    rhai.register_fn("with_balance", with_balance_int);

    let mut dataset_module = Module::new();
    dataset_module.set_native_fn("load", dataset_load);
    rhai.register_static_module("dataset", Arc::new(dataset_module));

    let mut baseline_module = Module::new();
    baseline_module.set_native_fn("run", baseline_run);
    rhai.register_static_module("baseline", Arc::new(baseline_module));
}

fn plan_timeframe(raw: &str) -> std::result::Result<Timeframe, Box<EvalAltResult>> {
    parse_timeframe(raw).map_err(|error| Box::<EvalAltResult>::from(error.to_string()))
}

fn plan_time(raw: &str) -> std::result::Result<PlanTimestamp, Box<EvalAltResult>> {
    parse_plan_time(raw).map_err(|error| Box::<EvalAltResult>::from(error.to_string()))
}

fn dataset_load(
    symbol: &str,
    primary_timeframe: Timeframe,
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
        primary_timeframe,
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
        tests.push(ValidatedPlanTestSpec {
            name: test.name,
            baseline,
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
    const REPLACEMENTS: [(&str, &str); 3] = [
        ("plan_result::new(", "__backtester_plan_result_new("),
        ("plan_test::new(", "__backtester_plan_test_new("),
        ("run_config::new(", "__backtester_run_config_new("),
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

    const HOLD_STRATEGY: &str = r#"
fn on_tick(market, context) {
    decision::hold()
}
"#;

    const TYPED_MULTI_TEST_PLAN: &str = r#"
fn plan() {
    let aapl = dataset::load("AAPL", timeframe("1d"), time("2020-01-02"), time("2020-01-05"));
    let msft = dataset::load("MSFT", timeframe("1d"), time("2020-01-02"), time("2020-01-05"));
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
        timeframe("1d"),
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
    strategy_config::new().with_secondary(secondary::optional(H1))
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
    let dataset = dataset::load("AAPL", timeframe("1m"), time("2020-01-01"), time("2020-01-02"));

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
    strategy_config::new().with_minimum_warmup(2)
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let plan = r#"
fn plan() {
    let dataset = dataset::load("AAPL", timeframe("1d"), time("2020-01-03"), time("2020-01-05"));

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
            RuntimeBacktestConfig::new("AAPL", Timeframe::days(1), 10000.0),
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
        timeframe("1m"),
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
    strategy_config::new().with_minimum_warmup(2)
}

fn on_tick(market, context) {
    decision::hold()
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", timeframe("1d"), time("2020-01-03"), time("2020-01-04"));

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
        .with_minimum_warmup(1)
        .with_secondary(secondary::optional(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", timeframe("1m"), time("2020-01-01T00:01:00Z"), time("2020-01-01T00:02:00Z"));

    plan_result::new()
        .with_test(
            plan_test::new("missing secondary warmup")
                .with_baseline(baseline::run(dataset, run_config::new().with_balance(10000.0)))
        )
}
"#,
            |symbol, timeframe, request| {
                Ok(match (timeframe, request) {
                    (tf, DatasetCandleRequest::WarmupPrefix { .. }) if tf == Timeframe::minutes(1) => {
                        vec![candle_for(symbol, timeframe, day(1), 99.0)]
                    }
                    (tf, DatasetCandleRequest::Range { .. }) if tf == Timeframe::minutes(1) => {
                        vec![candle_for(symbol, timeframe, day(1) + 60_000, 100.0)]
                    }
                    (tf, DatasetCandleRequest::WarmupPrefix { .. }) if tf == Timeframe::hours(1) => {
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
    let dataset = dataset::load("AAPL", timeframe("1d"), time("2020-01-02"), time("2020-01-03"));

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
    fn dataset_host_object_is_opaque_to_rhai() {
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    let dataset = dataset::load("AAPL", timeframe("1d"), time("2020-01-02"), time("2020-01-03"));
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
    let dataset = dataset::load("AAPL", timeframe("1d"), time("2020-01-02"), time("2020-01-03"));

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
    let first = dataset::load("AAPL", timeframe("1d"), time("2020-01-02"), time("2020-01-05"));
    let broken = dataset::load("BROKEN", timeframe("1d"), time("2020-01-02"), time("2020-01-05"));
    let should_not_run = dataset::load("MSFT", timeframe("1d"), time("2020-01-02"), time("2020-01-05"));
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
