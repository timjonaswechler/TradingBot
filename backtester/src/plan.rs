use std::fmt::Write as _;

use anyhow::{anyhow, Result};
use engine::{detect_warmup_period, Engine as StrategyEngine};
use rhai::{Dynamic, Engine as RhaiEngine, Map, Scope, AST, FLOAT, INT};
use shared::Candle;

use crate::{run_backtest, BacktestConfig, BacktestResult};

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
struct BaselinePlanSpec {
    name: String,
    symbol: String,
    interval: String,
    balance: f64,
}

pub fn execute_plan<F>(
    strategy_src: &str,
    plan_src: &str,
    mut load_candles: F,
) -> Result<PlanReport>
where
    F: FnMut(&str, &str) -> Result<Vec<Candle>>,
{
    let warmup_bars = {
        let strategy = StrategyEngine::new(strategy_src)?;
        detect_warmup_period(strategy.ast(), strategy.scope())
    };

    let (title, specs) = parse_plan(plan_src)?;
    let mut tests = Vec::with_capacity(specs.len());

    for spec in specs {
        let candles = load_candles(&spec.symbol, &spec.interval)?;
        if candles.is_empty() {
            return Err(anyhow!(
                "No candles for {}/{} — run `just seed` first.",
                spec.symbol,
                spec.interval,
            ));
        }

        let mut engine = StrategyEngine::new(strategy_src)?;
        let result = run_backtest(
            &mut engine,
            candles,
            BacktestConfig {
                initial_balance: spec.balance,
                warmup_bars,
            },
        )?;

        tests.push(BaselinePlanTest {
            name: spec.name,
            symbol: spec.symbol,
            interval: spec.interval,
            initial_balance: spec.balance,
            result,
        });
    }

    Ok(PlanReport { title, tests })
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

fn parse_plan(plan_src: &str) -> Result<(Option<String>, Vec<BaselinePlanSpec>)> {
    let rhai = RhaiEngine::new();
    let ast = compile_plan(&rhai, plan_src)?;
    let mut scope = Scope::new();

    rhai.run_ast_with_scope(&mut scope, &ast)
        .map_err(|e| anyhow!("plan script failed during setup: {e}"))?;

    let result: Dynamic = rhai
        .call_fn(&mut scope, &ast, "plan", ())
        .map_err(|e| anyhow!("plan() failed: {e}"))?;

    let map = result
        .try_cast::<Map>()
        .ok_or_else(|| anyhow!("plan() must return a map"))?;

    let title = map.get("title").and_then(dynamic_to_string);
    let tests_dynamic = map
        .get("tests")
        .cloned()
        .ok_or_else(|| anyhow!("plan() must return a `tests` array"))?;
    let tests = tests_dynamic
        .try_cast::<rhai::Array>()
        .ok_or_else(|| anyhow!("plan().tests must be an array"))?;

    if tests.is_empty() {
        return Err(anyhow!("plan().tests must contain at least one test"));
    }

    let mut specs = Vec::with_capacity(tests.len());
    for (index, test) in tests.into_iter().enumerate() {
        let map = test
            .try_cast::<Map>()
            .ok_or_else(|| anyhow!("plan().tests[{index}] must be a map"))?;
        specs.push(BaselinePlanSpec {
            name: required_string(&map, "name", index)?,
            symbol: required_string(&map, "symbol", index)?,
            interval: required_string(&map, "interval", index)?,
            balance: required_f64(&map, "balance", index)?,
        });
    }

    Ok((title, specs))
}

fn compile_plan(rhai: &RhaiEngine, plan_src: &str) -> Result<AST> {
    let ast = rhai
        .compile(plan_src)
        .map_err(|e| anyhow!("plan script compile error: {e}"))?;

    let has_plan = ast
        .iter_functions()
        .any(|f| f.name == "plan" && f.params.is_empty());
    if !has_plan {
        return Err(anyhow!("plan script must define `fn plan()`"));
    }

    Ok(ast)
}

fn required_string(map: &Map, key: &str, index: usize) -> Result<String> {
    map.get(key)
        .and_then(dynamic_to_string)
        .ok_or_else(|| anyhow!("plan().tests[{index}].{key} must be a string"))
}

fn required_f64(map: &Map, key: &str, index: usize) -> Result<f64> {
    map.get(key)
        .and_then(dynamic_to_f64)
        .ok_or_else(|| anyhow!("plan().tests[{index}].{key} must be a number"))
}

fn dynamic_to_string(value: &Dynamic) -> Option<String> {
    value.clone().try_cast::<String>()
}

fn dynamic_to_f64(value: &Dynamic) -> Option<f64> {
    value
        .clone()
        .try_cast::<FLOAT>()
        .or_else(|| value.clone().try_cast::<INT>().map(|v| v as f64))
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
fn on_tick(candles, context) {
    #{ signal: "HOLD" }
}
"#;

    const BASELINE_PLAN: &str = r#"
fn plan() {
    #{
        title: "Smoke test",
        tests: [
            #{
                name: "AAPL baseline",
                symbol: "AAPL",
                interval: "1d",
                balance: 10000.0,
            }
        ],
    }
}
"#;

    #[test]
    fn executes_one_baseline_plan_and_renders_markdown() {
        let report = execute_plan(HOLD_STRATEGY, BASELINE_PLAN, |symbol, interval| {
            assert_eq!(symbol, "AAPL");
            assert_eq!(interval, "1d");
            Ok(candles())
        })
        .unwrap();

        assert_eq!(report.tests.len(), 1);
        assert_eq!(report.tests[0].name, "AAPL baseline");

        let markdown = render_markdown(&report, "strategies/test.rhai");
        assert!(markdown.contains("# Smoke test"));
        assert!(markdown.contains("## 1. AAPL baseline"));
        assert!(markdown.contains("- Strategy: `strategies/test.rhai`"));
        assert!(markdown.contains("- Final equity:"));
    }

    #[test]
    fn missing_plan_function_fails_clearly() {
        let err = execute_plan(HOLD_STRATEGY, "let x = 1;", |_symbol, _interval| {
            Ok(candles())
        })
        .unwrap_err();

        assert!(err.to_string().contains("fn plan()"));
    }
}
