# Writing typed Runtime strategies

This folder contains `.rhai` examples for the Trading Runtime typed Strategy
Handling API. The current target API is the runtime-owned Rhai path described in
[ADR 0005](../docs/adr/0005-use-typed-rhai-strategy-api.md), not the legacy
`engine` API.

Use this README as the practical guide for creating a new strategy. Use
[`REFERENCE.md`](./REFERENCE.md) for the exact public Rhai API. Maintainers who
need runner/runtime event details should also read
[`docs/refactor/runtime-market-state-semantics.md`](../docs/refactor/runtime-market-state-semantics.md).

## Quick start

1. Copy one of the maintained examples in this folder.
2. Rename it to match your intent.
3. Declare the strategy-owned Primary Timeframe in `strategy_config()`.
4. Keep the required tick hook as `fn on_tick(market, context)`.
5. Return typed decisions from `decision::*`.
6. Use canonical `ta::*` for technical-analysis helpers.
7. Use read-only Portfolio/Position helpers and typed primitive Strategy State helpers for clarity.
8. Declare optional warmup/Secondary requirements in `strategy_config()`.
9. Declare optional anchored/structure compute in `anchored_config()`.

The live daemon and backtester migration to the new runtime API is tracked
separately. These examples are validated against `trading-runtime`'s typed Rhai
loader and runtime tick path.

## Required hooks

Every strategy must define `strategy_config()` and `on_tick(...)`:

```rhai
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1d"))
}

fn on_tick(market, context) {
    decision::hold()
}
```

`market` is the Market View: it contains Primary and configured Secondary market
data plus market-derived anchored outputs. `context` is the Strategy Context: it
contains grouped runtime session information such as `context.portfolio` and
`context.state`.

## Decisions

Return a typed `StrategyDecision` from the `decision` module:

```rhai
decision::hold()
decision::open_long(1.0)
decision::close_long()
decision::open_short(1.0)
decision::close_short()
```

Opening decisions take quantity in asset units/contracts. They can attach
runtime-managed entry risk parameters with fluent methods:

```rhai
fn on_tick(market, context) {
    let c = market.candle();

    decision::open_long(1.0)
        .with_stop_loss(c.close * 0.95)
        .with_take_profit(c.close * 1.10)
        .with_reason("breakout with hard exits")
}
```

`.with_reason(...)` is allowed on any decision and is diagnostic only.
`.with_stop_loss(...)` and `.with_take_profit(...)` are only valid on opening
decisions.

## Market View

Primary-Timeframe access:

```rhai
let c = market.candle();
let candles = market.candles();
let newest = candles[1];
```

Candle histories are 1-indexed and newest-first for strategy authors:
`market.candles()[1]` is the current Primary candle. Out-of-range indexes return
`()`. Scalar indicator bindings consume histories returned by
`market.candles(...)`. Use canonical `ta::*` for new strategies; the v1 pack
includes `sma`, `ema`, `dema`, `tema`, `slope`, `rsi`, `roc`, `cci`,
`williams_r`, `atr`, `mfi`, scalar `obv`, and the scalar crossover helpers
`ta::cross_over(...)` / `ta::cross_under(...)`:

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
        decision::open_long(1.0)
            .with_stop_loss(market.candle().close * 0.95)
            .with_reason("fast crossed above slow")
    } else {
        decision::hold()
    }
}
```

`indicators::*` remains a transitional alias for the scalar indicator functions
only. It still works for migration, but new strategies and examples should
prefer `ta::*`:

```rhai
indicators::sma(market.candles(), 20) // transitional alias only; prefer ta::sma
```

## Strategy configuration and timeframes

`strategy_config()` is required. Use it to declare exactly one Primary Timeframe,
strategy-declared minimum warmup, and Secondary-Timeframe requirements/defaults.
Run configuration remains authoritative for the Runtime Asset and runner inputs.

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

fn on_tick(market, context) {
    let primary = market.candle();
    let h1 = market.candle(H1);

    decision::hold().with_reason("read primary + H1")
}
```

Use typed `Timeframe` values from `timeframe("1h")` when reading Secondary data:

```rhai
let h1_candle = market.candle(H1);
let h1_history = market.candles(H1);
```

Optional Secondary context that is unavailable or stale returns `()` from
`market.candle(tf)` and `market.candles(tf)`. Required Secondary context that is
unavailable or stale blocks the Strategy Tick before `on_tick` is called.
Runtime-managed Risk Exits are checked on a Tradable Primary candle before
required Secondary blocking, so a hard exit may close an open position without
calling `on_tick`. Accessing an unconfigured timeframe is a Strategy Error.

## Strategy State

Strategy State is runtime-owned, session-local memory for one strategy instance.
It persists between Strategy Ticks in the same session/backtest, starts empty for
a new session, and is not live-persistent across process restarts in v1. V1 state
values are primitives only: int, float, bool, and string.

```rhai
fn on_tick(market, context) {
    let seen = context.state.int("seen", 0);
    context.state.set_int("seen", seen + 1);

    decision::hold().with_reason("seen tick")
}
```

Use matching primitive types per key; reading a key as a different primitive type
is a Strategy Error. The older overloaded `context.state.get(...)` /
`context.state.set(...)` API remains valid, but typed helpers make the intended
primitive type explicit. Strategy State remains primitive-only and session-local
in v1; do not use it for arrays, maps, host objects, Structure Object handles,
or restart persistence.

## Portfolio context

Portfolio data is grouped under `context.portfolio`:

```rhai
if context.portfolio.is_flat() {
    return decision::open_long(1.0);
}

let position = context.portfolio.position;
if position != () && position.is_long() {
    return decision::close_long().with_reason("exit long");
}
```

Useful read-only helpers include `context.portfolio.is_flat()`,
`context.portfolio.has_position()`, `context.portfolio.is_long()`,
`context.portfolio.is_short()`, plus `position.is_long()`,
`position.is_short()`, `position.has_stop_loss()`, and
`position.has_take_profit()` after checking `position != ()`.

`context.portfolio` is the runtime-local Portfolio Snapshot, not an external
broker account snapshot. Portfolio State, Execution Planning, Portfolio
Transitions, and Risk Exits remain runtime-owned; strategy code reads snapshots
and returns `decision::*` values rather than mutating portfolio/execution state.

## Warmup

Warmup is handled by the runtime before Strategy Ticks. The effective warmup is
resolved from auto-detected indicator requirements, `strategy_config()` minimum
warmup, and runtime configuration. V1 applies that resolved count to every
configured timeframe in the Warmup Plan; the runtime does not start Strategy
Ticks until the Primary Timeframe and each configured Secondary Timeframe have
satisfied warmup. During warmup, `on_tick` is not called and Strategy State is
not mutated.

Indicators can still return `()` when there is insufficient visible history or
an invalid period/offset argument, so keep explicit guards in strategy logic.
Full per-indicator documentation remains tracked by #26.

```rhai
let s = ta::sma(market.candles(), 20);
if s == () {
    return decision::hold().with_reason("warming up");
}
```

## Anchored / structure outputs

`anchored_config()` is optional and returns a typed `AnchoredConfig`. Anchored
outputs are market-derived and are read through `market`, not `context`.

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

## Recommended starting points

- `sma_cross.rhai` — rolling SMA crossover with entry risk parameters.
- `min_loss.rhai` — SMA crossover plus primitive Strategy State.
- `trendline_break.rhai` — typed anchored config and market-facing anchored outputs.

## Legacy donor API is not current guidance

The old `fn on_tick(candles, context)` hook and legacy loose return maps such as
`#{ signal: "BUY", size: 0.5 }` belong to the old engine donor material. They are
not supported by the new Trading Runtime Strategy Handling path and should not be
used for new strategies.
