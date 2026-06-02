# Rhai strategy authoring API namespace inventory (#97)

This document is a #97 design inventory. It is not an implementation plan and not
the current shipped strategy reference. Current implemented Rhai APIs are still
listed in `strategies/REFERENCE.md`.

## Scope decisions captured so far

- Rhai remains the v1/default strategy language.
- The Rhai strategy path should be isolated behind an internal Strategy Adapter
  boundary that implements the runtime's `StrategyHandler` interface.
- The Strategy Adapter boundary is an internal module seam, not a public plugin
  framework.
- Native Rust strategies, dynamic Rust plugins, Pine compatibility, Pine parser
  work, and multi-file user libraries are out of #97 v1 scope.
- #97 assumes a single strategy file with user-authored helper functions.
- Strategy State changes are ergonomics-only over the existing primitive,
  session-local v1 state.
- Read-only Portfolio Snapshot convenience helpers are in scope; Portfolio State,
  Execution Planning, Portfolio Transitions, Risk Exits, persistence,
  margin/buying-power, slippage/fees/spread, and dynamic risk-update semantics
  are out of scope.
- Performance work should start with measurements before optimization.
- Implement the essentials first and keep this document as the living inventory
  for later API expansion.

## Reference inspiration

Pinescription's feature inventory is useful as a taxonomy for strategy-authoring
surfaces: language core, series/market data, technical analysis helpers,
math/NA helpers, arrays/matrices, strings, drawing/plotting, request APIs, and
strategy APIs. #97 should borrow that kind of inventory discipline, not Pine
syntax or Pine compatibility behavior.

Reference: <https://github.com/woodstock-tokyo/pinescription/blob/main/docs/features.md>

Initial triage against that reference:

| Concern | #97 stance |
| --- | --- |
| Language core | Use Rhai's native functions, constants, conditionals, loops, and user helper functions. Do not invent Pine syntax. |
| Series/market data | Keep current Market View/CandleHistory for essentials; design typed series later. |
| Technical analysis helpers | Prioritize `ta::*` over broad API expansion. |
| Math/NA helpers | Small pure helper set may be useful after `ta::*`; keep semantics Rhai-native. |
| Arrays/matrices | Defer/reject for #97 v1; Strategy State v2 is not reopened. |
| Strings | Defer unless concrete strategy-author pain appears. |
| Drawing/plotting | Defer; no UI/manual drawing in #97. |
| Request/cross-symbol APIs | Reject for v1; one Runtime Asset first, multi-asset later. |
| Pine `strategy.*` APIs | Reject; TradingBot uses typed `decision::*` and runtime-owned execution semantics. |

## Essentials-first slice

If #97 is later split into implementation issues, the first useful slice should
stay small:

1. Keep `strategy_config()`, `on_tick(market, context)`, and `decision::*` as the
   stable base.
2. Document and test single-file user helper functions as first-class strategy
   authoring style.
3. Introduce `ta::*` as the canonical strategy-facing technical-analysis
   namespace, backed by the pure Rust `indicators` crate.
4. Add the first high-value TA helpers immediately with the `ta::*` namespace:
   `ta::cross_over` and `ta::cross_under`, before designing broad series APIs.
5. Add read-only Portfolio Snapshot and Position convenience helpers such as
   `context.portfolio.is_flat()` / `.has_position()` / `.is_long()` /
   `.is_short()` plus `position.is_long()` / `.is_short()` /
   `.has_stop_loss()` / `.has_take_profit()`.
6. Add Strategy State ergonomics only, e.g. typed primitive getters/setters.
7. Add/extend backtest performance measurements before changing data structures
   or introducing caching.

Defer until after the essentials slice: first-class `series::*`, multi-file
imports/libraries, annotation/plotting concepts, arrays/maps/matrices, and any
plugin/native strategy work.

## Namespace/status overview

| Surface | Status | Purpose |
| --- | --- | --- |
| `strategy_config::*` | Current / keep | Load-time strategy timeframe and warmup contract. |
| `secondary::*` | Current / keep | Secondary-Timeframe readiness declarations. |
| `timeframe(...)` | Current / keep | Explicit typed timeframe parsing boundary. |
| `structure_config::*` | Target from #84 / ADR 0008 | Load-time Market Structure declaration registry. |
| `structure_side::*` | Target from #84 / ADR 0008 | Typed Market Structure side values. |
| `decision::*` | Current / keep | Typed Strategy Decisions returned by `on_tick`. |
| `market.*` | Current / evolve | Market View reads: current candle/history now; series/structure reads later. |
| `market.structure.*` | Target from #84 / ADR 0008 | Runtime-owned active Structure Object snapshots. |
| `context.portfolio` | Current / add helpers | Read-only Runtime Portfolio Snapshot visible to strategies. |
| `context.state` | Current / add ergonomic helpers only | Primitive session-local Strategy State. |
| `ta::*` | Proposed v1 authoring namespace | Strategy-facing technical analysis functions over runtime/pure indicator implementations. |
| `series::*` / typed series values | Design direction | More Pine-like history/series operations without copying Pine syntax. |
| `risk::*` | Proposed pure-helper namespace | Position sizing / risk math helpers with no execution semantics. |
| `annotate::*` / `plot::*` | Deferred design | DB-free explanation/annotation concepts for later reports/UI; no UI drawing in #97. |
| `indicators::*` | Current / transitional alias | Existing strategy-facing name. New docs/examples should prefer canonical `ta::*`; keep `indicators::*` only as migration compatibility until an explicit removal slice. |
| `anchored_config::*`, `anchored::*`, `market.anchored(...)` | Current implemented / migration target | Current typed structure-aware API to be reframed toward Market Structure terminology. |

## Hooks

### Current / keep

```rhai
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    decision::hold()
}
```

### Target from #84 / ADR 0008

```rhai
fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 3, 3);
    s.objects.trendline("resistance", swing)
        .with_side(structure_side::high())
        .with_pivot_buffer(6)
        .with_tolerance(0.002)
        .with_min_touches(3)
        .with_max_active(1);
    s
}
```

### V1 authoring freedom rule

User-authored helper functions in the same strategy file should be documented as
normal and encouraged:

```rhai
fn is_flat(context) {
    context.portfolio.is_flat()
}

fn bullish_cross(market) {
    let candles = market.candles();
    let fast = ta::sma(candles, 20);
    let slow = ta::sma(candles, 50);
    let fast_prev = ta::sma(candles, 20, 1);
    let slow_prev = ta::sma(candles, 50, 1);
    ta::cross_over(fast_prev, slow_prev, fast, slow)
}
```

## `strategy_config::*`

Current/keep:

```rhai
strategy_config::new()
    .with_primary(timeframe("1d"))
    .with_minimum_warmup(200)
    .with_secondary(secondary::required(timeframe("1h")))
```

Non-goal: strategy configuration must not choose Runtime Asset, provider,
live/backtest mode, broker, DB persistence, or Portfolio State.

## `decision::*`

Current/keep:

```rhai
decision::hold()
decision::open_long(quantity)
decision::close_long()
decision::open_short(quantity)
decision::close_short()
```

Current/keep fluent methods:

```rhai
decision::open_long(1.0)
    .with_stop_loss(price)
    .with_take_profit(price)
    .with_reason("text")
```

Non-goals for #97:

- dynamic risk updates,
- order lifecycle modeling,
- slippage/fee/spread execution model,
- portfolio mutation from strategy code.

## `market.*`

Current/keep:

```rhai
market.candle()
market.candles()
market.candle(timeframe("1h"))
market.candles(timeframe("1h"))
```

Near-term ergonomic candidates:

```rhai
market.primary_timeframe()
market.has(timeframe("1h"))
market.current_bar()
```

Open design question: whether series reads should remain history-based or grow a
first-class series surface such as:

```rhai
let close = market.series.close();
let volume = market.series.volume(timeframe("1h"));
```

## `ta::*`

Purpose: canonical strategy-facing technical analysis namespace. Implementation
can reuse pure functions from the Rust `indicators` crate; the `indicators`
crate must stay free of Rhai/runtime dependencies.

Migration rule: new examples and strategy-author documentation should use
`ta::*`. The current `indicators::*` strategy namespace may remain temporarily as
a transitional alias, and warmup detection should recognize both namespaces while
the alias exists. Remove the alias only in an explicit cleanup slice once no
maintained examples/tests rely on it.

Short-term alias/rename candidates from current `indicators::*`:

```rhai
ta::sma(history, period)
ta::ema(history, period)
ta::dema(history, period)
ta::tema(history, period)
ta::slope(history, period)
ta::rsi(history, period)
ta::roc(history, period)
ta::cci(history, period)
ta::williams_r(history, period)
ta::atr(history, period)
ta::mfi(history, period)
ta::obv(history)
```

Offset variants should remain available where current APIs support them:

```rhai
ta::sma(history, period, offset)
```

High-value authoring helpers for the first `ta::*` slice:

```rhai
ta::cross_over(previous_a, previous_b, current_a, current_b)
ta::cross_under(previous_a, previous_b, current_a, current_b)
```

Semantics:

```rhai
ta::cross_over(fast_prev, slow_prev, fast, slow)  // fast_prev <= slow_prev && fast > slow
ta::cross_under(fast_prev, slow_prev, fast, slow) // fast_prev >= slow_prev && fast < slow
```

Additional pure helpers to consider later:

```rhai
ta::change(value, previous)
ta::percent_change(value, previous)
```

Longer-term, if first-class series values exist:

```rhai
ta::sma(close_series, 20)
ta::cross_over(fast_series, slow_series)
ta::highest(high_series, 20)
ta::lowest(low_series, 20)
ta::value_when(condition_series, value_series, occurrence)
```

## `series::*` / typed series values

Design direction only for now. This may be needed to make Rhai feel closer to a
real trading scripting surface while staying Rhai-native.

Possible shape:

```rhai
let close = market.series.close();
close[0]      // current value
close[1]      // previous value
close.len()
series::shift(close, 1)
series::change(close)
series::highest(close, 20)
series::lowest(close, 20)
```

Open concerns before implementation:

- indexing convention: Pine-like `0=current` vs current CandleHistory
  `1=current`,
- avoiding confusion with current `market.candles()[1]`,
- snapshot/copy costs in large backtests,
- whether series are scalar views over CandleHistory or cached Compute State.

## `context.portfolio`

Current data stays read-only:

```rhai
context.portfolio.realized_cash_balance
context.portfolio.equity
context.portfolio.completed_trades
context.portfolio.position
```

First-slice ergonomic helpers:

```rhai
context.portfolio.is_flat()
context.portfolio.has_position()
context.portfolio.is_long()
context.portfolio.is_short()
```

First-slice Position helpers:

```rhai
let p = context.portfolio.position;
p.is_long()
p.is_short()
p.has_stop_loss()
p.has_take_profit()
```

These helpers must be read-only convenience over the Runtime Portfolio Snapshot.
They must not introduce new Portfolio State or Execution semantics.

## `context.state`

Current semantics remain primitive-only, session-local, non-restart-persistent.

Current:

```rhai
context.state.get("seen", 0)
context.state.set("seen", seen + 1)
```

Ergonomic candidates only:

```rhai
context.state.int("seen", 0)
context.state.float("threshold", 0.0)
context.state.bool("enabled", false)
context.state.string("phase", "new")

context.state.set_int("seen", seen + 1)
context.state.set_float("threshold", 1.5)
context.state.set_bool("enabled", true)
context.state.set_string("phase", "active")
```

Rejected for #97:

- arrays,
- maps,
- host objects,
- storing Structure Object handles,
- restart persistence,
- Strategy State v2 semantics.

## `risk::*`

Purpose: pure strategy-author helpers for sizing/risk math. These functions may
use numbers supplied by the strategy, but they must not mutate Portfolio State or
plan execution.

Candidates:

```rhai
risk::quantity_for_cash(cash, entry_price)
risk::quantity_for_fraction(equity, fraction, entry_price)
risk::quantity_for_fixed_risk(equity, risk_fraction, entry_price, stop_price)
risk::stop_percent(entry_price, percent, side)
risk::take_profit_rr(entry_price, stop_price, reward_risk, side)
```

Open concern: naming must not imply broker buying power, margin, or reservation
semantics that the Runtime Portfolio Snapshot does not own.

## `market.structure.*`

Target from #84 / ADR 0008:

```rhai
let lines = market.structure.active("resistance");
```

Rules already decided by #84:

- object ID must be declared by `structure_config()`,
- unknown IDs are errors,
- declared but inactive objects return empty snapshot collections,
- runtime owns object truth/lifecycle/annotations,
- strategies may read/filter active snapshots but do not own persistent
  Structure Object truth.

## Deferred annotation/plotting concepts

#97 may inventory plotting/annotation concepts inspired by Pine, but v1 should
not implement UI/manual drawing behavior. A future namespace might emit DB-free
runtime explanation records, not draw directly:

```rhai
annotate::mark("entry_candidate").with_price(market.candle().close)
```

Open concerns:

- whether annotations are Strategy Decisions, Runtime Events, or a separate
  explanation channel,
- event volume in large backtests,
- persistence/UI mapping outside `trading-runtime`.

## Internal Strategy Adapter module seam

Not strategy-author API. Target internal ownership:

```text
trading-runtime StrategyHandler boundary
  └── RhaiStrategyAdapter
        ├── hook loading and validation
        ├── Rhai engine construction
        ├── host namespace registration
        ├── Market View wrappers
        ├── Strategy Context / Portfolio Snapshot wrappers
        ├── Strategy State bridge
        ├── TA/series/risk/structure host functions
        └── Rhai error mapping to StrategyError / load errors
```

The Trading Runtime should keep depending on `StrategyHandler`, not on Rhai
internals.

## Performance measurement list

Before optimizing #97 implementation details, measure at least:

- Rhai `on_tick` call overhead,
- Market View snapshot/copy cost,
- indicator recompute cost,
- `ta::*` wrapper overhead compared with direct pure indicator calls,
- multi-timeframe Market View cost,
- structure compute/update cost,
- large backtest / Monte Carlo parameter sweep cost.
