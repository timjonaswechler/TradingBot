# Runtime Migration Control

This document is the operational control map for the Trading Runtime refactor. It tells maintainers and agents what is canonical, what is donor material, what is transitional, and where intentional gaps are tracked.

It is not a glossary. Domain terms belong in `CONTEXT.md`.
It is not an ADR. Hard-to-reverse surprising trade-offs belong in `docs/adr/`.
It is not the detailed implementation plan. The working plan remains `docs/refactor/trading-runtime-refactor-plan.md`.

Update this document during future grilling sessions whenever a boundary, donor status, deletion rule, or intentional gap is clarified.

## Canonical target architecture

- `trading-runtime` is the canonical home for Trading Runtime behavior: market input handling, Market State, Market View, Strategy Handling, Strategy State, Portfolio State, Execution Planning, Portfolio Transitions, Risk Exits, and DB-free Runtime Events.
- `trading-daemon` is a LiveRunner and IO layer: CLI/config, timers, provider fetch, DB reads/writes through adapters, broker/IO adapters, logging, shutdown, and feeding market input or explicit commands into `trading-runtime`.
- `backtester` is a BacktestRunner and reporting/research layer: load/replay historical candles, feed `trading-runtime`, collect Runtime Events/snapshots, and compute reports/metrics.
- `db-layer` is a SpacetimeDB adapter: generated bindings, queries, reducers, and DB/domain mapping. It must not own Trading Runtime decisions, PnL, stops, or Portfolio Transitions.
- `indicators` owns pure indicator functions. It must not depend on Rhai, runtime state, DB, or runner behavior.
- `shared` is a temporary value-type crate on the path to `domain`. It may contain pure domain/value types while #36 is open, but it must not gain new Runtime semantics.

## Source-of-truth hierarchy

When documents or code disagree, use this order:

1. Accepted ADRs in `docs/adr/` for decisions they explicitly cover.
2. `CONTEXT.md` for canonical domain language only.
3. `docs/refactor/runtime-migration-control.md` for migration/control status.
4. `docs/refactor/trading-runtime-refactor-plan.md` for target plan and implementation phases.
5. Current issue bodies/comments for active slice-specific scope.
6. Existing code, only after checking whether it is canonical, transitional, or donor material below.

Old architecture documents are not source of truth unless an active issue explicitly says to use them as historical context.

## Current code classification

### Canonical / build here

- `trading-runtime/`
  - Build new Runtime behavior here.
  - New Strategy Handling must use the typed Rhai runtime API from ADR 0005.
  - New Portfolio/Execution behavior must be DB-free and shared by live and backtest.

### Runner / adapter layers

- `trading-daemon/`
  - Build only LiveRunner, IO, persistence orchestration, provider fetch, shutdown policy, and runtime feeding here.
  - Do not add new paper portfolio/execution semantics here.
  - If behavior belongs to live and backtest, it belongs in `trading-runtime`, not here.

- `backtester/`
  - Build only historical replay, reporting, metrics, and research orchestration here.
  - Do not add new portfolio/execution semantics here.
  - Runtime-backed backtests should use `trading-runtime` behavior.

- `db-layer/`
  - Build only DB adapter behavior and mapping here.
  - Do not add trading decisions, PnL, stop/take-profit, Strategy Tick, or Portfolio Transition logic here.

### Donor material / do not extend as architecture

- `engine/`
  - Donor only for old Rhai execution behavior, warmup detection, indicator bindings, anchored behavior, and Strategy State behavior.
  - Do not add new product behavior or new runtime-facing APIs here.
  - Do not make `trading-runtime` depend permanently on `engine`.
  - Once equivalent behavior is migrated and test-protected in `trading-runtime`, the donor code can be removed or absorbed.

### Transitional / treat with caution

- `shared/`
  - Temporary value-type crate until #36 resolves the `shared` -> `domain` cleanup.
  - Existing pure value types may remain temporarily.
  - New Runtime semantics must not be added here.
  - Helpers that express Portfolio/Execution behavior should migrate into `trading-runtime`.

- Legacy backtester engine-backed runner paths
  - Transitional regression/donor path only.
  - Do not extend with new behavior.
  - Can be removed once runtime-backed backtests cover the intended scenarios and #64 acceptance is satisfied.

- `trading-daemon/src/order_executor.rs`
  - Transitional live paper/DB execution path.
  - Do not use as the canonical model for new execution semantics.
  - Use it as migration context and regression reference only until live runtime-backed execution/persistence boundaries are complete.

- `trading-daemon/src/warmup.rs`
  - Transitional old-engine warmup path.
  - Do not extend for new runtime warmup behavior.
  - Runtime warmup behavior belongs in `trading-runtime`; runner fetching policy belongs in `trading-daemon`.

## Path inventory

Use this inventory when reviewing agent plans or deciding whether code may be changed, ported, or deleted.

### `trading-runtime/` canonical modules

- `trading-runtime/src/runtime.rs` — canonical Runtime orchestration and RuntimeStep production. Build shared live/backtest session behavior here.
- `trading-runtime/src/market_input.rs` — canonical runtime input boundary and `RuntimeConfig` materialization from the strategy-declared Primary/Secondary timeframe contract.
- `trading-runtime/src/market_state.rs` — canonical Market State storage and configured-timeframe handling.
- `trading-runtime/src/strategy.rs` — canonical strategy-facing abstractions: Market View, Strategy Context, Strategy State, StrategyHandler boundary.
- `trading-runtime/src/rhai_strategy.rs` — canonical typed Rhai strategy API. Do not reintroduce legacy map/signal compatibility here without reopening ADR 0004 / ADR 0005.
- `trading-runtime/src/strategy_config.rs` — canonical typed Strategy Configuration model.
- `trading-runtime/src/warmup.rs` — canonical runtime warmup resolution and auto-warmup detection target.
- `trading-runtime/src/decision.rs` — canonical Strategy Decision intent model.
- `trading-runtime/src/execution.rs` — canonical Execution Planning. Planning does not own DB persistence.
- `trading-runtime/src/portfolio.rs` — canonical runtime-local Portfolio State and Portfolio Transitions.
- `trading-runtime/src/risk_exit.rs` — canonical runtime-managed Risk Exit evaluation.
- `trading-runtime/src/events.rs` — canonical DB-free Runtime Events. Persistence mapping belongs outside.
- `trading-runtime/src/step.rs` — canonical ordered runtime step output.
- `trading-runtime/src/anchored.rs` — canonical runtime-facing anchored/structure-aware behavior already ported from old donor concepts.

### `engine/` donor inventory

- `engine/src/vm.rs` — donor for legacy Rhai execution, old `on_tick(candles, context)` behavior, Strategy State regression ideas, and old tests. Do not add new strategy API here.
- `engine/src/candle_wrapper.rs` — donor for old Rhai candle/context wrapper behavior and CandleList indexing semantics. Target API is Market View in `trading-runtime`.
- `engine/src/bindings.rs` — donor for old Rhai indicator bindings. New bindings belong in `trading-runtime/src/rhai_strategy.rs` or a runtime-owned submodule.
- `engine/src/warmup_detector.rs` — donor for warmup detection behavior. Target is `trading-runtime/src/warmup.rs`.
- `engine/src/warmup.rs` — donor/transitional old-engine warmup helper. Target warmup semantics belong in `trading-runtime`; runner fetch policy belongs in runner crates.
- `engine/src/anchored.rs` — donor for anchored behavior not yet fully absorbed. New anchored runtime-facing behavior belongs in `trading-runtime/src/anchored.rs`.
- `engine/src/indicator_cache.rs` — donor for old compute/cache behavior. New Compute State belongs inside `trading-runtime` when explicitly scoped.
- `engine/src/strategy_loader.rs` — donor for old load-time validation/config ideas. Target is typed `RhaiStrategy` loading/config extraction.
- `engine/src/error.rs` / `engine/src/lib.rs` — donor crate surface only. Do not expand public surface.

### `shared/` transitional inventory

#36 sequencing decision: perform the first `shared` → `domain` slice as a mechanical rename with workspace/import updates only, even if legacy modules are temporarily carried along under the new crate. Semantic cleanup must follow in separate test-protected slices using the classifications below; do not treat temporarily carried legacy modules as accepted future-domain contents.

- `shared/src/candle.rs` — temporary value type; likely remains in future `domain` unless #36 decides otherwise.
- `shared/src/timeframe.rs` — temporary value type; likely remains in future `domain` unless #36 decides otherwise.
- `shared/src/position.rs` — temporary value type. The future `domain` crate may keep a pure Open Position value (`PositionSide`, Runtime Asset/symbol, entry price, quantity, entry time, optional Entry Risk Parameters), but runtime-specific Portfolio Snapshot, Portfolio Transition, PnL/equity calculation, and Risk Exit semantics belong in `trading-runtime`. The current `Position::unrealised_pnl` helper is not future-domain behavior, and current `size` naming should migrate toward canonical `quantity` when the semantic cleanup slice reaches this type.
- `shared/src/signal.rs` — legacy decision/signal vocabulary (`Signal` / `TradeDecision`). Do not include it in the future `domain` crate and do not build new strategy semantics on it; target is `trading-runtime/src/decision.rs` (`StrategyDecision` plus typed Rhai `decision::*`). It is removable after old donor/transitional consumers are retired or migrated to canonical runtime Strategy Decisions.
- `shared/src/context.rs` — legacy strategy context shape. Do not include it in the future `domain` crate and do not extend it. The canonical Strategy Context is runtime-owned in `trading-runtime`; it may expose Runtime Portfolio Snapshot, Strategy State, and limited session metadata, but not Market Data, runner policy, DB fields, or Portfolio Transition behavior. Remove the legacy shared context after old `engine` / legacy backtester consumers are retired or migrated.
- `shared/src/executor.rs` — transitional/removable Portfolio/Execution helpers. Do not include an executor module in the future `domain` crate and do not add new behavior here. `shared::plan_action` and `shared::Action` are legacy planning over `Signal`; canonical Execution Planning is `trading-runtime::plan_execution` over `StrategyDecision`. `shared::realized_pnl` is Portfolio Transition/PnL semantics and should move into `trading-runtime/src/portfolio.rs` or stay private there before this shared module is deleted.

### `backtester/` inventory

- `backtester/src/lib.rs` runtime-backed path — runner/reporting layer. May be changed to feed/consume `trading-runtime`, compute metrics, and expose reports.
- `backtester/src/lib.rs` legacy `InMemoryExecutor` / engine-backed runner — transitional regression path. Do not extend with new semantics. Remove only after runtime-backed tests and #64 acceptance cover intended behavior.
- `backtester/src/plan.rs` — plan/research orchestration. It may orchestrate Runtime-backed backtests but must not create a second candle-by-candle trading semantics engine. Backtest Plan Rhai should use explicit constructors, typed host objects, and fluent methods for host APIs and returned plan results; the #16 raw-map plan shape is transitional smoke-test behavior and should not be extended. Plan scripting must not expose or parse strategy-facing Runtime decisions, portfolio transitions, or execution semantics.
- `backtester/src/main.rs` — CLI/runner surface only.
- `backtester/PRD-backtest-plan-engine.md` — historical context, not source of truth where it conflicts with runtime refactor decisions.

### `trading-daemon/` inventory

- `trading-daemon/src/live_engine.rs` — current live runtime feeder. This is runner/adapter code and may feed `TradingRuntime`; it must not own Portfolio/Execution semantics.
- `trading-daemon/src/order_executor.rs` — transitional old paper execution + DB persistence coupling. Do not use as canonical behavior. Persistence seam is owned by #37.
- `trading-daemon/src/warmup.rs` — transitional old-engine warmup helper. Do not extend for runtime warmup.
- `trading-daemon/src/config.rs` — runner-owned run configuration parsing. It may build RuntimeConfig, but strategy/runtime semantics remain in `trading-runtime`.
- `trading-daemon/src/cli.rs`, `trading-daemon/src/main.rs`, `trading-daemon/src/lib.rs` — runner/CLI composition only.

### `db-layer/` inventory

- `db-layer/src/client.rs` — DB client/connection adapter only.
- `db-layer/src/models.rs` — DB/domain mapping only. Mapping may adapt fields but must not calculate trading behavior.
- `db-layer/src/queries.rs` — DB query/reducer helpers only.
- `db-layer/src/error.rs`, `db-layer/src/lib.rs` — adapter crate surface only.
- `db-layer/tests/integration.rs` — DB behavior/mapping tests; not Runtime behavior tests.

## Deletion rule

Code may be removed when all are true:

1. The behavior is classified as donor or transitional in this document, or an issue explicitly marks it obsolete.
2. Equivalent intended behavior exists in the canonical target location, or the behavior is explicitly no longer desired.
3. Tests or issue acceptance criteria protect the intended behavior in the new location.
4. No active issue still names the old path as required implementation surface rather than donor/reference material.
5. The deletion does not hide an unresolved architecture decision. If unsure, stop and update this document or open/comment on the relevant issue.

## Agent guardrails

Agents working on this refactor must follow these rules:

- Before building on an old path, classify it using this document.
- If the path is donor material, copy/port behavior into the canonical target with tests; do not extend the donor as the target architecture.
- If the path is transitional, change it only as adapter/migration glue and only within the active issue scope.
- If an apparent gap is intentional, cite the issue that owns it. If no issue owns it, stop and ask for clarification.
- Do not add duplicate Portfolio/Execution semantics in `backtester` or `trading-daemon`.
- Keep Backtest Plan scripting as runner/research orchestration: datasets, run configuration, synthetic data preparation, Runtime-backed execution calls, validation, and reporting are allowed; strategy decisions, execution planning, portfolio transitions, and risk exits remain `trading-runtime` concerns.
- Backtest Plan dataset loading should name the Runtime Asset and visible Primary window only; Strategy Configuration / resolved RuntimeConfig supplies the Primary Timeframe and any Secondary Timeframes that the loader fetches automatically as context for the Runtime-backed run. Candle Timestamps are open/start timestamps for completed candle intervals. Secondary context ranges are derived from the visible Primary candle series: fetch required Secondary warmup before the first visible Primary candle, then fetch Secondary candles whose derived Candle Close Time is not after the last visible Primary candle's derived Candle Close Time.
- Synthetic Market Data / Monte Carlo mutation belongs in `backtester` as research data preparation before Runtime replay. Mutations may reorder, perturb, or regenerate copied candle datasets, but they must preserve candle invariants and feed the ordinary Runtime-backed backtest path; they must not add Portfolio/Execution semantics to `backtester` or special synthetic behavior to `trading-runtime`. The planned multi-timeframe Synthetic Market Data consistency model is lowest-timeframe-derived reaggregation: mutate the smallest configured timeframe and regenerate larger configured timeframes by OHLC aggregation. Grouped block permutation was considered in #92 and is not planned unless that issue is reopened or replaced by a new accepted methodology decision. Independent per-timeframe mutation is a weaker behavior only when explicitly scoped and documented. Monte Carlo iteration diagnostics may summarize Runtime output, including final equity, drawdown, trade count, blocked Strategy Tick count, and Runtime event counters, but the underlying semantics remain Runtime-owned. Reproducible Monte Carlo seeds should use a documented SplitMix64-based helper from `base_seed`, `iteration_index`, `stage_index`, and a stable `procedure_id`, not implementation-default RNG behavior.
- Do not reintroduce legacy `engine` strategy API compatibility unless an issue explicitly reopens ADR 0004 / ADR 0005.
- Do not mix External Account Snapshot behavior into Runtime Portfolio State unless #39 or a later accepted decision says so.
- Do not implement dynamic Position Risk Updates through `HOLD` or close decisions; #40 owns that semantics.
- Do not put DB IDs, reducer timing, cache polling, or SpacetimeDB details into `trading-runtime`.
- Prefer one canonical caller, one canonical executor, and many consumers over parallel implementations of the same semantics.

## Intentional open gaps

These are known gaps, not permission to invent local duplicate behavior:

- #36 — Rename `shared` to `domain` and remove runtime semantics. First implementation slice should be a mechanical crate/directory/import rename, with legacy contents carried only as explicitly transitional code. `shared::Signal` / `shared::TradeDecision` are legacy vocabulary, not the typed Rhai `decision::*` API, and should be excluded from future `domain`; delete them only after donor/transitional consumers (`engine`, legacy backtester executor, daemon paper executor/tests) are removed or migrated.
- #37 — Separate Runtime Events from DB persistence.
- #39 — External Account Snapshot / live account reconciliation.
- #40 — Position Risk Update Intents.
- #42 — Execution cost model for slippage, fees, and spread.
- #81 — Strategy State v2. Closed as not planned. V1 primitive, session-local Strategy State is intentional, not a missing feature. Do not add arrays/maps/host objects or restart persistence opportunistically. Real trading position restore is Portfolio State / Live Runner / DB persistence seam work, not Strategy State; see #37 and `.out-of-scope/strategy-state-v2-complex-persistent-state.md`. If richer strategy-authored scratch memory is needed later, open a focused issue with concrete examples.
- #82 — Additional Runtime Rhai indicator bindings. This is a Runtime Rhai adapter slice over existing pure `indicators` functions, not an indicator-algorithm refactor and not permission to make `indicators` depend on Rhai or `trading-runtime`. The v1 agent-ready scope is scalar-only bindings over `RhaiCandleHistory` / Market View histories: existing `sma`, plus `ema`, `dema`, `tema`, `slope`, `rsi`, `roc`, `cci`, `williams_r`, `atr`, `mfi`, and scalar `obv` with offset. Structured result objects such as `macd`, `bollinger`, `keltner`, `stochastic_*`, `adx`, `sar`, and `ichimoku` need a separate typed-result child issue. Session-/period-aware `vwap`, `pivot_points`, and `volume_profile` stay owned by #29. OBV history/series access stays owned by #30. Fibonacci's strategy-facing workflow stays out of #82 and is covered by #84 / ADR 0001, with #84 allowed to revise or supersede the current anchored direction.
- #83 — Market View candle history snapshot benchmark. The smoke measurement lives in `trading-runtime/tests/market_view_snapshot_cost.rs` and the 2026-06-01 result/decision is documented in `docs/refactor/rhai-market-view-snapshot-cost.md`: no optimization is required yet for expected live or ordinary backtest workloads. Do not implement caching, bounded histories, or borrowed views without a new accepted issue/design; any optimization design must preserve Rhai safety and runtime ownership.
- #84 — Anchored v2 / Market Structure redesign. Concept accepted; see ADR 0008 and `docs/refactor/runtime-market-structure.md`. Market Structure is a runtime-owned derived domain from Market State, exposed through Market View, with DB-free append-only Structure Annotations/Runtime output for explanation. Rhai strategies may read/select/filter active snapshots, but persistent Structure Object truth, lifecycle, and annotations remain owned by `trading-runtime`, not Strategy State, DB, runners, UI code, or old `engine` donor code. Long-term strategy-facing API should use explicit Market Structure language: `structure_config()` is the single declaration surface with one returned namespaced registry, no top-level auto-discovery, no duplicate handle-plus-registration path, and `market.structure.active("object_id")` for reads. Unknown object IDs should error; declared inactive objects return no active snapshots. Current `anchored` code is current runtime-facing behavior to reframe/port, not the term to expand indefinitely. ADR 0001's anchored-Fibonacci wording is partially superseded and must be revised or replaced before strategic Fibonacci implementation. #97 owns broader Python-/Pine-like Rhai programmability, persistent variable design, user modules, and Rust strategy/plugin alternatives.
- #85 — Rhai `::new` workaround cleanup. This is an isolated dependency/API hygiene slice, not a Runtime semantics change. Upgrade the workspace Rhai dependency to a minimum of 1.25, route all crate Rhai dependencies through the workspace dependency, and then re-check `module::new(...)` behavior. Use a minimum version constraint (`1.25`), not an exact pin; `Cargo.lock` provides the concrete resolved version. Acceptance must include dependency hygiene evidence such as one workspace Rhai resolution at `>=1.25` and direct crate dependencies using `rhai = { workspace = true }` where applicable. If `module::new(...)` still fails or is ambiguous, keep the approved constructor-normalization workaround and strengthen tests/comments, including string/comment preservation. If newer Rhai supports the desired syntax cleanly, remove the workaround while keeping the strategy-author API stable and proving the public constructor syntax loads without normalization. Rhai 1.25.0 fixes `AST::walk` traversal of `MethodCall` arguments, so the upgrade must include a warmup-detector regression test for indicator calls nested inside method-call arguments, such as a strategy writing an indicator value through `context.state.set(...)`.
- #87 — Per-timeframe warmup decision. Closed as not planned for now. V1 global effective warmup remains intentional: resolve one effective requirement from auto-detected indicator use, strategy-configured minimum warmup, and runtime minimum, then assign it to every configured timeframe. The `WarmupPlan` may stay keyed by timeframe as future-ready shape, but do not add strategy-facing per-timeframe warmup declarations until a concrete Strategy/Dataset need justifies typed config shape and merge rules; see `.out-of-scope/per-timeframe-strategy-warmup.md`. Any future per-timeframe design must clarify interactions with auto-detection, #18 warmup-aware dataset loading, and #64 Runtime-backed backtests.
- #97 — Pine-inspired Rhai strategy-authoring API design. Keep this as a human/design issue and separate from #84. Analyze Pine Script as a reference system for bar-by-bar strategy authoring, but produce a Rhai-native typed/fluent API proposal aligned with ADR 0005. The working namespace/API inventory lives in `docs/refactor/rhai-strategy-authoring-api.md`. The clarified v1 direction is Rhai-first: use an internal Strategy Adapter boundary around Rhai hook loading/validation, host API registration, Market View/Strategy Context wrappers, Strategy State bridging, and strategy-facing namespaces. Treat this as an internal module seam, not a public plugin framework. Native Rust strategies, dynamic Rust plugins, Pine compatibility, Pine parser work, general Rhai rewrites, and UI/manual drawing behavior are out of scope for #97 unless a later accepted issue/ADR reopens them. Read-only strategy-facing Portfolio Snapshot convenience helpers may be proposed in #97, but Portfolio State, Execution Planning, Portfolio Transition, Risk Exit, DB persistence, margin/buying-power, slippage/fees/spread, and dynamic risk-update semantics must remain outside #97. Strategy State work in #97 is ergonomics-only over the existing primitive, session-local v1 API; do not add arrays, maps, host objects, restart persistence, or new Strategy State semantics under #97. Strategy authoring in #97 should assume a single strategy file with user-authored helper functions; multi-file Rhai imports, user library packaging, path resolution, sandboxing, and live/backtest deployment of shared strategy libraries are follow-up design concerns, not v1 #97 scope. The strategy-facing technical-analysis namespace should move toward canonical `ta::*` for author ergonomics; the existing `indicators::*` strategy namespace may remain only as a transitional alias with warmup detection recognizing both while the alias exists. New docs/examples should use `ta::*`. The Rust `indicators` crate remains the pure implementation crate and must not gain Rhai/runtime dependencies.

## How to update this document during grilling

When a grilling session resolves a boundary or migration question, update this document immediately if the answer changes one of these:

- Which crate/module is canonical for a behavior.
- Whether an old path is donor, transitional, canonical, or removable.
- Which issue owns a known gap.
- Whether agents may build on, port from, or delete a path.
- The deletion rule for a specific old module.

Do not add domain definitions here; update `CONTEXT.md` instead.
Do not record hard-to-reverse architectural trade-offs only here; create or update an ADR when the ADR criteria are met.
