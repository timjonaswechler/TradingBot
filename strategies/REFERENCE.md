# Typed Strategy Reference

This file documents the current strategy-author Rhai surface for the Trading
Runtime typed Strategy Handling path. It intentionally describes the target
runtime API, not the legacy `engine` API. Maintainer-level event, runner, and
backtester semantics are documented in
[`docs/refactor/runtime-market-state-semantics.md`](../docs/refactor/runtime-market-state-semantics.md).

See [ADR 0005](../docs/adr/0005-use-typed-rhai-strategy-api.md) for the decision
to use typed constructors and fluent methods instead of loose maps and magic
strings.

## Strategy file contract

Required hooks:

```rhai
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    decision::hold()
}
```

Optional load-time hooks:

```rhai
fn anchored_config() {
    anchored_config::new()
}
```

Notes:

- Top-level code runs once when the strategy is loaded.
- `on_tick(market, context)` is called only for Strategy Ticks after warmup and
  required Secondary readiness checks pass.
- `strategy_config()` must declare exactly one Primary Timeframe and may declare
  minimum warmup plus Secondary-Timeframe requirements/defaults.
- `anchored_config()` may declare typed anchored/structure compute.
- A strategy file may include an optional metadata comment such as
  `// name: "sma_cross"`.

## Decisions

`on_tick` must return a typed `StrategyDecision` from `decision::*`.

| Constructor | Meaning |
| --- | --- |
| `decision::hold()` | No strategy-driven position transition. |
| `decision::open_long(quantity)` | Open a long position with quantity in asset units/contracts. |
| `decision::close_long()` | Close an existing long position. |
| `decision::open_short(quantity)` | Open a short position with quantity in asset units/contracts. |
| `decision::close_short()` | Close an existing short position. |

Fluent methods:

| Method | Valid on | Effect |
| --- | --- | --- |
| `.with_stop_loss(price)` | opening decisions only | Adds a runtime-managed hard stop-loss entry risk parameter. |
| `.with_take_profit(price)` | opening decisions only | Adds a runtime-managed hard take-profit entry risk parameter. |
| `.with_reason(text)` | all decisions | Adds diagnostic context; no execution semantics. |

Risk-parameter example:

```rhai
fn on_tick(market, context) {
    let c = market.candle();

    decision::open_long(1.0)
        .with_stop_loss(c.close * 0.95)
        .with_take_profit(c.close * 1.10)
        .with_reason("risk managed long")
}
```

Returning `()`, strings, arrays, or object maps is a Strategy Error.

## Market View

`market` exposes market data and market-derived structure outputs.

Primary-Timeframe access:

| Expression | Returns |
| --- | --- |
| `market.candle()` | current Primary candle |
| `market.candles()` | Primary `CandleHistory` |
| `market.candles()[1]` | current Primary candle |

Secondary-Timeframe access uses typed `Timeframe` values:

```rhai
const H1 = timeframe("1h");

fn on_tick(market, context) {
    let h1 = market.candle(H1);
    let h1_history = market.candles(H1);

    if h1 == () || h1_history == () {
        return decision::hold().with_reason("optional H1 unavailable");
    }

    decision::hold()
}
```

Rules:

- `market.candle()` / `market.candles()` read the Primary Timeframe.
- `market.candle(tf)` / `market.candles(tf)` read a configured Secondary
  Timeframe.
- Candle histories are 1-indexed and newest-first for strategy authors.
- Out-of-range history indexes return `()`.
- Optional unavailable/stale Secondary context returns `()`.
- Required unavailable/stale Secondary context blocks the Strategy Tick before
  `on_tick`.
- Runtime-managed Risk Exits are checked on Tradable Primary candles before
  required Secondary blocking, so a hard exit can close an open position without
  an `on_tick` call.
- Accessing an unconfigured timeframe is a Strategy Error.

### `Candle` fields and methods

| Expression | Type |
| --- | --- |
| `candle.open` | float |
| `candle.high` | float |
| `candle.low` | float |
| `candle.close` | float |
| `candle.volume` | float |
| `candle.timestamp` | integer |
| `candle.symbol` | string |
| `candle.timeframe` | `Timeframe` |
| `candle.body()` | float |
| `candle.range()` | float |

### `CandleHistory`

| Expression | Returns |
| --- | --- |
| `history[n]` | nth candle back, 1-indexed newest-first, or `()` |
| `history.len()` | number of visible candles |

## Indicators

Indicator bindings consume `CandleHistory` values from `market.candles(...)`.
Canonical indicator bindings live under `ta::*`. The typed runtime currently
exposes the scalar v1 pack:

| Function | Returns |
| --- | --- |
| `ta::sma(history, period)` / `ta::sma(history, period, offset)` | `float` or `()` |
| `ta::ema(history, period)` / `ta::ema(history, period, offset)` | `float` or `()` |
| `ta::dema(history, period)` / `ta::dema(history, period, offset)` | `float` or `()` |
| `ta::tema(history, period)` / `ta::tema(history, period, offset)` | `float` or `()` |
| `ta::slope(history, period)` / `ta::slope(history, period, offset)` | `float` or `()` |
| `ta::rsi(history, period)` / `ta::rsi(history, period, offset)` | `float` or `()` |
| `ta::roc(history, period)` / `ta::roc(history, period, offset)` | `float` or `()` |
| `ta::cci(history, period)` / `ta::cci(history, period, offset)` | `float` or `()` |
| `ta::williams_r(history, period)` / `ta::williams_r(history, period, offset)` | `float` or `()` |
| `ta::atr(history, period)` / `ta::atr(history, period, offset)` | `float` or `()` |
| `ta::mfi(history, period)` / `ta::mfi(history, period, offset)` | `float` or `()` |
| `ta::obv(history)` / `ta::obv(history, offset)` | `float` or `()` |
| `ta::cross_over(previous_a, previous_b, current_a, current_b)` | `bool` |
| `ta::cross_under(previous_a, previous_b, current_a, current_b)` | `bool` |

`ta::cross_over(a_prev, b_prev, a, b)` is `a_prev <= b_prev && a > b`.
`ta::cross_under(a_prev, b_prev, a, b)` is `a_prev >= b_prev && a < b`.

`indicators::*` remains a transitional alias for the scalar indicator functions
and offset variants, for example:

```rhai
indicators::sma(market.candles(), 20) // transitional alias only; prefer ta::sma
```

New strategies and maintained examples should prefer `ta::*`. Full indicator
documentation remains tracked by #26; structured-result, session-/period-aware,
OBV-series, first-class `series::*`, and strategic Fibonacci APIs are outside
this scalar Runtime binding pack.

Example:

```rhai
fn on_tick(market, context) {
    let candles = market.candles();
    let fast = ta::sma(candles, 20);
    let slow = ta::sma(candles, 50);
    let fast_prev = ta::sma(candles, 20, 1);
    let slow_prev = ta::sma(candles, 50, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return decision::hold().with_reason("warming up");
    }

    if ta::cross_over(fast_prev, slow_prev, fast, slow) && context.portfolio.is_flat() {
        decision::open_long(1.0).with_reason("fast crossed above slow")
    } else {
        decision::hold()
    }
}
```

Most history-dependent indicators return `()` until enough visible history is
available or when a period/offset argument is invalid. Keep explicit warmup
guards in strategy code before passing indicator outputs to crossover helpers.

## Strategy Context

`context` is grouped. Market data is not exposed through `context`.

| Expression | Returns |
| --- | --- |
| `context.portfolio` | runtime Portfolio Snapshot |
| `context.state` | session-local Strategy State handle |

### Portfolio Snapshot

| Expression | Returns |
| --- | --- |
| `context.portfolio.realized_cash_balance` | realized cash balance |
| `context.portfolio.equity` | current equity derived from Portfolio State and mark price |
| `context.portfolio.completed_trades` | number of completed trades |
| `context.portfolio.position` | `OpenPosition` or `()` |
| `context.portfolio.is_flat()` | `true` when no position is open |
| `context.portfolio.has_position()` | `true` when a position is open |
| `context.portfolio.is_long()` | `true` when the open position is long |
| `context.portfolio.is_short()` | `true` when the open position is short |

`context.portfolio` is a read-only runtime-local Portfolio Snapshot. It is not
an external broker/account snapshot. Portfolio State, Execution Planning,
Portfolio Transitions, and Risk Exits remain runtime-owned; strategy code reads
snapshots and returns `decision::*` values rather than mutating portfolio or
execution state.

### `OpenPosition`

| Expression | Returns |
| --- | --- |
| `position.side` | `"long"` or `"short"` |
| `position.entry_price` | float |
| `position.quantity` | float |
| `position.entry_time` | integer timestamp |
| `position.stop_loss` | float or `()` |
| `position.take_profit` | float or `()` |
| `position.is_long()` | `true` when this position is long |
| `position.is_short()` | `true` when this position is short |
| `position.has_stop_loss()` | `true` when this position has a stop-loss price |
| `position.has_take_profit()` | `true` when this position has a take-profit price |

Example:

```rhai
fn on_tick(market, context) {
    if context.portfolio.is_flat() {
        return decision::open_long(1.0).with_reason("enter");
    }

    let position = context.portfolio.position;
    if position != () && position.is_long() && market.candle().close < position.entry_price * 0.98 {
        return decision::close_long().with_reason("strategy exit");
    }

    decision::hold()
}
```

## Strategy State

Strategy State is runtime-owned, session-local memory for one running strategy.
It persists between Strategy Ticks in one runtime session and starts empty for a
new session/backtest. V1 does not persist Strategy State across live process
restarts.

Primitive-only typed helper API:

| Expression | Returns / effect |
| --- | --- |
| `context.state.int(name, default_int)` | stored int or default |
| `context.state.float(name, default_float)` | stored float or default |
| `context.state.bool(name, default_bool)` | stored bool or default |
| `context.state.string(name, default_string)` | stored string or default |
| `context.state.set_int(name, value)` | stores an int |
| `context.state.set_float(name, value)` | stores a float |
| `context.state.set_bool(name, value)` | stores a bool |
| `context.state.set_string(name, value)` | stores a string |

The older overloaded `context.state.get(name, default)` and
`context.state.set(name, value)` API remains supported for int, float, bool, and
string primitives. Use one primitive type per key. Reading a key as a different
type is a Strategy Error. Strategy State remains primitive-only and
session-local in v1; arrays, maps, host objects, Structure Object handles, and
restart persistence are not supported.

```rhai
fn on_tick(market, context) {
    let seen = context.state.int("seen", 0);
    context.state.set_int("seen", seen + 1);

    decision::hold().with_reason("seen tick")
}
```

## Strategy Configuration

`strategy_config()` returns a typed `StrategyConfig`.

| Expression | Meaning |
| --- | --- |
| `strategy_config::new()` | Starts a Strategy Configuration. |
| `.with_primary(tf)` | Declares the strategy's required Primary Timeframe. |
| `.with_minimum_warmup(n)` | Declares a global minimum warmup. |
| `.with_secondary(secondary)` | Declares a Secondary-Timeframe requirement/default. |
| `timeframe("1h")` | Parses and validates a typed `Timeframe`. |
| `secondary::required(tf)` | Requires Secondary context before Strategy Ticks. |
| `secondary::optional(tf)` | Allows Strategy Ticks when Secondary context is unavailable. |
| `.with_max_missing_candles(n)` | Sets Secondary freshness tolerance. |

```rhai
const PRIMARY = timeframe("1d");
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(PRIMARY)
        .with_minimum_warmup(200)
        .with_secondary(
            secondary::required(H1)
                .with_max_missing_candles(1)
        )
}
```

Strategy Configuration owns Primary and Secondary Timeframes, but does not
choose the Runtime Asset, live/backtest mode, provider, broker, portfolio state,
or execution semantics. Run Configuration binds the strategy contract to those
runner inputs.

## Warmup

The runtime resolves effective warmup from:

```text
max(auto_detected_warmup, strategy_config_minimum_warmup, runtime_minimum_warmup)
```

Warmup input rebuilds Market State/compute state but does not call `on_tick`,
does not mutate Strategy State, and does not produce Strategy Decisions or
Portfolio Transitions. In multi-timeframe runs, warmup must satisfy each
configured timeframe in the Warmup Plan before Strategy Ticks begin. V1 resolves
one global effective count and assigns it to every configured timeframe; the
plan is keyed by timeframe so future per-timeframe requirements can be added
without changing the strategy hook shape.

## Anchored / structure-aware compute

This section documents the current implemented API. #84 / ADR 0008 accepts a
future Market Structure authoring surface using `structure_config()` and
`market.structure.active(...)`; do not expand `anchored` as the long-term
strategy-authoring term.

`anchored_config()` returns a typed `AnchoredConfig`. Anchored outputs are read
through `market`.

### Config builders

| Expression | Meaning |
| --- | --- |
| `anchored_config::new()` | Empty anchored config. |
| `.with_detector(detector)` | Adds a detector. |
| `.with_evaluator(evaluator)` | Adds an evaluator. |
| `pivot_detector::new("id")` | Creates a pivot detector. |
| `.with_left_bars(n)` / `.with_right_bars(n)` | Configures pivot confirmation windows. |
| `anchored::trendline("name", "pivot_id")` | Creates a trendline evaluator. |
| `.with_side(pivot_side::high())` | Resistance-style high-pivot trendline. |
| `.with_side(pivot_side::low())` | Support-style low-pivot trendline. |
| `.with_pivot_buffer(n)` | Number of pivots retained for fitting. |
| `.with_tolerance(x)` | Touch tolerance. |
| `.with_min_touches(n)` | Minimum touches; must be at least 3. |
| `.with_max_lines(n)` | Maximum output lines. |

```rhai
fn anchored_config() {
    anchored_config::new()
        .with_detector(
            pivot_detector::new("swing")
                .with_left_bars(3)
                .with_right_bars(3)
        )
        .with_evaluator(
            anchored::trendline("resistance", "swing")
                .with_side(pivot_side::high())
                .with_pivot_buffer(6)
                .with_tolerance(0.002)
                .with_min_touches(3)
                .with_max_lines(1)
        )
}
```

### Output access

| Expression | Returns |
| --- | --- |
| `market.anchored(name)` | anchored evaluator output, or `()` |
| `market.last_pivot(detector_id, pivot_side::high())` | last high `PivotEvent`, or `()` |
| `market.last_pivot(detector_id, pivot_side::low())` | last low `PivotEvent`, or `()` |

Currently supported anchored evaluator output:

- `anchored::trendline(...)` returns `Array<TrendLine>` or `()` via
  `market.anchored(name)`.

Example:

```rhai
fn on_tick(market, context) {
    let lines = market.anchored("resistance");
    if type_of(lines) == "array" && lines.len() > 0 {
        let current_bar = market.candles().len() - 1;
        let line = lines[0];
        if market.candle().close > line.y_at(current_bar) {
            return decision::open_long(1.0).with_reason("break above resistance");
        }
    }

    decision::hold()
}
```

### `TrendLine`

| Expression | Returns |
| --- | --- |
| `line.slope` | float |
| `line.intercept` | float |
| `line.touches` | integer |
| `line.anchor_start_bar` | integer |
| `line.anchor_end_bar` | integer |
| `line.side` | `"resistance"` or `"support"` |
| `line.y_at(bar)` | float |

### `PivotEvent`

| Expression | Returns |
| --- | --- |
| `pivot.bar` | integer |
| `pivot.price` | float |
| `pivot.volume` | float |
| `pivot.side` | `"high"` or `"low"` |

## Legacy donor API is not supported here

The old hook `fn on_tick(candles, context)` and loose return maps such as
`#{ signal: "BUY", size: 0.5 }` are legacy old-engine donor material only. They
are not the target Strategy Handling API and are not compatibility-mapped by the
Trading Runtime typed Rhai path.
