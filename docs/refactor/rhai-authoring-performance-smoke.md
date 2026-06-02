# Rhai authoring surface performance smoke measurement

Issue: #105
Date: 2026-06-02

## What is measured

The ignored smoke test in
`trading-runtime/tests/rhai_authoring_performance_smoke.rs` measures the current
runtime-backed Rhai Strategy Tick path for the essentials-first #97 authoring
surface. It is measurement-only: it does not set a pass/fail performance
threshold and does not change strategy/runtime semantics.

The test compares each representative strategy against a minimal Rhai
`decision::hold()` baseline in the same scenario group. The representative
`ta_state_portfolio` profile exercises:

- `market.candles()` and, in multi-timeframe scenarios, `market.candles(tf)`;
- `ta::sma` current/offset calls over visible candle history;
- `ta::cross_over` and `ta::cross_under` scalar helpers;
- `context.state.int` and `context.state.set_int`;
- `context.portfolio.is_flat()`, `.has_position()`, `.is_long()`, and
  `.is_short()`;
- `position.is_long()`, `.is_short()`, `.has_stop_loss()`, and
  `.has_take_profit()` when the scenario starts with an open long position.

Each scenario seeds 500 candles per configured timeframe, runs 200 unmeasured
Primary ticks to warm the interpreter/runtime path, then measures completed
Primary candle handling. The multi-timeframe scenarios configure `1m` Primary
plus required `1h` and `1d` Secondary timeframes with a large missing-candle
tolerance so the seeded Secondary histories remain visible during measurement.

Rerun locally with:

```bash
cargo test -p trading-runtime --release --test rhai_authoring_performance_smoke \
  measure_rhai_authoring_surface_smoke -- --ignored --nocapture
```

The earlier Market View snapshot-only smoke remains in
`trading-runtime/tests/market_view_snapshot_cost.rs` and is documented in
`docs/refactor/rhai-market-view-snapshot-cost.md`.

## Local smoke result

Environment: Darwin 25.5.0 arm64, rustc 1.93.1, `cargo test --release`.

| scenario | group | profile | timeframes | history/timeframe | pre-measure ticks | visible candles on first measured tick | initial position | measured ticks | elapsed | mean/tick | delta/tick vs group baseline | ratio vs baseline |
|---|---|---|---:|---:|---:|---:|---|---:|---:|---:|---:|---:|
| primary_hold_baseline_500 | primary_flat_500 | hold_baseline | 1 | 500 | 200 | 701 | flat | 5,000 | 381.990ms | 76.397µs | 0ns | 1.00x |
| primary_ta_state_portfolio_500 | primary_flat_500 | ta_state_portfolio | 1 | 500 | 200 | 701 | flat | 5,000 | 434.054ms | 86.810µs | +10.413µs | 1.14x |
| multi_hold_baseline_500_each_long_position | multi_long_position_500_each | hold_baseline | 3 | 500 | 200 | 1,701 | long | 2,000 | 118.090ms | 59.045µs | 0ns | 1.00x |
| multi_ta_state_portfolio_position_500_each | multi_long_position_500_each | ta_state_portfolio | 3 | 500 | 200 | 1,701 | long | 2,000 | 149.878ms | 74.939µs | +15.894µs | 1.27x |

## Notes for future comparisons

These numbers are smoke measurements, not rigorous benchmarks. Use them to
compare order-of-magnitude before/after values when evaluating future
optimization work. If this path becomes material in larger backtests, follow-up
candidates should be scoped separately and validated with profiling before any
behavioral or data-structure changes.

Follow-up candidates only:

1. Add a Criterion or profiler-backed benchmark for the same public runtime path
   if release smoke output becomes too noisy for an optimization decision.
2. Revisit Market View snapshot caching, bounded history windows, or borrowed
   views only under a new accepted optimization issue/design.
3. Evaluate first-class `series::*` or incremental indicator/compute state only
   as future API/architecture work, not as part of #105.

No caching, bounded histories, borrowed Market View optimization, first-class
`series::*`, Portfolio/Execution semantic changes, or DB/runtime boundary
changes were introduced by #105.
