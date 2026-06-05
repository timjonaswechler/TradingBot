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
- `domain` is the value-type crate produced by the mechanical `shared` → `domain` rename. It may temporarily carry legacy value/decision/context helpers while #36 cleanup continues, but it must not gain new Runtime semantics.

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

- No active donor crate remains for the old strategy engine. The legacy `engine/` crate was removed in #107 after the runtime-backed strategy, warmup, market-view, anchored, portfolio, risk-exit, and backtest paths became the productive paths.
- Historical references to the old engine in ADRs and refactor notes remain donor/history context only. Do not recreate the legacy `engine` crate or old `on_tick(candles, context)` compatibility without reopening ADR 0004 / ADR 0005.

### Transitional / treat with caution

- `domain/`
  - Value-type crate produced by the mechanical `shared` -> `domain` rename while #36 semantic cleanup continues.
  - Existing pure value types may remain temporarily.
  - Legacy non-domain helpers are carried only as transitional code until follow-up cleanup issues remove or migrate them; the legacy context helper was removed in #112.
  - New Runtime semantics must not be added here.
  - Helpers that express Portfolio/Execution behavior should migrate into `trading-runtime`.

- Legacy backtester engine-backed runner paths
  - Removed in #107. Backtester productive code now uses runtime-backed replay/reporting APIs.
  - Do not recreate `backtester::run_backtest(&mut Engine, ...)` or `InMemoryExecutor`; backtest behavior should feed `trading-runtime` and derive reports from Runtime Events/snapshots.

- Legacy daemon `PaperExecutor` path
  - Removed in #118 after runtime-backed Paper Trading restore/projection and live runner wiring became the active Paper Trading path.
  - Do not recreate `trading-daemon/src/order_executor.rs` or build Paper Trading on legacy `Signal` / `TradeDecision` execution helpers; Paper Trading transitions come from `trading-runtime` and are projected by `trading-daemon/src/paper_trading_persistence.rs`.

- `trading-daemon/src/warmup.rs`
  - Removed in #107 after live warmup moved to the runtime-feeding path in `trading-daemon/src/live_engine.rs`.
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
- `trading-runtime/src/decision.rs` — canonical Strategy Decision intent model. Do not move Strategy Decision into future `domain`; it is a runtime API contract interpreted by Execution Planning and exposed to Rhai through typed `decision::*`.
- `trading-runtime/src/execution.rs` — canonical Execution Planning. Planning does not own DB persistence.
- `trading-runtime/src/portfolio.rs` — canonical runtime-local Portfolio State and Portfolio Transitions. It consumes passive `domain::OpenPosition` / `domain::ClosedPosition` values, but transition logic and PnL/equity calculation remain here.
- `trading-runtime/src/risk_exit.rs` — canonical runtime-managed Risk Exit evaluation.
- `trading-runtime/src/events.rs` — canonical DB-free Runtime Events. Persistence mapping belongs outside.
- `trading-runtime/src/step.rs` — canonical ordered runtime step output.
- `trading-runtime/src/anchored.rs` — canonical runtime-facing anchored/structure-aware behavior already ported from old donor concepts.

### Removed legacy `engine/` inventory

#107 removed the legacy `engine/` crate and old engine-backed paths. Desired donor behavior is now protected by runtime-backed tests or documented as intentionally removed:

- Strategy loading, typed Rhai decisions/configuration, Market View access, Strategy Context/State, and indicator binding behavior are covered in `trading-runtime` Rhai strategy tests and strategy example tests.
- Warmup detection/resolution and live/backtest warmup feeding are covered in `trading-runtime` warmup/market-input tests and runtime-backed `backtester` tests.
- Anchored/structure-facing behavior lives in `trading-runtime/src/anchored.rs` and related runtime tests; the old `engine/src/anchored.rs` path is gone.
- Portfolio transitions, long/short realized PnL, risk exits, equity snapshots, and force close behavior are covered by `trading-runtime` portfolio/runtime/risk-exit tests.
- The old legacy Rhai contract `on_tick(candles, context)` and signal/size map compatibility are intentionally not preserved under ADR 0004 and ADR 0005.

### `domain/` transitional inventory

#36 sequencing decision: the first `shared` → `domain` slice is a full mechanical rename with no compatibility alias/wrapper: rename the directory, package name, workspace member/dependencies, and Rust imports from `shared` to `domain`. Legacy modules may be temporarily carried along under the new crate only as explicitly transitional code. Semantic cleanup must follow in separate test-protected slices using the classifications below; do not treat temporarily carried legacy modules as accepted future-domain contents. Acceptance for the mechanical rename includes no `shared/` directory, no `shared` package/dependency/imports in productive code, no alias/wrapper crate, `cargo test --workspace`, formatting/check-diff validation, and an explicit search proving remaining `shared` references are only intentional historical documentation if any.

- `domain/src/candle.rs` — temporary value type that should remain in future `domain`. Pure value helpers such as `body`, `range`, or derived `close_time` are allowed because they describe the candle value itself. Runtime/runner interpretations such as stop-hit checks, freshness/readiness, Strategy Tick triggering, or dataset orchestration do not belong here.
- `domain/src/timeframe.rs` — temporary value type that should remain in future `domain`. Pure parsing/display/duration helpers are allowed; runtime timeframe contracts, Secondary readiness policies, warmup plans, and dataset loading policy belong outside `domain`.
- `domain/src/position.rs` — passive value types that should remain in future `domain`: `PositionSide`, `OpenPosition`, `ClosedPosition`, and `EntryRiskParameters`. `PositionSide` keeps passive formatting/serde helpers only. `OpenPosition` contains `PositionSide`, Runtime Asset/symbol, entry price, `quantity`, entry time, and grouped Entry Risk Parameters. `ClosedPosition` contains the closed `OpenPosition`, exit price, exit time, and realized PnL as a result value. `EntryRiskParameters` groups optional stop-loss and take-profit prices as data only. These values must stay field/data-only: no PnL/equity helpers, no close/open/update methods, no stop/take-profit validation or evaluation, and no Portfolio Transition behavior. Runtime-specific Portfolio Snapshot, Portfolio Transition, PnL/equity calculation, Entry Risk validation, and Risk Exit semantics belong in `trading-runtime`.
- `domain/src/signal.rs` — legacy decision/signal vocabulary (`Signal` / `TradeDecision`). Do not include it in the future `domain` crate and do not build new strategy semantics on it; target is `trading-runtime/src/decision.rs` (`StrategyDecision` plus typed Rhai `decision::*`). It is removable after old donor/transitional consumers are retired or migrated to canonical runtime Strategy Decisions.
- `domain/src/context.rs` — removed in #112. The legacy strategy context shape is no longer exported from `domain`. The canonical Strategy Context is runtime-owned in `trading-runtime`; it may expose Runtime Portfolio Snapshot, Strategy State, and limited session metadata, but not Market Data, runner policy, DB fields, or Portfolio Transition behavior.
- `domain/src/executor.rs` — transitional/removable Portfolio/Execution helpers. Do not include an executor module in the future `domain` crate and do not add new behavior here. `domain::plan_action` and `domain::Action` are legacy planning over `Signal`; canonical Execution Planning is `trading-runtime::plan_execution` over `StrategyDecision`. `domain::realized_pnl` is legacy Portfolio Transition/PnL semantics; canonical runtime PnL/equity calculation now lives internally in `trading-runtime/src/portfolio.rs`. After #118 these helpers no longer have a productive daemon Paper Trading consumer; their remaining cleanup is owned by #36. Realized PnL remains visible as runtime output (for example `ClosedPosition.realized_pnl` / Runtime Events), but PnL/equity formulas should not be exposed as public `domain` helper APIs.

### `backtester/` inventory

- `backtester/src/lib.rs` runtime-backed path — runner/reporting layer. May be changed to feed/consume `trading-runtime`, compute metrics, and expose reports.
- `backtester/src/lib.rs` legacy `InMemoryExecutor` / engine-backed runner — removed in #107. Do not recreate it; runtime-backed backtests should feed `trading-runtime` and derive reports from Runtime Events/snapshots.
- `backtester/src/plan.rs` — plan/research orchestration. It may orchestrate Runtime-backed backtests but must not create a second candle-by-candle trading semantics engine. Backtest Plan Rhai should use explicit constructors, typed host objects, and fluent methods for host APIs and returned plan results; the #16 raw-map plan shape is transitional smoke-test behavior and should not be extended. Plan scripting must not expose or parse strategy-facing Runtime decisions, portfolio transitions, or execution semantics.
- `backtester/src/main.rs` — CLI/runner surface only.
- `backtester/PRD-backtest-plan-engine.md` — historical context, not source of truth where it conflicts with runtime refactor decisions.

### `trading-daemon/` inventory

- `trading-daemon/src/live_engine.rs` — current live runtime feeder. This is runner/adapter code and may feed `TradingRuntime`; it must not own Portfolio/Execution semantics.
- `trading-daemon/src/order_executor.rs` — removed in #118. Do not recreate the legacy PaperExecutor execution + DB persistence coupling; Paper Trading uses `trading-runtime` plus `trading-daemon/src/paper_trading_persistence.rs`.
- `trading-daemon/src/warmup.rs` — removed in #107 after live warmup moved to the runtime-feeding path in `trading-daemon/src/live_engine.rs`. Runtime warmup behavior belongs in `trading-runtime`; runner fetching policy belongs in `trading-daemon`.
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
- Do not turn Runtime Events into a persisted event store in V1. Paper Trading projects selected runtime-semantic outputs through the Paper Trading Persistence Adapter into dedicated paper persistence records unless a separate accepted decision reopens event sourcing.
- Prefer one canonical caller, one canonical executor, and many consumers over parallel implementations of the same semantics.

## Intentional open gaps

These are known gaps, not permission to invent local duplicate behavior:

- #36 — Rename `shared` to `domain` and remove runtime semantics. First implementation slice should be a mechanical crate/directory/import rename, with legacy contents carried only as explicitly transitional code. `domain::Signal` / `domain::TradeDecision` are legacy vocabulary, not the typed Rhai `decision::*` API, and should be excluded from future `domain`; after #118 the daemon Paper Trading path no longer consumes the legacy paper executor/tests, so remaining `domain` signal/executor cleanup is owned by #36.
- #37 — Separate Runtime Events from DB persistence. The accepted V1 direction is projection, not event sourcing: `trading-runtime` emits DB-free ordered Runtime Events, and the Live Runner projects selected semantic outputs through a Paper Trading Persistence Adapter into persistence records. Raw Runtime Events are not themselves the persisted source of truth unless a later accepted decision explicitly introduces an event store. V1 projection of `PositionOpened` and `PositionClosed` into live persistence is scoped to Paper Trading / Simulated Execution, where the Runtime creates the portfolio truth. For Real-Money Live execution, the Broker/Trading Provider is the source of truth for actual exposure; DB writes may be cache/audit/metadata only and must not become authoritative Runtime restore without #39 / broker-execution reconciliation decisions. Other events, including `PortfolioUpdated`, may be logged or counted, but must not drive Paper Trading position/trade persistence without a follow-up issue. V1 Paper Trading persistence does not add portfolio/equity snapshot tables; restart derives realized cash from configured starting balance plus persisted paper trade PnL and open exposure from `paper_open_positions`. The target daemon adapter module for the V1 projection and Paper Trading restore should be separate from the live runner orchestration, e.g. `paper_trading_persistence`, rather than embedding RuntimeEvent-to-DB mapping directly in `live_engine` or `db-layer`. It should call low-level `db-layer` helpers, build initial runtime-local `PortfolioState` for Paper Trading, and project selected runtime events, while keeping DB IDs/reducer details out of `trading-runtime`. `db-layer` may expose paper-specific query/reducer helpers such as open-paper-position, record-paper-position-closed, get-paper-open-position, and get-paper-trades, but it must not interpret `RuntimeStep`/`RuntimeEvent` or restore `PortfolioState` itself. Paper Trading projection recovery should not rely on application log replay; use idempotent projection operations with deterministic keys, and make the close-position-plus-insert-trade projection atomic through a single DB reducer where the DB adapter can support it (for example `record_paper_position_closed`, which deduplicates the completed trade, validates/removes the matching open position, and inserts the paper trade in one operation). Paper Trading projection should use dedicated Paper Trading persistence tables rather than treating the existing `live_positions` / `live_trades` tables as the V1 target. The target table names are `paper_open_positions` for currently open Paper Trading positions and `paper_trades` for completed Paper Trading positions. `paper_open_positions` should enforce at most one open Paper Trading position per Strategy × Runtime Asset, matching the Runtime V1 one-open-position invariant. This is a deliberate scope correction: new paper-specific tables let the adapter use projection keys, typed exit categories, and atomic close-plus-trade reducers without implying those records are authoritative Real-Money broker truth. Existing `live_positions` / `live_trades` remain transitional legacy storage for old data/admin compatibility; the old PaperExecutor path was removed in #118 after replacement and deletion criteria were met. Paper Trading restore should derive initial runtime-local Portfolio State from configured starting balance plus persisted completed trade PnL, persisted completed trade count, and any persisted open position for the strategy/runtime asset; this restore rule does not apply to Real-Money Live execution. Numeric sentinel values such as `0.0` for missing stop-loss/take-profit should not be carried into the new paper persistence schema: runtime, domain values, Paper Trading Persistence Adapter interfaces, and paper-specific DB records should use nullable/optional Entry Risk Parameters. Legacy sentinel conversion remains isolated to legacy `live_*` DB mapping while those paths exist. In Paper Trading, portfolio-transition projection is part of the simulated live truth: transient DB failures may be retried, but an unconfirmed `PositionOpened`/`PositionClosed` projection must stop the Paper Live Runner before it processes further Tradable Candles. Portfolio-transition projection for one Runtime Session should be synchronously confirmed before the runner feeds that Runtime Session another Tradable Candle; diagnostic logging/counting may remain asynchronous because it is not authoritative persistence. The Paper Trading Persistence Adapter should process persistable events in `RuntimeStep` order and must not reorder runtime output; V1 Runtime behavior should still produce at most one portfolio transition per step. `PositionOpened` projection is idempotent only when an existing persisted open position matches the same deterministic projection key / position data; a different existing open position for the same Strategy × Runtime Asset is a fatal Paper Trading persistence inconsistency. `PositionClosed` projection is idempotent when the corresponding completed trade projection already exists under the same deterministic key; otherwise it requires a matching persisted open position, and absence of both matching open position and completed trade is a fatal Paper Trading persistence inconsistency. Projection keys should be deterministic hashes over canonical runtime data rather than Runtime Event IDs: for open positions, include a version tag plus Strategy Identity, Runtime Asset, side, entry time, entry price, and quantity; for completed trades, include those open-position identity fields plus exit time, exit price, realized PnL, and typed exit kind. The Paper Trading Persistence Adapter should connect `PositionClosed` to the persisted open row by recomputing the open-position projection key from `closed_position.position`, with Strategy Identity × Runtime Asset as a supporting uniqueness boundary; Runtime state must not carry DB auto-increment IDs. Entry Risk Parameters are persisted data to compare, not primary identity fields. Paper Trading Persistence keys and restore should use an operator-owned Strategy Identity from Run Configuration plus Runtime Asset; strategy file paths and source-code hashes may be metadata, but not primary identity. Persistent Paper Trading requires an explicit Strategy Identity in Run Configuration; non-persistent backtests may omit it. Live Runner configuration must distinguish Paper Trading from future Real-Money Live execution. Paper Trading mode uses Simulated Execution Runtime transitions plus the Paper Trading Persistence Adapter; Real-Money mode must not reuse this projection as broker truth. The Trading Runtime itself must not know whether a Runner is paper, backtest, or Real-Money live; runners/adapters provide the appropriate Runtime inputs or commands so all modes share one runtime boundary without embedding broker/DB mode flags in `trading-runtime`. Paper persistence identity should use `runtime_asset` as the canonical asset field; provider/display `symbol` may be stored as metadata but should not replace Runtime Asset in projection keys or restore queries. V1 `paper_open_positions` should store projection key, Strategy Identity, Runtime Asset, side, entry price, quantity, entry time, optional stop-loss, optional take-profit, and optional entry metadata such as reason/source path; it should not store broker order/fill IDs or exit information. V1 `paper_trades` should store projection key, Strategy Identity, Runtime Asset, side, entry price, exit price, quantity, realized PnL, entry time, exit time, optional stop-loss, optional take-profit, typed exit kind (strategy exit, risk exit stop-loss, risk exit take-profit, or force close), and optional entry/exit metadata; it should not store broker order/fill IDs or a raw Runtime Event blob as source of truth. Typed exit semantics are required; human-readable entry/exit reasons are optional metadata, and if richer reasons are needed they should be added as explicit Runtime output fields in a small follow-up rather than inferred from neighboring events.
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
