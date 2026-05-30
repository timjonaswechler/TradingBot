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
- `trading-runtime/src/market_input.rs` — canonical runtime input boundary and run configuration for Primary/Secondary timeframes.
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

- `shared/src/candle.rs` — temporary value type; likely remains in future `domain` unless #36 decides otherwise.
- `shared/src/timeframe.rs` — temporary value type; likely remains in future `domain` unless #36 decides otherwise.
- `shared/src/position.rs` — temporary value type. Runtime-specific Portfolio Snapshot/Transition semantics belong in `trading-runtime`.
- `shared/src/signal.rs` — legacy decision/signal vocabulary. Do not build new strategy semantics on it; target is `trading-runtime/src/decision.rs`.
- `shared/src/context.rs` — legacy strategy context shape. Do not extend; target Strategy Context is runtime-owned.
- `shared/src/executor.rs` — transitional Portfolio/Execution helpers. Do not add new behavior; target is `trading-runtime/src/execution.rs` and `trading-runtime/src/portfolio.rs`.

### `backtester/` inventory

- `backtester/src/lib.rs` runtime-backed path — runner/reporting layer. May be changed to feed/consume `trading-runtime`, compute metrics, and expose reports.
- `backtester/src/lib.rs` legacy `InMemoryExecutor` / engine-backed runner — transitional regression path. Do not extend with new semantics. Remove only after runtime-backed tests and #64 acceptance cover intended behavior.
- `backtester/src/plan.rs` — plan/research orchestration. It may orchestrate backtests but must not create a second candle-by-candle trading semantics engine.
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
- Do not reintroduce legacy `engine` strategy API compatibility unless an issue explicitly reopens ADR 0004 / ADR 0005.
- Do not mix External Account Snapshot behavior into Runtime Portfolio State unless #39 or a later accepted decision says so.
- Do not implement dynamic Position Risk Updates through `HOLD` or close decisions; #40 owns that semantics.
- Do not put DB IDs, reducer timing, cache polling, or SpacetimeDB details into `trading-runtime`.
- Prefer one canonical caller, one canonical executor, and many consumers over parallel implementations of the same semantics.

## Intentional open gaps

These are known gaps, not permission to invent local duplicate behavior:

- #36 — Rename `shared` to `domain` and remove runtime semantics.
- #37 — Separate Runtime Events from DB persistence.
- #39 — External Account Snapshot / live account reconciliation.
- #40 — Position Risk Update Intents.
- #42 — Execution cost model for slippage, fees, and spread.
- #65 — Maintainer/runner/backtester documentation after runtime integration.
- #67 — Protective Runner Shutdown policy.
- #81 — Strategy State v2.
- #82 — Additional Runtime Rhai indicator bindings.
- #83 — Market View candle history snapshot benchmark.
- #84 — Anchored v2.
- #85 — Rhai `::new` workaround cleanup.
- #87 — Per-timeframe warmup decision.

## How to update this document during grilling

When a grilling session resolves a boundary or migration question, update this document immediately if the answer changes one of these:

- Which crate/module is canonical for a behavior.
- Whether an old path is donor, transitional, canonical, or removable.
- Which issue owns a known gap.
- Whether agents may build on, port from, or delete a path.
- The deletion rule for a specific old module.

Do not add domain definitions here; update `CONTEXT.md` instead.
Do not record hard-to-reverse architectural trade-offs only here; create or update an ADR when the ADR criteria are met.
