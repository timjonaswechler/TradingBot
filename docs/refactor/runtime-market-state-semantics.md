# Runtime Market State and runner semantics

This document is maintainer guidance for the current multi-timeframe Trading
Runtime integration. It describes implemented behavior, not future target API.
Use `CONTEXT.md` for glossary definitions and
`docs/refactor/runtime-migration-control.md` for canonical/donor/transitional
path classification.

## Scope and boundaries

- `trading-runtime` owns Market Input validation, Market State, Market View,
  Warmup, Secondary readiness checks, Risk Exits, Strategy Ticks, Portfolio
  Transitions, and ordered DB-free Runtime Events.
- `trading-daemon` is the live runner/IO layer. It fetches/subscribes to
  candles, feeds runtime input, applies live-runner shutdown policy, and requests
  runtime commands such as `force_close(...)`.
- `backtester` is a historical runner/reporting layer. It loads historical
  candles, replays them into one Trading Runtime, collects `RuntimeStep`s, and
  derives reports from runtime events and snapshots.

Do not add duplicate Portfolio/Execution semantics to runners. If live and
backtest behavior should match, the behavior belongs in `trading-runtime`.

## Runtime configuration and Market Input

A `RuntimeConfig` describes one Runtime Asset, one strategy-declared Primary
Timeframe, and zero or more strategy-declared Secondary Timeframes. The Primary
Timeframe is the only timeframe whose completed candles can become Tradable
Candles and Strategy Ticks. Secondary Timeframes are context only.

The runtime accepts two market-input forms:

| Input | Meaning |
| --- | --- |
| `MarketInput::WarmupCandle(candle)` | Historical Warmup Input for any configured timeframe. It rebuilds Market State/compute state and advances warmup progress. |
| `MarketInput::CompletedCandle(candle)` | A completed live or replay candle after the runner's warmup preload phase. Primary input may enter the tradable path after warmup. Secondary input updates Market State only. |

Unknown or unconfigured timeframes are runtime-boundary errors
(`RuntimeInputError::UnknownTimeframe`) and are not stored as Market State.
Accepted input is stored even when it does not produce a Strategy Tick.

A completed Primary candle received before runtime warmup is complete is accepted
into Market State but is not treated as Warmup Input and does not produce a
Tradable Candle, Risk Exit, Strategy Tick, Strategy Decision, or Portfolio
Transition.

## Warmup Plan

Warmup is represented as a `WarmupPlan` keyed by configured timeframe. The
runtime considers warmup complete only after every configured timeframe has
received its required number of `WarmupCandle` inputs.

Current v1 resolution assigns the same global effective warmup count to every
configured timeframe:

```text
max(auto_detected_indicator_warmup, strategy_config_minimum_warmup, runtime_minimum_warmup)
```

The plan is still keyed by timeframe so future per-timeframe requirements can be
introduced without changing the runtime input model. Strategy configuration can
raise the minimum warmup but cannot lower detected indicator requirements or the
runner/runtime minimum.

During Warmup Input:

1. Market State and runtime-owned compute state are updated.
2. `WarmupInputAccepted` and `WarmupAdvanced` events are emitted.
3. `WarmupCompleted` is emitted once all configured timeframe requirements are
   met.
4. `on_tick` is not called, Strategy State is not mutated, Risk Exits are not
   evaluated, and Portfolio Transitions are not produced.

## Completed candles after warmup

### Secondary-Timeframe completed candles

A completed Secondary candle updates Market State and runtime-owned compute
state, then emits `MarketInputAccepted`. It does not emit
`TradableCandleAccepted`, does not evaluate Risk Exits, and never calls
`on_tick`.

### Primary-Timeframe completed candles

After warmup, a completed Primary candle is the active tradable boundary. The
runtime records it, emits `MarketInputAccepted`, then emits
`TradableCandleAccepted` before deciding whether the candle creates a Risk Exit,
a blocked Strategy Tick, or a Strategy Tick.

The ordering is intentional:

1. Store the Primary candle in Market State.
2. Check runtime-managed Risk Exits for an existing open position.
3. If no Risk Exit occurs, evaluate required/optional Secondary readiness.
4. If required Secondary context is unavailable, emit `StrategyTickBlocked` and
   do not call strategy code.
5. Otherwise emit `StrategyTickStarted`, call `on_tick(market, context)`, plan
   execution, apply any Portfolio Transition, and complete the tick.

Risk Exits therefore happen before required-context Strategy Tick blocking. A
Risk Exit can close a position on a Primary candle without producing a Strategy
Tick or a Protective Runner Shutdown signal.

## Secondary readiness and Market View visibility

Secondary readiness is evaluated from the latest accepted completed candle for a
configured Secondary Timeframe:

- `Missing`: no candle has been accepted for that Secondary Timeframe.
- `Fresh`: latest Secondary close timestamp is within the allowed window.
- `Stale`: the Primary candle timestamp is later than
  `latest_secondary_timestamp + secondary_duration * (max_missing_candles + 1)`.

Freshness uses the Secondary Timeframe duration and close timestamps. The Primary
Timeframe duration is not used for Secondary freshness.

Runtime readiness and Market View visibility share the same internal policy:

- Required Secondary unavailable: `StrategyTickBlocked` is emitted and `on_tick`
  is not called.
- Optional Secondary unavailable: `SecondaryContextUnavailable` is emitted, the
  Strategy Tick may continue, and `market.candle(tf)` / `market.candles(tf)`
  return `()` for that timeframe.
- Available Secondary context: the latest known completed Secondary candle and
  history are visible through Market View.
- Unconfigured timeframe access is a Strategy Error, not missing optional data.

## Runtime event ordering summary

Typical Primary tick with no Risk Exit and all required context available:

```text
MarketInputAccepted
TradableCandleAccepted
StrategyTickStarted
StrategyDecisionProduced
ExecutionActionPlanned
[PositionOpened | PositionClosed]
[PortfolioUpdated]
StrategyTickCompleted
TradableCandleCompleted
```

Required Secondary blocked Primary candle:

```text
MarketInputAccepted
TradableCandleAccepted
StrategyTickBlocked
TradableCandleCompleted
```

Optional Secondary unavailable Primary candle still proceeds and includes a
`SecondaryContextUnavailable` diagnostic before `StrategyTickStarted`.

Risk Exit Primary candle:

```text
MarketInputAccepted
TradableCandleAccepted
RiskExitTriggered
ExecutionActionPlanned
PositionClosed
PortfolioUpdated
TradableCandleCompleted
```

No `StrategyTickStarted` is emitted for the Risk Exit path.

## Live runner behavior

The live daemon runs one runtime task per configured asset. The asset config
binds a strategy file to a Runtime Asset and runner policies; the strategy's
`strategy_config()` supplies the Primary Timeframe and any Secondary Timeframes.

The live runner:

1. Loads the typed Rhai strategy with `RhaiStrategy`.
2. Builds `RuntimeConfig` from the Runtime Asset plus strategy-owned timeframe
   contract.
3. Resolves the Warmup Plan.
4. Preloads Warmup Input per configured timeframe from the database.
5. Subscribes to completed candles for all configured timeframes.
6. Feeds all accepted candles into one `TradingRuntime` instance.

The live runner does not mutate runtime Portfolio State directly. Graceful
user-requested shutdown may request runtime `force_close(...)` when
`liquidate_on_shutdown = true`; this is separate from Protective Runner Shutdown.

## Protective Runner Shutdown

Protective Runner Shutdown is live-runner policy, not Trading Runtime semantics.
It consumes runtime events after a `RuntimeStep` and never re-evaluates Secondary
readiness itself.

The current policy is configured per asset:

```toml
[assets.protective_shutdown]
enabled = true
required_secondary_failure_threshold = 3
```

Defaults are `enabled = true` and
`required_secondary_failure_threshold = 3`. A threshold of `0` is invalid; set
`enabled = false` to disable the policy.

Policy behavior:

- Only `StrategyTickBlocked` events for required Secondary context increment
  counters.
- Optional `SecondaryContextUnavailable` diagnostics do not increment counters.
- Counters are tracked per required Secondary Timeframe.
- A non-blocked Tradable Candle resets the consecutive-block counters. This
  includes Primary candles that took the Risk Exit path.
- When any counter reaches the threshold, the live runner stops the affected
  runtime.

Shutdown behavior:

- If the runtime is flat, the runner stops without calling `force_close(...)`.
- If an open position exists, the runner requests runtime `force_close(...)`
  with reason `"protective runner shutdown"` before stopping.
- The mark candle is the latest observed completed Primary candle, falling back
  to the existing Primary-candle database lookup.
- If no Primary mark candle is available for an open position, the runner returns
  a clear error and does not pretend the position was closed.

Protective Runner Shutdown is distinct from Strategy Exit, Risk Exit, and normal
user-requested shutdown controlled by `liquidate_on_shutdown`.

## Backtester behavior

Runtime-backed backtests load typed Runtime strategies, build `RuntimeConfig`
from the Runtime Asset plus strategy-owned timeframe contract, resolve the Warmup
Plan, and replay historical Primary and Secondary candles into one
`TradingRuntime`.

Historical replay is globally ordered by candle close timestamp across configured
timeframes. At the same timestamp, Secondary input is replayed before Primary
input so a Primary Strategy Tick can see same-boundary Secondary context without
future leakage.

For each configured timeframe, the first `WarmupPlan` requirement count is sent
as Warmup Input; remaining candles are sent as completed input. Completed
Secondary input updates Market State only. Completed Primary input uses the same
Risk Exit, Secondary readiness, Strategy Tick, and Portfolio Transition semantics
as live runtime input.

Backtest reports are derived from `RuntimeStep` events and snapshots. The
runtime-backed backtester does not implement Protective Runner Shutdown; that
policy is live-runner-specific.
