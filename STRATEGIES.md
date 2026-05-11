# Writing Strategies

Reference for authoring `.rhai` strategy files in `strategies/`.

## The contract

Every strategy must define:

```rhai
fn on_tick(candles, context) {
    // ... logic ...
    #{ signal: "HOLD" }   // or "BUY" / "SELL" / "SHORT" / "COVER"
}
```

Optionally:

```rhai
fn anchored_config() {
    #{ detectors: [...], evaluators: [...] }
}
```

Called once at load time. Enables the event-driven indicator pipeline (see
[Anchored indicators](#anchored-indicators)).

## Return shape of `on_tick`

Object map with one required field + four optional:

| Field         | Type   | Default       | Notes                                      |
| ------------- | ------ | ------------- | ------------------------------------------ |
| `signal`      | string | —             | `BUY` / `SELL` / `HOLD` / `SHORT` / `COVER` |
| `size`        | float  | 1.0 (0 on HOLD) | Portfolio fraction 0.0–1.0                |
| `stop_loss`   | float  | —             | Hard stop price                            |
| `take_profit` | float  | —             | Take-profit price                          |
| `reason`      | string | —             | Logged alongside the trade                 |

## `candles` API

1-indexed, **newest first**:

| Expression                | Returns                         |
| ------------------------- | ------------------------------- |
| `candles[1]`              | current candle, or `()` if empty |
| `candles[n]`              | nth bar back, or `()`            |
| `candles.len()`           | number of candles                |
| `candles.closes()` etc.   | array of prices, newest first   |

Per-candle getters: `.open`, `.high`, `.low`, `.close`, `.volume`,
`.timestamp`, `.symbol`, `.bar`, plus `.body()` and `.range()`.

`bar` is the absolute bar index (0-based, monotonic for the session). Use it
with `TrendLine.y_at(bar)` and similar.

## `context` API

| Expression                          | Returns                            |
| ----------------------------------- | ---------------------------------- |
| `context.balance`                   | cash                               |
| `context.equity`                    | balance + open position value      |
| `context.trades_count`              | closed trades so far               |
| `context.position`                  | `Position` or `()`                 |
| `context.has_position()`            | bool                               |
| `context.anchored(name)`            | output of an anchored evaluator — see below |
| `context.last_pivot(id, "high"\|"low")` | `PivotEvent` or `()`           |

`Position` has `.side` (`"Long"`/`"Short"`), `.entry_price`, `.size`,
`.entry_time`, `.stop_loss`, `.take_profit`.

## Anchored indicators

Indicators that live on `[start, end]` segments instead of walking with every
bar. Re-evaluated **only when an event fires** (new pivot, session change,
…). Ideal for trendlines, AMD-style phase logic, slope-between-pivots.

### Config shape

```rhai
fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot",   left: 5, right: 5 },
            #{ id: "lon", kind: "session", session: "LONDON",
               start: "0900", end: "1700", tz: 2 },
        ],
        evaluators: [
            #{
                expose_as: "res", kind: "trendline", side: "resistance",
                pivot_source: "p", pivot_buffer: 6,
                tolerance: 0.002, min_touches: 3, max_lines: 1,
            },
            #{
                expose_as: "trend_slope", kind: "slope_between_pivots",
                pivot_source: "p", side: "low",
            },
        ],
    }
}
```

Validation happens at load time. Hard failures:
- duplicate detector `id`
- duplicate evaluator `expose_as`
- evaluator referencing an unknown `pivot_source`
- out-of-range params (e.g. `tolerance` outside `(0, 0.5)`)

### Supported detectors

| `kind`    | fields                                           | emits           |
| --------- | ------------------------------------------------ | --------------- |
| `pivot`   | `id`, `left`, `right`                            | PivotHigh/Low   |
| `session` | `id`, `session`, `start` (`"HHMM"`), `end`, `tz` | SessionOpen/Close |

`session` values: `"ASIA"`, `"LONDON"`, `"NEWYORK"` (or `"NY"`), or a plain `u8` for custom IDs.

### Supported evaluators

| `kind`                    | output type                         | required fields                                                                      |
| ------------------------- | ----------------------------------- | ------------------------------------------------------------------------------------ |
| `trendline`               | `Array<TrendLine>`                  | `side` (`resistance`/`support`), `pivot_source`, `pivot_buffer`, `tolerance`, `min_touches`, `max_lines` |
| `slope_between_pivots`    | `float` or `()`                     | `pivot_source`, `side` (`high`/`low`)                                                |

### Reading outputs

```rhai
let lines = context.anchored("res");          // Array<TrendLine>, possibly empty
if type_of(lines) == "array" && lines.len() > 0 {
    let line = lines[0];
    let y    = line.y_at(candles[1].bar);
    if candles[1].close > y {
        return #{ signal: "BUY", reason: "break" };
    }
}

let slope = context.anchored("trend_slope");  // float or ()
if type_of(slope) == "f64" && slope > 0.0 { /* ... */ }

let p = context.last_pivot("p", "high");      // PivotEvent or ()
```

`TrendLine` fields: `.slope`, `.intercept`, `.touches`, `.anchor_start_bar`,
`.anchor_end_bar`, `.side`, `.y_at(bar)`.

`PivotEvent` fields: `.bar`, `.price`, `.volume`, `.side`.

### Tick semantics

1. New candle arrives.
2. Detectors run; events go into their rings.
3. Evaluators re-fit **only if their source detector fired this tick**. Otherwise the last fit's output is re-exposed.
4. `on_tick` runs — sees the current outputs, including a line that is *just
   being broken* on this bar.
5. After `on_tick`, broken trendlines are pruned; the next tick won't see them.

Consequence: to detect a break, compare `close` against `y_at(bar)` on the
current bar. The line is guaranteed visible exactly on the break bar and
absent thereafter.

## Engine gotchas

### Expression complexity limit

Rhai caps expression depth. Nested map literals with template strings will
trip the compiler with `Expression exceeds maximum complexity`.

```rhai
// ✗ Too deep — template string inside nested map
return #{
    signal: "BUY",
    take_profit: c.close + risk * RR,
    reason: `broke at ${c.close}`,
};

// ✓ Pull computed fields into lets; avoid template strings in returns
let tp = c.close + risk * RR;
return #{
    signal: "BUY",
    take_profit: tp,
    reason: "broke resistance",
};
```

### Warmup

Before enough history exists, most indicators return `()`. Always check and
early-return `HOLD` during warmup:

```rhai
let s = indicators::sma(candles, 20);
if s == () { return #{ signal: "HOLD", reason: "warming up" }; }
```

### Top-level constants

`const`/`let` at file scope run once and are usable by both
`anchored_config` and `on_tick`. Prefer this over magic numbers.

## Examples in this repo

- `sma_cross.rhai` — rolling indicators only, classic SMA crossover.
- `trendline_break.rhai` — anchored pipeline: pivot-based 3+-touch resistance / support, breakout entry, opposite line as stop.
