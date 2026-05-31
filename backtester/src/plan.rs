use std::{fmt::Write as _, sync::Arc};

use anyhow::{anyhow, Result};
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::{Candle, Timeframe};

use crate::{run_runtime_backtest_with_loader, BacktestResult, RuntimeBacktestConfig};

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
struct BaselinePlanSpec {
    symbol: String,
    interval: String,
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
    F: FnMut(&str, Timeframe) -> Result<Vec<Candle>>,
{
    let plan = parse_plan(plan_src)?;
    let mut tests = Vec::with_capacity(plan.tests.len());

    for (index, test_spec) in plan.tests.into_iter().enumerate() {
        let test_identity = format!("plan test {} ('{}')", index + 1, test_spec.name);
        let baseline = test_spec.baseline;
        let primary_timeframe = parse_timeframe(&baseline.interval)
            .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        let runtime_result = run_runtime_backtest_with_loader(
            strategy_src,
            RuntimeBacktestConfig::new(
                baseline.symbol.clone(),
                primary_timeframe,
                baseline.balance,
            ),
            |symbol, timeframe| load_candles(symbol, timeframe),
        )
        .map_err(|error| anyhow!("{test_identity} failed: {error}"))?;
        if runtime_result.result.equity_curve.is_empty() {
            return Err(anyhow!(
                "{test_identity} failed: No tradable candles for {}/{} — run `just seed` first.",
                baseline.symbol,
                baseline.interval,
            ));
        }
        let result = runtime_result.result;

        tests.push(BaselinePlanTest {
            name: test_spec.name,
            symbol: baseline.symbol,
            interval: baseline.interval,
            initial_balance: baseline.balance,
            result,
        });
    }

    Ok(PlanReport {
        title: plan.title,
        tests,
    })
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
        .map_err(|e| anyhow!("Invalid plan interval '{}': {e}", raw))
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

    rhai.register_fn("__backtester_plan_result_new", PlanResultSpec::new);
    rhai.register_fn("__backtester_plan_test_new", |name: &str| PlanTestSpec {
        name: name.to_string(),
        baseline: None,
    });
    rhai.register_fn("with_title", |mut result: PlanResultSpec, title: &str| {
        result.title = Some(title.to_string());
        result
    });
    rhai.register_fn("with_test", with_test);
    rhai.register_fn("with_baseline", with_baseline);

    let mut baseline_module = Module::new();
    baseline_module.set_native_fn("run", |symbol: &str, interval: &str, balance: FLOAT| {
        baseline_run(symbol, interval, balance)
    });
    baseline_module.set_native_fn("run", |symbol: &str, interval: &str, balance: INT| {
        baseline_run(symbol, interval, balance as f64)
    });
    rhai.register_static_module("baseline", Arc::new(baseline_module));
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
    symbol: &str,
    interval: &str,
    balance: f64,
) -> std::result::Result<BaselinePlanSpec, Box<EvalAltResult>> {
    if symbol.trim().is_empty() {
        return Err("baseline::run symbol must not be empty".into());
    }
    if interval.trim().is_empty() {
        return Err("baseline::run interval must not be empty".into());
    }
    if !balance.is_finite() {
        return Err("baseline::run balance must be finite".into());
    }

    Ok(BaselinePlanSpec {
        symbol: symbol.to_string(),
        interval: interval.to_string(),
        balance,
    })
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
    const REPLACEMENTS: [(&str, &str); 2] = [
        ("plan_result::new(", "__backtester_plan_result_new("),
        ("plan_test::new(", "__backtester_plan_test_new("),
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

    fn make_candle(ts: i64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            symbol: "AAPL".into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            timeframe: "1d".parse().unwrap(),
        }
    }

    fn candles() -> Vec<Candle> {
        vec![
            make_candle(1, 100.0),
            make_candle(2, 101.0),
            make_candle(3, 102.0),
        ]
    }

    const HOLD_STRATEGY: &str = r#"
fn on_tick(market, context) {
    decision::hold()
}
"#;

    const TYPED_MULTI_TEST_PLAN: &str = r#"
fn plan() {
    plan_result::new()
        .with_title("Smoke test")
        .with_test(
            plan_test::new("AAPL baseline")
                .with_baseline(baseline::run("AAPL", "1d", 10000.0))
        )
        .with_test(
            plan_test::new("MSFT baseline")
                .with_baseline(baseline::run("MSFT", "1d", 5000))
        )
}
"#;

    #[test]
    fn typed_plan_result_renders_multiple_tests_in_insertion_order() {
        let mut requests = Vec::new();
        let report = execute_plan(HOLD_STRATEGY, TYPED_MULTI_TEST_PLAN, |symbol, timeframe| {
            requests.push((symbol.to_string(), timeframe));
            Ok(candles())
        })
        .unwrap();

        assert_eq!(
            requests,
            vec![
                ("AAPL".to_string(), Timeframe::days(1)),
                ("MSFT".to_string(), Timeframe::days(1)),
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
    fn missing_plan_function_fails_clearly() {
        let err = execute_plan(HOLD_STRATEGY, "let x = 1;", |_symbol, _timeframe| {
            Ok(candles())
        })
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
            |_symbol, _timeframe| Ok(candles()),
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
            |_symbol, _timeframe| Ok(candles()),
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
            |_symbol, _timeframe| Ok(candles()),
        )
        .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("with_baseline"));
        assert!(msg.contains("BaselineRun"));
    }

    #[test]
    fn plan_execution_fails_fast_with_failing_test_identity() {
        let mut requests = Vec::new();
        let err = execute_plan(
            HOLD_STRATEGY,
            r#"
fn plan() {
    plan_result::new()
        .with_test(plan_test::new("first").with_baseline(baseline::run("AAPL", "1d", 10000.0)))
        .with_test(plan_test::new("broken").with_baseline(baseline::run("BROKEN", "1d", 10000.0)))
        .with_test(plan_test::new("should not run").with_baseline(baseline::run("MSFT", "1d", 10000.0)))
}
"#,
            |symbol, _timeframe| {
                requests.push(symbol.to_string());
                if symbol == "BROKEN" {
                    Err(anyhow::anyhow!("loader exploded"))
                } else {
                    Ok(candles())
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
