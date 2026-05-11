## Problem Statement

TradingBot2 kann heute Strategien nur gegen genau einen Candle-Stream pro Engine-Instanz ausführen. Als Strategie-Autor kann ich deshalb kein Setup ausdrücken, das Signale auf einem Primärintervall handelt und gleichzeitig Filter oder Bestätigung auf höheren oder niedrigeren Intervallen nutzt. Typische Multi-Timeframe-Workflows wie „trade auf 1h, filtere mit 1d-Trend“ oder „entry auf 15m nur wenn 4h-Struktur bullisch ist“ sind dadurch weder im Backtester noch im Live-Daemon sauber möglich. Außerdem besteht bei einer naiven Umsetzung die Gefahr von Look-Ahead-Bias, Cache-Vermischung zwischen Intervallen und Abweichungen zwischen Backtest- und Live-Verhalten.

## Solution

TradingBot2 bekommt Multi-Timeframe-Strategieunterstützung mit einem klaren Primärintervall und beliebig vielen zusätzlichen Referenzintervallen. Die Engine verwaltet mehrere abgeschlossene Candle-Serien pro Symbol, exponiert sie in Rhai über eine timeframe-bewusste API und führt `on_tick` nur auf dem Primärintervall aus. Höhere oder niedrigere Referenzintervalle sind dabei nur als bereits geschlossene Bars sichtbar, damit Backtests und Live-Trading dieselben Entscheidungen sehen und kein Look-Ahead-Bias entsteht. Backtester und Live-Daemon werden auf dieselbe Multi-Timeframe-Engine-Orchestrierung umgestellt, damit eine Strategie dieselbe Semantik in beiden Pfaden hat.

## User Stories

1. As a strategy author, I want to define a primary trading timeframe, so that signal execution has a single unambiguous cadence.
2. As a strategy author, I want to read candles from a higher timeframe inside the same strategy, so that I can gate entries with trend confirmation.
3. As a strategy author, I want to read candles from a lower timeframe inside the same strategy, so that I can build finer-grained entry logic without splitting my strategy across binaries.
4. As a strategy author, I want indicators on each timeframe to be isolated from each other, so that an EMA on `1d` never contaminates an EMA on `1h`.
5. As a strategy author, I want the same Rhai strategy file to run in the backtester and the live daemon, so that I do not maintain two strategy implementations.
6. As a strategy author, I want higher-timeframe data to become visible only after that candle has closed, so that my backtests do not cheat.
7. As a strategy author, I want warmup to account for all referenced timeframes, so that the strategy does not start with partially valid context.
8. As a strategy author, I want strategy code to access other timeframes through a small, predictable API, so that multi-timeframe logic remains readable.
9. As a strategy author, I want multi-timeframe strategies to work with anchored indicators too, so that event-driven structure tools remain usable.
10. As a backtester user, I want a backtest run to load and synchronize multiple candle series, so that reported trades reflect the full strategy context.
11. As a backtester user, I want the event merge rules to be deterministic, so that repeated runs over the same data produce identical results.
12. As a backtester user, I want a clear error when a referenced timeframe has insufficient data, so that I can fix seeding gaps quickly.
13. As a live trading user, I want the daemon to subscribe to all intervals required by one strategy instance, so that live decisions match the backtest model.
14. As a live trading user, I want only one strategy instance per symbol/strategy combination even when multiple intervals feed it, so that state is shared correctly.
15. As a live trading user, I want strategy state to persist consistently across ticks regardless of which supporting interval updated last, so that trailing logic and counters remain correct.
16. As a system maintainer, I want the engine to separate market data orchestration from trading decisions, so that timeframe alignment can be tested in isolation.
17. As a system maintainer, I want a deep module responsible for timeframe synchronization, so that multi-timeframe correctness does not leak into every indicator binding.
18. As a system maintainer, I want a deep module responsible for strategy-visible series lookup, so that future additions like multi-symbol context have a stable place to plug in.
19. As a system maintainer, I want live and backtest runners to reuse the same scheduling semantics, so that bug fixes land once.
20. As a system maintainer, I want explicit configuration for required intervals, so that strategy requirements are visible before runtime.
21. As a system maintainer, I want observability around which timeframe triggered each decision, so that debugging mixed-interval strategies is practical.
22. As a system maintainer, I want cache invalidation rules to be timeframe-aware, so that incremental indicator updates stay correct.
23. As a strategy reviewer, I want multi-timeframe strategies to fail fast on invalid timeframe references, so that mistakes surface at load time instead of mid-run.
24. As a future UI user, I want the eventual GPUI backtest flow to inherit the same multi-timeframe engine contract, so that CLI and UI results stay aligned.

## Implementation Decisions

- The engine will move from a single candle history to a timeframe-indexed market data model. One strategy instance will own multiple closed-bar series keyed by timeframe string.
- Multi-timeframe execution will be centered on a primary timeframe. `on_tick` will execute only when the primary timeframe closes. Supporting timeframes will update internal series state but will not independently trigger trade decisions.
- Strategy-visible data will be exposed through a small timeframe-aware lookup surface instead of replacing the existing candle contract wholesale. The current primary `candles` argument will remain the primary series, and additional timeframe access will be added via context or a companion lookup abstraction.
- Candle visibility semantics will be closed-bar only for every timeframe. No partially formed higher-timeframe candle will be observable from Rhai. This is a hard anti-look-ahead rule and should be preserved across backtest and live execution.
- Indicator caches will be partitioned per timeframe. Cache keys that currently depend only on indicator parameters will be extended to include the timeframe identity so that incremental EMA/RSI/ATR state stays isolated.
- Anchored runtime outputs will remain strategy-local, but any evaluator or detector that consumes candles must be bound to a specific timeframe. If the first iteration keeps anchored indicators primary-timeframe-only, that limitation must be explicit and enforced.
- Warmup detection and warmup loading will become multi-timeframe-aware. The system will compute the required historical preload for each referenced timeframe and block execution until every required series has enough history or fail clearly when data is missing.
- A dedicated timeframe synchronization module will be introduced as a deep module. Its responsibility will be: ingest per-timeframe candle streams, maintain latest closed bars per timeframe, merge them into a deterministic event timeline, and expose the exact strategy-visible snapshot for each primary tick.
- A dedicated strategy data access module will be introduced as a deep module. Its responsibility will be: present primary and supporting series to Rhai in a stable API, hide storage details, and centralize boundary behaviors like empty series, indexing, and newest-first semantics.
- The backtester will stop assuming a single `Vec<Candle>` input and instead consume a multi-series input set plus synchronization rules. It will produce decisions only on primary timeframe ticks while still updating supporting series between those ticks.
- The live daemon will stop running one independent engine per interval for the same strategy intent. Instead, one strategy instance will subscribe to all required intervals for one symbol and route those candles through the same multi-timeframe coordinator used conceptually by the backtester.
- Configuration will declare the primary interval and supporting intervals explicitly rather than inferring intent from a flat list alone. This keeps the operational model readable and prevents ambiguous execution cadence.
- Strategy loader validation will be extended so timeframe references are validated at load time where possible. Invalid references, duplicate declarations, or illegal primary/support combinations should fail before any tick is processed.
- Logging and diagnostics will record which timeframe triggered the current decision and which supporting timeframe snapshots were visible, so that mixed-interval debugging is practical.
- The design will preserve the existing newest-first, 1-indexed candle semantics inside Rhai for each individual series. Multi-timeframe support should feel like “the same candle API, but namespaced by timeframe,” not like a completely different scripting model.

## Testing Decisions

- Good tests will assert external behavior only: visible candle snapshots, trigger timing, decision timing, trade outcomes, and reproducibility. They will not assert internal storage shapes, lock usage, or cache implementation details.
- The timeframe synchronization module will receive focused unit tests covering deterministic ordering, primary-tick triggering, late-arriving supporting candles, duplicate timestamps, and closed-bar visibility guarantees.
- The strategy data access module will receive focused unit tests covering timeframe lookup, newest-first indexing, empty series behavior, and parity of the primary-series API with the existing single-timeframe contract.
- The indicator cache layer will receive tests proving timeframe isolation, especially for incremental indicators that currently store mutable running state.
- Engine-level tests will verify that a Rhai strategy can mix primary and supporting intervals without look-ahead, and that state persists correctly across interleaved supporting updates.
- Backtester tests will verify that the same multi-timeframe input produces deterministic trades and metrics across repeated runs.
- Live-daemon integration tests will verify that one strategy instance consuming multiple interval streams produces the same decisions as the backtester for the same candle sequence.
- Warmup tests will verify that missing history on any required timeframe yields a clear failure or a documented degraded mode, whichever behavior is chosen.
- Prior art should follow the existing style already present in the codebase: engine behavior tests around Rhai execution and state persistence, backtester tests around trade/equity outcomes, warmup tests around historical preload, and integration tests around SpacetimeDB-backed runtime flows.

## Out of Scope

- Multi-symbol strategies that read candles from other assets.
- UI work for GPUI beyond preserving compatibility with the new engine contract.
- Portfolio-level coordination across multiple strategy instances.
- Optimizer, walk-forward testing, or parameter search.
- Persistent storage of backtest sessions beyond today’s existing backtester scope.
- A generalized partially formed candle model for intra-bar decisioning.
- Automatic timeframe resampling from a single base stream unless the project explicitly decides to support that later.
- Strategy language redesign beyond the minimum API needed for timeframe-aware candle access.

## Further Notes

- The highest-risk failure mode is accidental look-ahead from exposing an unfinished higher-timeframe candle. The implementation should treat this as a correctness bug, not a minor edge case.
- The cleanest rollout is likely incremental: first primary + higher-timeframe read support in the engine and backtester, then live-daemon orchestration, then any anchored-indicator expansion beyond the primary timeframe.
- The most valuable deep modules here are the timeframe synchronization module and the strategy data access module. If those boundaries stay clean, the rest of the refactor should remain tractable.
- The existing architecture already distinguishes engine, backtester, daemon, and SpacetimeDB-backed candle access. Multi-timeframe support should strengthen that separation rather than spreading timeframe logic across every caller.
