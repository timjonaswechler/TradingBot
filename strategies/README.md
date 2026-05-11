# Writing strategies

This folder contains `.rhai` trading strategies.

Use this README as the practical guide for creating a new strategy. Use
[`REFERENCE.md`](./REFERENCE.md) for the exact public Rhai API.

## Quick start

1. Copy a starter strategy into this folder.
2. Rename it to match your intent.
3. Edit the constants and `on_tick` logic.
4. Backtest it.
5. Only then wire it into the live daemon config.

### Backtest loop

```bash
just backtest --strategy strategies/my_strategy.rhai --symbol AAPL --interval 1d
```

If you do not have candles yet:

```bash
just seed
```

To run the live daemon with a strategy path from `trading-bot.toml`:

```bash
just run
```

## Folder conventions

- Put new strategy files in `strategies/`.
- Use a `.rhai` filename.
- Add an optional header comment near the top:

```rhai
// name: "my_strategy"
```

- Keep the header name, filename, and strategy intent aligned.
- Prefer top-level `const` values for tunable parameters.
- Start from one of the two reference examples in this folder:
  - `sma_cross.rhai` for rolling indicators
  - `trendline_break.rhai` for anchored indicators

## Execution model

- Top-level code runs once when the strategy is loaded.
- `fn on_tick(candles, context)` runs once per visible candle.
- `fn anchored_config()` is optional and is called once at load time.
- Backtest warmup is auto-detected from `indicators::*` calls when possible.
- Even with auto warmup, indicators can still return `()` during startup, so keep explicit warmup guards in strategy code.

## Starter template: rolling indicators

```rhai
// name: "my_strategy"

const FAST = 20;
const SLOW = 50;

fn on_tick(candles, context) {
    let fast      = indicators::ema(candles, FAST);
    let slow      = indicators::sma(candles, SLOW);
    let fast_prev = indicators::ema(candles, FAST, 1);
    let slow_prev = indicators::sma(candles, SLOW, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return #{ signal: "HOLD", reason: "warming up" };
    }

    let crossed_up   = fast_prev <= slow_prev && fast > slow;
    let crossed_down = fast_prev >= slow_prev && fast < slow;

    if crossed_up && !context.has_position() {
        return #{
            signal: "BUY",
            size: 0.5,
            reason: "fast crossed above slow",
        };
    }

    if crossed_down && context.has_position() {
        return #{
            signal: "SELL",
            reason: "fast crossed below slow",
        };
    }

    #{ signal: "HOLD" }
}
```

## Starter template: anchored indicators

```rhai
// name: "my_anchored_strategy"

const PIVOT_LEFT   = 5;
const PIVOT_RIGHT  = 5;
const PIVOT_BUFFER = 6;
const TOLERANCE    = 0.002;
const MIN_TOUCHES  = 3;

fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot", left: PIVOT_LEFT, right: PIVOT_RIGHT },
        ],
        evaluators: [
            #{
                expose_as:    "resistance",
                kind:         "trendline",
                side:         "resistance",
                pivot_source: "p",
                pivot_buffer: PIVOT_BUFFER,
                tolerance:    TOLERANCE,
                min_touches:  MIN_TOUCHES,
                max_lines:    1,
            },
        ],
    }
}

fn on_tick(candles, context) {
    let c = candles[1];
    if c == () { return #{ signal: "HOLD" }; }

    let lines = context.anchored("resistance");
    let has_lines = type_of(lines) == "array" && lines.len() > 0;
    if !has_lines {
        return #{ signal: "HOLD", reason: "waiting for anchored fit" };
    }

    let line = lines[0];
    if c.close > line.y_at(c.bar) && !context.has_position() {
        return #{
            signal: "BUY",
            size: 0.5,
            reason: "break above resistance",
        };
    }

    #{ signal: "HOLD" }
}
```

## Authoring conventions for this repo

### 1. Always guard warmup explicitly

Most indicator functions return `()` until enough history exists.

```rhai
let rsi = indicators::rsi(candles, 14);
if rsi == () {
    return #{ signal: "HOLD", reason: "warming up" };
}
```

### 2. Prefer constants over magic numbers

```rhai
const RSI_PERIOD = 14;
const OVERSOLD   = 30.0;
```

### 3. Return `reason` for meaningful actions

Good `reason` strings make backtests and live runs much easier to inspect.

### 4. Be explicit about position checks

Use `context.has_position()` or `context.position == ()` before opening or closing.

### 5. Use matching state types per key

If you persist strategy state between ticks, keep each key either integer-based
or float-based.

```rhai
let seen = context.state("seen", 0);
context.set_state("seen", seen + 1);
```

```rhai
let peak = context.state_f("peak", 0.0);
if candles[1] != () && candles[1].close > peak {
    context.set_state_f("peak", candles[1].close);
}
```

### 6. Treat helper arrays as plain arrays

`candles[1]` is 1-indexed and newest-first.

`candles.closes()`, `opens()`, `highs()`, `lows()`, and `volumes()` return
plain Rhai arrays, so they are 0-indexed. That means:

```rhai
candles[1].close == candles.closes()[0]
```

when data exists.

## Recommended starting points

### `sma_cross.rhai`

Start here if you want:
- rolling indicators only
- simple long-only entry/exit logic
- a small example with warmup guards and offset lookbacks

### `trendline_break.rhai`

Start here if you want:
- anchored indicators
- pivot-driven trendlines
- event-driven breakout logic with stops and targets

Other files in this folder may be experimental; the two strategies above are
the maintained starter references.

## When to open `REFERENCE.md`

Open [`REFERENCE.md`](./REFERENCE.md) when you need:
- exact `on_tick` return fields
- the full `candles` and `context` API
- the full list of currently exposed `indicators::...` functions
- anchored detector/evaluator config details
- edge cases and engine gotchas
