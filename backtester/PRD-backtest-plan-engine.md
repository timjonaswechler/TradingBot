## Problem Statement

TradingBot2 can currently run a single in-memory backtest from the CLI against one historical candle series and print a summary, but it cannot express a richer backtesting workflow. As a strategy author, I cannot write a backtest plan that loads one historical dataset, runs a baseline test, runs synthetic stress tests against reordered historical candles, and then returns structured data for a report. I also cannot keep the trading strategy and the testing procedure separate: the strategy is written in Rhai, but the backtesting workflow itself is still hard-coded in Rust.

This makes robustness testing awkward. The current backtester can tell me how one strategy performed on one historical path, but it cannot express “take this historical dataset, preserve the same candles, reorder them deterministically, rerun the strategy many times, and compare the original path against the synthetic distribution” as a first-class workflow. It also lacks a small scripting surface for assembling multiple tests into one report-ready result.

## Solution

TradingBot2 gets a dedicated backtest-plan scripting layer alongside the existing strategy engine. A plan script written in Rhai will orchestrate backtesting workflows without replacing the existing candle-by-candle trading engine. The strategy remains a separate Rhai file passed on the CLI. The plan script will load a historical dataset from the DB, run one or more baseline backtests, run one or more synthetic Monte Carlo tests using deterministic candle permutation of the same historical dataset, and return a structured plan result that Rust renders to Markdown.

The first supported synthetic procedure will be a Monte Carlo candle permutation test. It will take the fully loaded historical dataset, including its hidden warmup prefix, permute the candles without replacement, resequence timestamps monotonically, rerun the strategy with the same run configuration, and collect a synthetic result distribution. The plan result will contain named tests, where each test pairs one full baseline backtest run with one synthetic result object. The CLI will validate the returned structure strictly, fail fast on any test failure, and print a Markdown report to stdout.

## User Stories

1. As a strategy author, I want to keep my trading strategy in its own Rhai file, so that trading logic and testing workflow remain separate.
2. As a backtester user, I want to pass a plan file and a strategy file independently on the CLI, so that one plan can be reused against different strategies.
3. As a backtester user, I want the strategy path to be required on the CLI, so that the testing workflow never hides which strategy was executed.
4. As a strategy author, I want a plan script entry point with a simple `plan()` function, so that the scripting model stays small and easy to learn.
5. As a backtester user, I want the plan script to return structured data instead of printing its own report, so that report formatting stays centralized and consistent.
6. As a backtester user, I want the plan result to contain a list of named tests, so that one plan can produce a report with several clearly ordered comparison sections.
7. As a backtester user, I want test ordering in the returned list to define report ordering, so that the plan script controls the narrative flow of the report.
8. As a backtester user, I want each test to contain one baseline result and one synthetic result bundle, so that each test remains a clear comparison unit.
9. As a backtester user, I want the baseline to be a normal historical backtest run, so that the original path is always computed with the same execution logic as today.
10. As a backtester user, I want synthetic tests to start from historical candles instead of made-up candles, so that the stress test stays grounded in real market structure.
11. As a backtester user, I want the first synthetic procedure to permute historical candles without replacement, so that every synthetic run uses the same candle population as the source dataset.
12. As a backtester user, I want synthetic timestamps to be resequenced monotonically after permutation, so that time-based metrics remain coherent.
13. As a strategy author, I want the backtester to preserve the same warmup bar count between baseline and synthetic runs, so that comparisons stay fair.
14. As a strategy author, I want the synthetic procedure to permute the entire loaded dataset including hidden warmup candles, so that the synthetic path is internally consistent rather than partly original and partly synthetic.
15. As a backtester user, I want a dataset loader that accepts symbol, interval, start, and end, so that the plan script can define exactly which historical window to test.
16. As a strategy author, I want `start` in dataset loading to mean the first tradable candle, so that indicator warmup before the visible window is handled automatically.
17. As a backtester user, I want the loader to fetch the hidden warmup prefix automatically from historical data before the visible start date, so that early bars inside the requested test window have valid indicator context.
18. As a backtester user, I want a clear failure if there is not enough historical data before the requested start to satisfy warmup, so that I do not accidentally run a degraded test.
19. As a backtester user, I want a clear failure if the visible test window is empty, so that dataset mistakes are caught immediately.
20. As a backtester user, I want the first dataset loader to read from the DB only, so that V1 stays aligned with the existing in-memory backtester architecture.
21. As a backtester user, I want dataset objects to be opaque in the plan script, so that data integrity and transformation rules remain controlled by host functions.
22. As a backtester user, I want baseline run objects to be opaque in the plan script, so that V1 stays small and plan scripts cannot depend on unstable internal shapes.
23. As a backtester user, I want Monte Carlo result objects to be opaque in the plan script, so that V1 scripting focuses on orchestration rather than result introspection.
24. As a plan author, I want a simple host function to run a baseline backtest from a dataset and a run config, so that plans can assemble baseline and synthetic tests explicitly.
25. As a plan author, I want the run config to at least include starting balance, so that every test states how much capital the backtest trades with.
26. As a backtester user, I want plan-level defaults to be representable as normal Rhai maps, so that shared settings such as balance can be reused across tests without introducing implicit runtime state.
27. As a plan author, I want the Monte Carlo function to require iterations and seed explicitly, so that every stochastic test is fully declared in the plan.
28. As a backtester user, I want one base seed to deterministically derive all iteration seeds, so that the full test is reproducible from a single declared seed.
29. As a backtester user, I want one synthetic procedure per test, so that each test answers one robustness question cleanly.
30. As a backtester user, I want many synthetic iterations inside one synthetic result bundle, so that Monte Carlo outputs a distribution instead of a single anecdotal run.
31. As a backtester user, I want each synthetic iteration result to store its iteration number, seed, final equity, max drawdown, and trade count, so that I can debug or extend reporting later without storing full trade histories for every iteration.
32. As a backtester user, I want the baseline result to remain full-fidelity, including the source dataset and detailed backtest output, so that debugging and future report formats have access to the original context.
33. As a report reader, I want the synthetic summary to include percentiles for final equity and max drawdown, so that I can judge the strategy’s distribution under path variation.
34. As a report reader, I want the synthetic summary to include the baseline value’s percentile within the synthetic distribution, so that I can see whether the original historical path was unusually lucky or unlucky.
35. As a report reader, I want the first report format to be Markdown, so that the result is easy to read in the terminal, save to a file, or publish elsewhere.
36. As a CLI user, I want the Markdown report printed to stdout, so that I can immediately read it or redirect it to a file.
37. As a maintainer, I want strict validation of the plan return shape, so that invalid plan outputs fail before rendering.
38. As a maintainer, I want strict validation of each test’s required fields and host-object types, so that the renderer can rely on a stable contract.
39. As a maintainer, I want the whole plan execution to fail fast if any test fails, so that reports are never silently partial.
40. As a maintainer, I want the strategy to be compiled before plan execution so that warmup requirements are known when datasets are loaded, so that dataset loading and backtest execution share one warmup contract.
41. As a maintainer, I want the existing trading engine to remain the only candle-by-candle execution engine, so that trading semantics do not fork between normal backtests and plan-driven backtests.
42. As a maintainer, I want the new plan scripting layer to orchestrate runs rather than reimplement trade execution, so that strategy behavior stays identical across entry points.
43. As a maintainer, I want the synthetic candle permutation logic isolated behind a deep module, so that deterministic reordering, timestamp resequencing, and fairness rules can be tested independently of the CLI and renderer.
44. As a maintainer, I want dataset loading and warmup-window assembly isolated behind a deep module, so that historical windowing rules are testable without involving the whole plan runtime.
45. As a future UI user, I want the plan result to stay structured and renderer-independent, so that other output targets beyond CLI Markdown remain possible later.
46. As a future feature developer, I want additional synthetic procedures to appear as separate host functions instead of one generic string-driven method switch, so that the API remains explicit and easy to validate.
47. As a future plan author, I want the same plan runtime to support ordinary baseline-only reports as well as Monte Carlo reports, so that not every plan must be stochastic.
48. As a maintainer, I want V1 to avoid free-form result introspection inside Rhai, so that the first scripting surface remains narrow and stable.
49. As a maintainer, I want plan scripts to assemble tests using opaque host objects, so that internal Rust data structures can evolve without breaking user scripts.
50. As a maintainer, I want prior backtester semantics such as in-memory execution, deterministic warmup handling, and no DB writes during test execution to remain true under plan execution, so that the new capability extends the existing architecture instead of replacing it.

## Implementation Decisions

- The existing strategy execution engine remains the canonical candle-by-candle trading engine. The new work adds a separate backtest-plan runtime rather than a second trading engine.
- Plan scripts and strategy scripts are separate concepts. The CLI will require both a plan file and a strategy file. The strategy stays global for the whole plan and cannot be chosen from inside the plan script.
- Plan execution begins by loading and compiling the strategy, then deriving its warmup requirement once. Dataset loading will use that warmup requirement implicitly so that visible test windows start with valid indicator context.
- A dedicated plan runtime module will be introduced as a deep module. Its job is to compile and execute the plan script, register the small host API, call `plan()`, validate the returned structure, and convert it into a typed Rust-side plan result for rendering.
- A dedicated dataset-loading module will be introduced as a deep module. Its job is to load one historical window from the DB, interpret `start` as the first visible tradable bar, fetch the hidden warmup prefix before `start`, ensure the full dataset is chronological, and fail clearly when the window or warmup requirements cannot be satisfied.
- A dedicated synthetic-dataset module will be introduced as a deep module. Its first responsibility is deterministic candle permutation without replacement across the entire loaded dataset, including warmup candles, followed by monotonic timestamp resequencing. It should hide shuffling mechanics, seed derivation, and timestamp repair behind a small interface.
- The baseline backtest path will continue to use the existing in-memory backtester core rather than introducing a separate execution model for plan-driven runs.
- The plan host API in V1 will stay deliberately small and imperative: load a dataset, run a baseline backtest, run a candle-permutation Monte Carlo test, and return a structured plan object. The plan script orchestrates calls, but does not print output or inspect deep result internals.
- The dataset object exposed to Rhai will be an opaque host object. Plan scripts can pass it to host functions and place it inside returned structures indirectly through run objects, but cannot mutate or iterate raw candles directly.
- The baseline backtest result exposed to Rhai will be an opaque host object representing a historical backtest run. Internally it will contain the source dataset, the resolved run configuration, and the full backtest result.
- The synthetic Monte Carlo result exposed to Rhai will be an opaque host object. Internally it will contain the synthetic procedure’s configuration, reduced per-iteration results, and summary statistics needed by the renderer.
- Plan-level defaults such as balance remain explicit Rhai values rather than implicit runtime state. V1 will not introduce a special defaults registry or global setter API.
- The first Monte Carlo host function will be named for its actual semantics: candle permutation rather than generic reshuffling. This avoids conflating candle-path testing with trade-PnL resampling methods described elsewhere.
- The first Monte Carlo procedure will operate on historical candles rather than on trades. It will not synthesize new candle values in V1; it will only reorder existing historical candles.
- The first Monte Carlo procedure will require a starting balance, an iteration count, and a seed. The balance determines the backtest run configuration for every synthetic run. The seed is a single base seed from which deterministic per-iteration seeds are derived.
- Every synthetic iteration will rerun the full strategy on a fully permuted dataset using the same run configuration and the same warmup-bar count as the baseline. This preserves fairness between original and synthetic paths.
- The synthetic result bundle will store reduced per-iteration outputs rather than full trade lists and equity curves for every run. V1 will keep only the minimum iteration metadata required for reporting and future debugging: iteration index, seed, final equity, max drawdown, and trade count.
- The baseline run remains full-fidelity and keeps the full dataset plus the normal backtest output. This preserves the richest possible original-context artifact without imposing the same memory cost on every synthetic iteration.
- Each plan-level test will be assembled in Rhai as a normal map with strict required fields: `name`, `baseline`, and `synthetic`. This keeps orchestration flexible while still giving the runtime a stable validation contract.
- The top-level plan return shape will be a normal map with optional `title`, optional `notes`, and required `tests`. `tests` ordering will be treated as the report ordering.
- Validation will be strict and structural. The runtime will verify that `plan()` exists, returns a map, contains a `tests` array, and that every test entry has the required keys with the expected host-object types.
- Errors in any test will abort the entire plan execution. V1 intentionally prefers fail-fast semantics over partial reports.
- The renderer will be a separate module from the plan runtime. It will accept the typed validated plan result and produce Markdown for stdout. This keeps scripting, validation, and presentation decoupled.
- Summary statistics for the candle-permutation Monte Carlo result will focus on `final_equity` and `max_drawdown` in V1. For each metric, the summary will include the baseline value, synthetic `p5`, `p50`, `p95`, and the baseline percentile inside the synthetic distribution. The percentile is stored internally as a 0.0–1.0 value and rendered as needed.
- The synthetic result object will not duplicate a baseline result. The baseline historical run and the synthetic result bundle remain separate objects that the test map ties together.
- The design should preserve the current in-memory, no-DB-writes backtesting model. Plan execution is an orchestration layer over the existing backtester, not a new persistence workflow.
- The design should preserve compatibility with future expansion. New stochastic procedures should appear as additional explicit host functions rather than as a string-switched generic method field inside one umbrella API.

## Testing Decisions

- Good tests will assert external behavior only: plan API behavior, dataset loading semantics, warmup-window behavior, permutation determinism, result validation, summary statistics, and final Markdown content. They will not assert lock usage, AST storage details, or private implementation structure.
- The plan runtime module will receive focused tests proving that valid plan scripts compile and return accepted structures, and that invalid return shapes fail with clear errors.
- The dataset-loading module will receive focused tests covering visible window selection, hidden warmup prefix inclusion, failure on insufficient warmup history, failure on empty visible windows, and chronological dataset assembly.
- The synthetic candle-permutation module will receive focused tests covering deterministic seed behavior, permutation without replacement, preservation of candle count, inclusion of the warmup prefix in permutation, and monotonic timestamp resequencing.
- The baseline backtest integration used by plan execution will receive tests proving that plan-driven backtests still match ordinary backtest semantics for the same dataset and run configuration.
- The Monte Carlo execution layer will receive tests proving that all requested iterations run deterministically from the base seed, that reduced per-iteration results are captured correctly, and that summary percentiles and baseline percentile comparisons are computed correctly.
- The plan-result validator will receive tests proving that missing `tests`, non-array `tests`, missing `name`, wrong host-object types, and other malformed return shapes fail clearly before rendering.
- The Markdown renderer will receive tests proving that a validated plan result is rendered in test order, that baseline-versus-synthetic comparison data appears correctly, and that optional title and notes are handled properly.
- Prior art should follow the testing style already present in the codebase: engine behavior tests around Rhai execution and state persistence, backtester tests around trade and equity outcomes, warmup-related tests around historical preload semantics, and DB-layer integration tests around query behavior.

## Out of Scope

- Allowing the plan script to choose or override the strategy file.
- Allowing plan scripts to inspect deep fields inside datasets, baseline runs, or Monte Carlo result objects.
- Allowing plan scripts to print output or build Markdown directly.
- Additional data sources such as CSV files or arbitrary filesystem inputs.
- Multiple disjoint time windows inside one dataset.
- Generating brand-new synthetic candle values rather than reordering existing historical candles.
- Trade-PnL resampling Monte Carlo, regime-switching Monte Carlo, block permutation, or any other stochastic procedure beyond full candle permutation without replacement.
- Partial success reporting when one test fails.
- Parallel execution, optimizer flows, parameter search, or walk-forward testing.
- UI features beyond preserving the structured result boundary so that other renderers remain possible later.
- Persistent storage of plan results or synthetic runs.
- A broad general-purpose orchestration language; V1 is intentionally a narrow plan runtime over a small host API.

## Further Notes

- The most valuable deep modules for this feature are the dataset loader, the synthetic candle-permutation engine, and the plan runtime/validator. Keeping those boundaries deep and small should make future stochastic procedures much easier to add.
- The biggest correctness risk is accidental divergence between baseline and synthetic execution semantics. The design should therefore centralize execution through the existing in-memory backtester and preserve one shared warmup contract.
- Another important correctness risk is producing synthetic datasets with inconsistent time structure. Timestamp resequencing should be treated as part of the synthetic dataset contract, not as an optional formatting step.
- V1 intentionally chooses opaque host objects and minimal plan introspection to keep the scripting surface stable. If later work adds conditional branching based on results, it should do so by designing a deliberate read-only result API rather than exposing raw internals ad hoc.
- The Markdown renderer should be considered an output adapter, not the source of truth. The typed validated plan result remains the canonical representation of what a plan produced.