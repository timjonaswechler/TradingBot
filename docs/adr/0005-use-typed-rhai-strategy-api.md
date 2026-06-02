# Use typed Rhai strategy APIs instead of stringly typed maps

- Status: accepted
- Date: 2026-05-29

The new Trading Runtime exposes strategy-facing Rhai APIs as typed custom values built through constructors and fluent methods rather than as loosely parsed object maps and magic strings. This applies to Strategy Decisions, Strategy Configuration, Timeframes, Secondary-Timeframe requirements, and Market Structure / currently implemented anchored configuration.

Examples of the intended shape:

```rhai
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("30m"))
        .with_minimum_warmup(200)
        .with_secondary(
            secondary::required(H1)
                .with_max_missing_candles(1)
        )
}

fn on_tick(market, context) {
    let h1 = market.candle(H1);

    decision::open_long(2.0)
        .with_stop_loss(95.0)
        .with_take_profit(120.0)
        .with_reason("breakout")
}
```

The old engine's legacy return shape, such as `#{ signal: "BUY", size: 0.5 }`, is not supported in the new Strategy Handling path. New stringly typed maps, such as `#{ intent: "OPEN_LONG" }`, are also not the primary API. Raw strings are acceptable at explicit parsing boundaries such as `timeframe("1h")`, where they are immediately validated and converted into typed values.

## Considered options

- Keep the old `signal`/`size` map shape — rejected because it preserves legacy ambiguity and conflicts with ADR 0004.
- Use new maps with fields like `intent`, `quantity`, `timeframe`, or `readiness` — rejected because they still rely on magic strings and shape parsing where typed constructors are clearer.
- Use typed Rhai custom values via constructors and fluent methods — accepted because it gives strategy authors readable APIs while letting the runtime receive typed decisions/configuration.

## Consequences

Strategy examples and docs should use typed constructors and fluent methods such as `decision::*`, `strategy_config::new().with_primary(...)`, `secondary::required(...)`, `.with_stop_loss(...)`, `.with_take_profit(...)`, and `.with_reason(...)`. Legacy `signal`/`size` returns are unsupported and should produce clear strategy errors rather than compatibility mapping. Fluent risk methods are part of the StrategyDecision custom type API; using them where they do not apply, such as on `hold` or close decisions, should produce a clear strategy error rather than silently mutating an irrelevant decision. `.with_reason(...)` is allowed for all decisions and remains diagnostic only.
