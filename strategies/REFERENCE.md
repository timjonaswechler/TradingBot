# Strategy reference

This file documents the current public Rhai API for strategies in this repo.

It is intentionally limited to what strategy authors can use today through
`on_tick`, `anchored_config`, `candles`, `context`, and `indicators::...`.
Not every Rust helper in `indicators/` is exposed to Rhai.

## Typed Trading Runtime Market View

The new `trading-runtime` Rhai path uses typed hooks and values:

```rhai
const H1 = timeframe("1h");

fn on_tick(market, context) {
    let primary = market.candle();
    let h1 = market.candle(H1);
    let h1_history = market.candles(H1);

    if h1 == () || h1_history == () {
        return decision::hold().with_reason("optional H1 unavailable");
    }

    let h1_sma = indicators::sma(h1_history, 20);
    decision::hold()
}
```

- `market.candle()` / `market.candles()` read the Primary Timeframe.
- `market.candle(tf)` / `market.candles(tf)` accept typed `Timeframe` values from `timeframe("1h")`.
- Candle histories are 1-indexed and newest-first: `[1]` is the newest candle in that timeframe.
- Optional Secondary-Timeframe context that is missing or stale returns `()` from both `market.candle(tf)` and `market.candles(tf)`.
- Required unavailable Secondary-Timeframe context is blocked by runtime readiness before `on_tick` is called.
- Accessing an unconfigured timeframe is a Strategy Error.

## Strategy file contract

Every strategy must define:

```rhai
fn on_tick(candles, context) {
    #{ signal: "HOLD" }   // or BUY / SELL / SHORT / COVER
}
```

Optionally:

```rhai
fn anchored_config() {
    #{ detectors: [...], evaluators: [...] }
}
```

Notes:
- Top-level code runs once at load time.
- `anchored_config()` is called once at load time if present.
- `on_tick(candles, context)` is called on each tick.
- A strategy file may include an optional metadata comment such as:

```rhai
// name: "sma_cross"
```

## `on_tick` return shape

`on_tick` must return a Rhai object map.

| Field | Type | Required | Default | Notes |
| --- | --- | --- | --- | --- |
| `signal` | string | yes | — | `BUY`, `SELL`, `HOLD`, `SHORT`, `COVER` |
| `size` | float | no | `1.0` for directional signals, `0.0` for `HOLD` | Portfolio fraction `0.0..=1.0` |
| `stop_loss` | float | no | — | Hard stop price |
| `take_profit` | float | no | — | Take-profit price |
| `reason` | string | no | — | Logged alongside the trade |

Example:

```rhai
return #{
    signal: "BUY",
    size: 0.5,
    stop_loss: candles[1].low * 0.98,
    take_profit: candles[1].close * 1.10,
    reason: "fast crossed above slow",
};
```

## `candles` API

`candles` is a `CandleList`.

### Indexing and helpers

`candles[n]` is 1-indexed and newest-first.

| Expression | Returns |
| --- | --- |
| `candles[1]` | current candle, or `()` if empty |
| `candles[n]` | nth candle back, or `()` if out of range |
| `candles.len()` | number of visible candles |
| `candles.closes()` | plain Rhai array of closes, newest first |
| `candles.opens()` | plain Rhai array of opens, newest first |
| `candles.highs()` | plain Rhai array of highs, newest first |
| `candles.lows()` | plain Rhai array of lows, newest first |
| `candles.volumes()` | plain Rhai array of volumes, newest first |

Important:
- `candles[1]` is 1-indexed.
- helper arrays like `candles.closes()` are plain Rhai arrays, so they are 0-indexed.
- therefore `candles[1].close == candles.closes()[0]` when data exists.

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
| `candle.bar` | integer, absolute 0-based bar index |
| `candle.body()` | float |
| `candle.range()` | float |

## `context` API

`context` is a `Context` wrapper with portfolio state, anchored outputs, and a
small persistent key-value store.

### Portfolio fields

| Expression | Returns |
| --- | --- |
| `context.balance` | cash balance |
| `context.equity` | balance plus open-position value |
| `context.trades_count` | number of closed trades so far |
| `context.position` | `Position` or `()` |
| `context.has_position()` | bool |

### Persistent state API

State persists between ticks for one strategy instance.

| Expression | Returns / effect |
| --- | --- |
| `context.state(name, default_int)` | stored integer-like value or the provided default |
| `context.state_f(name, default_float)` | stored float-like value or the provided default |
| `context.set_state(name, value)` | stores an integer-like value |
| `context.set_state_f(name, value)` | stores a float-like value |

Use matching read/write pairs per key.

Example:

```rhai
let peak = context.state_f("peak_pnl", 0.0);
if context.position != () {
    let pnl = context.position.pnl();
    if pnl > peak {
        context.set_state_f("peak_pnl", pnl);
    }
}
```

### Anchored output access

| Expression | Returns |
| --- | --- |
| `context.anchored(name)` | anchored evaluator output, or `()` |
| `context.last_pivot(id, "high"|"low")` | `PivotEvent` or `()` |

`context.anchored(name)` returns:
- `Array<TrendLine>` for `trendline`
- `float` or `()` for `slope_between_pivots`

## `Position` API

`context.position` exposes these fields and methods when non-empty:

| Expression | Returns |
| --- | --- |
| `position.side` | `"Long"` or `"Short"` |
| `position.entry_price` | float |
| `position.size` | float |
| `position.entry_time` | integer timestamp |
| `position.stop_loss` | float or `()` |
| `position.take_profit` | float or `()` |
| `position.pnl()` | unrealized PnL at current price |
| `position.value()` | current notional value at current price |

## Anchored indicators

Anchored indicators are event-driven. They do not recompute every bar. They
recompute when their source detector fires.

### `anchored_config()` shape

```rhai
fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot", left: 5, right: 5 },
            #{ id: "lon", kind: "session", session: "LONDON",
               start: "0900", end: "1700", tz: 2 },
        ],
        evaluators: [
            #{
                expose_as: "res",
                kind: "trendline",
                side: "resistance",
                pivot_source: "p",
                pivot_buffer: 6,
                tolerance: 0.002,
                min_touches: 3,
                max_lines: 1,
            },
            #{
                expose_as: "trend_slope",
                kind: "slope_between_pivots",
                pivot_source: "p",
                side: "low",
            },
        ],
    }
}
```

Validation happens at load time.

Load-time errors include:
- duplicate detector `id`
- duplicate evaluator `expose_as`
- unknown `pivot_source`
- invalid parameter ranges

### Supported detectors

| `kind` | Required fields | Emits |
| --- | --- | --- |
| `pivot` | `id`, `left`, `right` | pivot high / low events |
| `session` | `id`, `session`, `start`, `end`, `tz` | session open / close events |

Session values:
- `"ASIA"`
- `"LONDON"`
- `"NEWYORK"`
- `"NY"`
- a numeric string that parses to `u8` for custom IDs

### Supported evaluators

| `kind` | Output | Required fields |
| --- | --- | --- |
| `trendline` | `Array<TrendLine>` | `expose_as`, `side`, `pivot_source`, `pivot_buffer`, `tolerance`, `min_touches`, `max_lines` |
| `slope_between_pivots` | `float` or `()` | `expose_as`, `pivot_source`, `side` |

`trendline.side` must be `"resistance"` or `"support"`.

`slope_between_pivots.side` must be `"high"` or `"low"`.

### Reading anchored outputs

```rhai
let lines = context.anchored("res");
if type_of(lines) == "array" && lines.len() > 0 {
    let line = lines[0];
    let y = line.y_at(candles[1].bar);
    if candles[1].close > y {
        return #{ signal: "BUY", reason: "break above resistance" };
    }
}

let slope = context.anchored("trend_slope");
if type_of(slope) == "f64" && slope > 0.0 {
    // trend filter
}

let p = context.last_pivot("p", "high");
if p != () {
    // use p.bar, p.price, p.volume, p.side
}
```

### `TrendLine` API

| Expression | Returns |
| --- | --- |
| `line.slope` | float |
| `line.intercept` | float |
| `line.touches` | integer |
| `line.anchor_start_bar` | integer |
| `line.anchor_end_bar` | integer |
| `line.side` | `"resistance"` or `"support"` |
| `line.y_at(bar)` | float |

### `PivotEvent` API

| Expression | Returns |
| --- | --- |
| `pivot.bar` | integer |
| `pivot.price` | float |
| `pivot.volume` | float |
| `pivot.side` | `"high"` or `"low"` |

### Tick semantics

1. A new candle arrives.
2. Detectors run.
3. Evaluators recompute only if their source detector fired this tick.
4. `on_tick` sees the current anchored outputs.
5. Broken trendlines are pruned after `on_tick`, so the break bar still sees them and the next tick does not.

That means breakout detection should compare the current candle against the
current line value on the same bar.

## `indicators::...` API

All indicator functions below are currently exposed to Rhai.

Unless noted otherwise, indicators return `()` when there is insufficient data.

### Trend

| Function | Returns |
| --- | --- |
| `indicators::sma(candles, period)` | `float` or `()` |
| `indicators::sma(candles, period, offset)` | `float` or `()` |
| `indicators::ema(candles, period)` | `float` or `()` |
| `indicators::ema(candles, period, offset)` | `float` or `()` |
| `indicators::dema(candles, period)` | `float` or `()` |
| `indicators::dema(candles, period, offset)` | `float` or `()` |
| `indicators::tema(candles, period)` | `float` or `()` |
| `indicators::tema(candles, period, offset)` | `float` or `()` |
| `indicators::macd(candles, fast, slow, signal)` | `#{ line, signal, histogram }` or `()` |
| `indicators::macd(candles, fast, slow, signal, offset)` | `#{ line, signal, histogram }` or `()` |
| `indicators::sar(candles, step, max)` | `#{ value, side, reversed, ep, af }` or `()` |
| `indicators::sar(candles, step, max, offset)` | `#{ value, side, reversed, ep, af }` or `()` |
| `indicators::adx(candles, period)` | `#{ adx, plus_di, minus_di }` or `()` |
| `indicators::adx(candles, period, offset)` | `#{ adx, plus_di, minus_di }` or `()` |
| `indicators::ichimoku(candles)` | `#{ tenkan, kijun, span_a, span_b, chikou }` or `()` |
| `indicators::ichimoku(candles, offset)` | `#{ tenkan, kijun, span_a, span_b, chikou }` or `()` |

### Momentum

| Function | Returns |
| --- | --- |
| `indicators::rsi(candles, period)` | `float` or `()` |
| `indicators::rsi(candles, period, offset)` | `float` or `()` |
| `indicators::cci(candles, period)` | `float` or `()` |
| `indicators::cci(candles, period, offset)` | `float` or `()` |
| `indicators::stochastic_fast(candles, period)` | `#{ k, d }` or `()` |
| `indicators::stochastic_fast(candles, period, offset)` | `#{ k, d }` or `()` |
| `indicators::stochastic_slow(candles, period)` | `#{ k, d }` or `()` |
| `indicators::stochastic_slow(candles, period, offset)` | `#{ k, d }` or `()` |
| `indicators::stochastic_full(candles, period)` | `#{ k, d }` or `()` (k_smooth=3, d_period=3) |
| `indicators::stochastic_full(candles, period, k_smooth)` | `#{ k, d }` or `()` |
| `indicators::stochastic_full(candles, period, k_smooth, d_period)` | `#{ k, d }` or `()` |
| `indicators::williams_r(candles, period)` | `float` or `()` |
| `indicators::williams_r(candles, period, offset)` | `float` or `()` |
| `indicators::roc(candles, period)` | `float` or `()` |
| `indicators::roc(candles, period, offset)` | `float` or `()` |

### Volatility

| Function | Returns |
| --- | --- |
| `indicators::bollinger(candles, period, std_dev)` | `#{ upper, middle, lower }` or `()` |
| `indicators::bollinger(candles, period, std_dev, offset)` | `#{ upper, middle, lower }` or `()` |
| `indicators::atr(candles, period)` | `float` or `()` |
| `indicators::atr(candles, period, offset)` | `float` or `()` |
| `indicators::keltner(candles, period, multiplier)` | `#{ upper, middle, lower }` or `()` |
| `indicators::keltner(candles, period, multiplier, offset)` | `#{ upper, middle, lower }` or `()` |

### Volume

| Function | Returns |
| --- | --- |
| `indicators::obv(candles)` | `float` or `()` |
| `indicators::obv(candles, offset)` | `float` or `()` |
| `indicators::vwap(candles)` | `float` or `()` |
| `indicators::vwap(candles, offset)` | `float` or `()` |
| `indicators::mfi(candles, period)` | `float` or `()` |
| `indicators::mfi(candles, period, offset)` | `float` or `()` |
| `indicators::volume_profile(candles, buckets)` | `Array<#{ price, volume }>` or `()` |
| `indicators::volume_profile(candles, buckets, offset)` | `Array<#{ price, volume }>` or `()` |

### Support / resistance and geometry

| Function | Returns |
| --- | --- |
| `indicators::pivot_points(candles)` | `#{ pp, r1, r2, r3, s1, s2, s3 }` or `()` |
| `indicators::pivot_points(candles, offset)` | `#{ pp, r1, r2, r3, s1, s2, s3 }` or `()` |
| `indicators::fibonacci(candles, low, high)` | `Array<float>` |
| `indicators::slope(candles, period)` | `float` or `()` |
| `indicators::slope(candles, period, offset)` | `float` or `()` |

## Public API boundary

This reference is about the Rhai authoring surface, not every Rust helper in
`indicators/`.

Examples of Rust helpers that exist today but are not exposed directly to Rhai:
- `ema_series`
- `atr_series`
- `ichimoku_custom`

`fibonacci` is the one public indicator without an offset overload because its
result depends only on the explicit `low` and `high` arguments, not on candle
history.

Likewise, the anchored Rust module contains internal event types that are not
currently configurable from strategy code. The public strategy surface today is:
- detectors: `pivot`, `session`
- evaluators: `trendline`, `slope_between_pivots`

## Engine gotchas

### Warmup

Always expect warmup.

```rhai
let s = indicators::sma(candles, 20);
if s == () {
    return #{ signal: "HOLD", reason: "warming up" };
}
```

The backtester tries to infer warmup from `indicators::*` calls and top-level
constants. If it cannot resolve periods, it falls back to `200` bars.

### Expression complexity

Large nested return maps can trip Rhai's expression complexity limit.

```rhai
// Better
let tp = candles[1].close * 1.10;
return #{
    signal: "BUY",
    take_profit: tp,
    reason: "break above resistance",
};
```

### Use top-level constants for strategy parameters

Top-level `const` values run once at load time and are visible to both
`anchored_config()` and `on_tick()`.

```rhai
const FAST = 20;
const SLOW = 50;
```
