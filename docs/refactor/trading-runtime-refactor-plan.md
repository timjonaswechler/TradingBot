# Trading Runtime Refactor Plan

## Problem Statement

TradingBot2 currently splits trading semantics across several places: Rhai strategy execution lives in the current engine crate, live paper execution and persistence live in the daemon, and backtest execution/portfolio logic live in the backtester. This makes it easy for live trading and backtesting semantics to drift apart.

The goal is to rebuild the backend architecture around one shared Trading Runtime. Live and backtest runs should differ only in their market data source and runner; strategy evaluation, portfolio transitions, execution semantics, warmup handling, and event flow should be shared.

Existing architecture documents are not treated as source of truth for this refactor. This plan is the new working source for the runtime cleanup.

Use `docs/refactor/runtime-migration-control.md` as the operational control map for agents and maintainers during the migration: it classifies code as canonical, donor, transitional, or removable and records which intentional gaps are owned by follow-up issues.

## Solution

Create a clean target architecture with these responsibilities:

- `domain`: pure domain/value types such as candles, positions, signals, decisions, and later orders/fills.
- `indicators`: pure indicator functions with no runtime, Rhai, DB, or IO dependency.
- `trading-runtime`: the heart of the system. It owns runtime/event flow, Rhai strategy handling, strategy hooks, market state, market view, strategy state, compute/indicator state, portfolio state, execution semantics, and trading events. It should be built as a new runtime module/crate rather than by mechanically renaming the current `engine` crate; existing engine code is a donor for strategy-handling pieces, not the target architecture itself.
- `trading-daemon`: live runner only. It owns CLI/config, timers, provider fetching, DB writes/reads, broker/IO adapters, and calls the Trading Runtime.
- `backtester`: backtest runner/reporting only. It loads historical data, feeds the Trading Runtime, and computes reports/metrics/research results. It must not own trading semantics.
- `db-layer`: SpacetimeDB adapter only. It owns generated bindings, queries, reducers, and DB/domain mapping. It must not own trading semantics.
- `trading-ui`: later GPUI app for charts and results; out of scope for this refactor.

The current engine crate should not remain as a separate legacy strategy-engine crate. A new `trading-runtime` should be built explicitly, and the current engine's Rhai strategy handling should be transferred into it as an internal component when that part of the runtime is reached.

## Key Decisions

- Live and backtest must use the same Trading Runtime.
- Live and backtest differ by runner and market data source, not by strategy/execution semantics.
- Runners feed market input and explicit runtime commands into the Trading Runtime; they must not call strategy evaluation and execution as separate public steps.
- The Trading Runtime manages one runtime asset at first.
- Multi-timeframe support is in scope; multi-asset portfolio coordination is later.
- A strategy has one Primary Timeframe that triggers tradable ticks.
- Secondary Timeframes provide context but do not independently trigger trades in the first version.
- Strategy-facing market data should be exposed through a Market View, not through the portfolio/runtime context.
- Target Rhai API should move toward `on_tick(market, context)`.
- New strategy decisions should use explicit direction-aware intents: `HOLD`, `OPEN_LONG`, `CLOSE_LONG`, `OPEN_SHORT`, and `CLOSE_SHORT`, rather than the old `BUY`, `SELL`, `SHORT`, `COVER` signal names.
- Opening strategy decisions should specify `quantity` as asset units/contracts. The first runtime version should not interpret ambiguous `size` or calculate quantity from notional/balance fractions; strategies can calculate quantity from `context` and `market` themselves.
- `stop_loss` and `take_profit` on opening strategy decisions are Entry Risk Parameters. `HOLD` and close decisions must not implicitly update stops or targets; dynamic risk updates are tracked separately in #40.
- `market.candle()` and `market.candles()` refer to the Primary Timeframe.
- `market.candle("1h")` and `market.candles("1h")` refer to Secondary Timeframes.
- `context` is for Portfolio State, Strategy State, and runtime-visible session information.
- Strategy Hooks are named optional functions called by the runtime at defined lifecycle points.
- Warmup should be derived from strategy indicator usage where possible.
- Effective warmup should be the maximum of detected strategy warmup, user/configured minimum warmup, and runtime minimum warmup. User configuration may increase warmup but must not lower the detected requirement.
- Strategy Configuration owns the runtime timeframe contract: exactly one Primary Timeframe plus any Secondary-Timeframe requirements/defaults and minimum warmup. Run Configuration owns symbol/provider/mode/source, portfolio inputs, and runner policies.
- GPU/compute-shader work belongs to later Batch Compute / research workflows, not the live tradable tick path.
- Runtime output should be ordered, DB-free Runtime Events plus any necessary runtime snapshot data. DB IDs, reducer/cache timing, and backtest metrics stay outside the runtime output.
- Runtime-local Portfolio State uses realized-cash semantics in the first version: opening a position does not subtract/reserve notional from cash balance; closing a position applies realized PnL to cash balance. More realistic buying-power, margin, and reservation behavior belongs to later account snapshot / portfolio-coordinator work.
- Event ordering inside a `RuntimeStep` should be deterministic. A tradable primary tick should emit market input, tick start, strategy decision, execution action, portfolio transition if any, portfolio update if any, and tick completion in order.
- Existing old docs should not be extensively patched; replace them with a smaller set of clearer source-of-truth documents.

## Tiny-Commit Implementation Plan

### Phase 1: Establish clean names and boundaries

1. Add the new runtime/refactor decision document as the source of truth.
2. Add or update the glossary so the terms Strategy State, Portfolio State, Trading Runtime, Market State, Market View, Primary Timeframe, Secondary Timeframe, Runtime Asset, Compute State, and Batch Compute are stable.
3. Rename `shared` to `domain` and update workspace references.
4. Keep domain limited to pure value types and tiny pure helpers.
5. Move any execution/state-transition helpers out of domain into the future runtime boundary.
6. Run the full test suite after the rename before changing behavior.

### Phase 2: Create the new trading-runtime crate

7. Create a new `trading-runtime` crate/module as the explicit target architecture rather than mechanically renaming the current `engine` crate.
8. Add the runtime core, event flow, market input boundary, and runtime-local portfolio skeleton in the new crate first.
9. Treat the current `engine` crate as a temporary donor for Rhai strategy handling, warmup detection, indicator bindings, anchored runtime, and strategy state behavior.
10. Transfer engine pieces into `trading-runtime` only when the corresponding runtime module is ready, preserving behavior with tests.
11. Do not make `trading-runtime` depend permanently on `engine`; the `engine` crate should be removed or fully absorbed once migration is complete.

### Phase 3: Introduce event and runtime core

12. Add core event types for market input, strategy output, execution actions, portfolio transitions, portfolio updates, and runtime diagnostics.
13. Add a `TradingRuntime` type that owns market state, strategy handling, strategy state, runtime-local portfolio state, execution state, and emitted events.
14. Add `on_primary_candle(candle)` as the first entrypoint for completed Primary Timeframe candles.
15. Add `force_close(mark_candle, reason)` as the explicit runtime command used by runner policies such as shutdown liquidation.
16. Ensure a single runtime instance handles one Runtime Asset and one runtime-local portfolio snapshot.
17. Keep the first event flow deterministic and ordered.
18. Add behavior tests for HOLD while flat, BUY→SELL long flow, and SHORT→COVER short flow.

### Phase 4: Market State and Market View

18. Replace the single `candles` argument model with a Market View concept.
19. Add Primary Timeframe candle history to Market State.
20. Add Secondary Timeframe storage to Market State without making it trigger trades.
21. Expose `market.candle()` and `market.candles()` for the Primary Timeframe.
22. Expose timeframe-specific candle access for Secondary Timeframes.
23. Update Rhai strategy tests to target `on_tick(market, context)`.
24. Remove the old `on_tick(candles, context)` contract once the new tests and examples are in place.

### Phase 5: Strategy config, hooks, and warmup

25. Add a required Strategy Configuration extraction step for typed strategy loading.
26. Let Strategy Configuration declare exactly one Primary Timeframe plus optional Secondary-Timeframe requirements/defaults.
27. Let Strategy Configuration declare a minimum warmup requirement.
28. Preserve automatic warmup detection from indicator usage.
29. Define effective warmup as the maximum of detected strategy warmup, strategy-configured minimum warmup, and runtime minimum warmup; Strategy Configuration may increase warmup but must not lower detected requirements.
30. Add optional non-timeframe Strategy Hook detection with default/no-op behavior.
31. Keep `on_tick` as the primary required tick hook unless a later explicit decision changes this.
32. Add tests for missing Strategy Configuration, missing/duplicate Primary Timeframe declarations, invalid Secondary declarations, optional hooks, strategy-declared warmup, and auto-detected warmup.

### Phase 6: Portfolio State and execution semantics

33. Introduce Portfolio State inside `trading-runtime`.
34. Move open-position, close-position, PnL, stop-loss, take-profit, and trade-count semantics into runtime-owned code.
35. Represent strategy output as runtime events or runtime decisions before applying portfolio/execution transitions.
36. Implement paper/in-memory execution as a runtime execution mode.
37. Make backtest and live paper execution use the same transition logic.
38. Add tests for long open/close, short open/close, stop-loss, take-profit, ignored duplicate open signals, and equity updates.

### Phase 7: Backtester becomes a runner/reporting layer

39. Replace backtester-owned execution/portfolio logic with calls into `TradingRuntime`.
40. Keep backtester responsibility limited to loading candles, feeding them in order, collecting runtime output, and producing metrics/reports.
41. Preserve existing backtest result semantics where they are intentional.
42. Add regression tests comparing old expected trade/equity scenarios against the runtime-backed runner.
43. Remove obsolete backtester execution state once runtime-backed tests pass.

### Phase 8: Daemon becomes a live runner/IO layer

44. Replace daemon-owned paper execution semantics with calls into `TradingRuntime`.
45. Keep daemon responsibility limited to config, timer loop, provider fetch, DB writes, DB reads, broker/IO adapters, logging, and shutdown.
46. On live startup, restore required Portfolio State from DB through db-layer mapping, then initialize Trading Runtime.
47. During live ticks, fetch/persist market data outside the runtime, then feed completed candles into the runtime.
48. Persist runtime-emitted position/trade changes through db-layer.
49. Remove obsolete daemon paper execution state once runtime-backed integration tests pass.

### Phase 9: DB layer boundary cleanup

50. Keep db-layer focused on SpacetimeDB bindings, queries, reducer calls, and DB/domain mapping.
51. Rename mappings from shared terminology to domain terminology.
52. Ensure db-layer does not calculate strategy decisions, stops, PnL, or portfolio transitions.
53. Keep DB integration tests focused on storage behavior and mappings only.

### Phase 10: Docs, examples, and old-doc cleanup

54. Create a small current architecture document based on the new source-of-truth terms.
55. Update strategy reference docs for `on_tick(market, context)`, Market View, Strategy Hooks, and warmup behavior.
56. Update example strategies to the new API.
57. Archive or delete obsolete architecture/design docs after the new docs exist.
58. Run full tests and CLI smoke tests.
59. Open follow-up issues for Batch Compute, GPU research acceleration, multi-asset portfolio coordination, UI/GPUI, and advanced plugin/scheduling APIs.

## Testing Decisions

Good tests should assert external behavior rather than private module structure. Tests should prove that live-style and backtest-style runners use the same runtime semantics.

Test coverage should include:

- Domain value-type behavior.
- Indicator behavior and warmup semantics.
- Rhai strategy loading, hook detection, and strategy config extraction.
- Market View access for Primary and Secondary Timeframes.
- Automatic and strategy-declared warmup resolution.
- Runtime event flow for hold, buy, sell, short, cover, stop-loss, and take-profit.
- Portfolio State transitions and equity/PnL updates.
- Backtest runner output using the runtime.
- Daemon persistence boundaries using mocked or integration DB behavior.
- DB-layer mappings and reducer/query behavior.

Prior art already exists in engine tests for Rhai execution/warmup/state, backtester tests for trade/equity outcomes, daemon tests for paper execution, and db-layer integration tests for storage/mapping behavior. These should be migrated toward the new runtime boundaries rather than duplicated indefinitely.

## Out of Scope

- Full GPUI trading UI.
- Multi-asset portfolio coordination.
- GPU/compute-shader execution in the live tick path.
- Full plugin framework or Bevy-style ECS.
- Broker-specific real-money execution beyond adapter boundaries.
- Large old-doc cleanup before the new source-of-truth docs exist.
- Keeping the old engine API as a long-term compatibility layer.

## Further Notes

This refactor should be done in small commits, but not as a throwaway intermediate architecture. Each commit should move directly toward the target model and leave the workspace compiling where practical. Temporary compatibility is acceptable only inside an active migration step and should be removed before considering the refactor complete.
