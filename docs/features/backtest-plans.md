# Runtime-backed Backtest Plans

Backtest Plans are Rhai scripts that orchestrate reusable historical research
workflows. They are separate from strategy scripts: the plan loads datasets,
starts baseline and Synthetic Market Data runs, and returns a typed report
object; the strategy is still the Runtime-backed Rhai strategy executed on every
Strategy Tick.

## Run the examples

Seed local historical candles first, then pass both a strategy path and a plan
path to the backtester:

```sh
just db-start        # separate terminal, if the local DB is not running
just db-setup        # first local setup only
just seed            # uses trading-bot.toml; seeds AAPL 30m, 60m, and 1d from 2020-01-01

just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/plan.rhai
just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/candle_permutation_monte_carlo.rhai
```

In plan mode, `--strategy` is required and `--plan` is required. `--symbol`,
`--interval`, and `--balance` are direct-backtest CLI settings; a plan declares
its own Runtime Asset, Primary Timeframe, visible window, and run balance.

The same plan can be reused with another typed Runtime strategy:

```sh
just backtest --strategy strategies/min_loss.rhai --plan backtest_plan/plan.rhai
```

A plan cannot choose or replace the strategy file. The backtester loads the
strategy before plan execution so the Runtime configuration, Strategy
Configuration, Secondary Timeframes, and WarmupPlan are known before datasets are
loaded.

## Available example plans

- [`backtest_plan/plan.rhai`](../../backtest_plan/plan.rhai) — baseline-only
  AAPL daily window.
- [`backtest_plan/candle_permutation_monte_carlo.rhai`](../../backtest_plan/candle_permutation_monte_carlo.rhai)
  — baseline plus the #20 Synthetic Market Data candle-permutation Monte Carlo
  procedure.

Both examples use the typed/fluent Backtest Plan host API. They do not return raw
maps and do not expose datasets, baseline runs, or Monte Carlo bundles for Rhai
field inspection.

## Plan host API style

Backtest Plan APIs follow the same typed/fluent direction as ADR 0005 strategy
APIs: construct opaque host objects, pass them to host functions, and assemble a
typed result.

```rhai
fn plan() {
    let dataset = dataset::load(
        "AAPL",
        timeframe("1d"),
        time("2021-01-04"),
        time("2022-01-03"),
    );
    let config = run_config::new().with_balance(10000.0);
    let baseline = baseline::run(dataset, config);
    let synthetic = monte_carlo::candle_permutation(
        baseline,
        monte_carlo_config::new(25, 42),
    );

    plan_result::new()
        .with_title("AAPL candle-path robustness")
        .with_test(
            plan_test::new("Synthetic Market Data: candle permutation")
                .with_baseline(baseline)
                .with_synthetic(synthetic)
        )
}
```

Current plan-facing constructors/functions:

- `timeframe("1d")` and `time("2021-01-04")`
- `dataset::load(asset, primary_timeframe, start, end)`
- `run_config::new().with_balance(balance)`
- `baseline::run(dataset, run_config)`
- `monte_carlo_config::new(iterations, base_seed)`
- `monte_carlo::candle_permutation(baseline, config)`
- `plan_test::new(name).with_baseline(...).with_synthetic(...)`
- `plan_result::new().with_title(...).with_test(...)`

## Dataset loader contract

`dataset::load(asset, primary_timeframe, start, end)` declares one visible
Runtime-backed dataset window:

- `asset` names the Runtime Asset, for example `"AAPL"`.
- `primary_timeframe` names the Primary Timeframe whose completed candles become
  Tradable Candles, for example `timeframe("1d")`.
- `start` is the first visible Primary-Timeframe Tradable Candle.
- `end` is exclusive; visible Primary windows are half-open: `[start, end)`.
- `time(...)` accepts RFC3339 timestamps and date-only `YYYY-MM-DD` as UTC
  midnight.
- The loader reads the DB-backed historical candle source only.

The plan author does not list Secondary Timeframes in `dataset::load`. The
backtester loads the strategy first, merges `strategy_config()` into the Runtime
configuration, resolves the Runtime `WarmupPlan`, and then loads every configured
timeframe needed for the run.

Warmup behavior:

- The hidden Primary warmup prefix before `start` is loaded automatically from
  the resolved Runtime `WarmupPlan`.
- Configured Secondary Timeframes also receive their hidden warmup prefixes.
- Secondary context after warmup is derived from the visible Primary series and
  is loaded only through the last visible Primary Candle Timestamp; the loader
  does not fetch future Secondary candles after the visible Primary window.
- If the visible Primary window is empty, or the DB does not contain enough
  Primary/Secondary history before `start` to satisfy Runtime warmup, plan
  execution fails before rendering a partial report.

The assembled dataset is opaque inside Rhai. Plans can pass it to
`baseline::run(...)`, but cannot inspect or mutate raw candles.

## Report ordering and Markdown output

Report order is the order in which tests are attached to the typed
`plan_result` with `.with_test(...)`. The first attached test renders as `## 1`,
the second as `## 2`, and so on.

The Markdown report starts with the plan title, strategy path, and test count.
Each test section includes the baseline Runtime-backed metrics:

- symbol and Primary interval
- initial balance
- final equity
- max drawdown and max drawdown percent
- completed trade count

Synthetic Market Data Monte Carlo tests add a comparison section:

- `Procedure` identifies the mutation procedure, currently `Candle permutation`.
- `Iterations` is the number declared in `monte_carlo_config::new(...)`.
- The metric table compares baseline final equity and max drawdown against
  synthetic p5, p50, and p95 values.
- `Baseline percentile` shows where the original historical path sits inside the
  synthetic distribution. For final equity, a high percentile means the baseline
  ended above most synthetic candle paths. For max drawdown, a high percentile
  means the baseline drawdown was larger/more painful than most synthetic paths.
- Reduced iteration diagnostics list each iteration seed, final equity, max
  drawdown, trade count, blocked Strategy Tick count, Strategy Exit count, Risk
  Exit count, and Force Close count.

Reports are printed to stdout, so they can be read in the terminal or redirected:

```sh
just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/candle_permutation_monte_carlo.rhai > report.md
```

## Synthetic Market Data vs trade-order resampling

Synthetic Market Data Monte Carlo mutates copied historical candle datasets
before replay. The Trading Runtime then sees ordinary market input and reruns the
full strategy against each synthetic candle path. The currently available
procedure is #20 candle permutation: it reorders existing candle payloads without
replacement into the original chronological timestamp slots while preserving OHLC
invariants.

Future Synthetic Market Data mutation issues remain separate and are not
available yet:

- #90 — OHLC noise with repair
- #91 — log-difference bar permutation
- #92 — grouped multi-timeframe candle permutation
- #93 — regenerate higher timeframes from a mutated lowest timeframe
- #94 — composed Synthetic Market Data mutation pipelines

Trade-order resampling (#95) is a different future analysis. It operates on a
completed baseline trade ledger/equity path after the backtest, does not mutate
candles, and does not rerun the Trading Runtime. Use Synthetic Market Data
mutation to ask, “What if the market candle path had varied?” Use trade-order
resampling to ask, “Given these completed trade outcomes, what if their order or
sample had varied?”
