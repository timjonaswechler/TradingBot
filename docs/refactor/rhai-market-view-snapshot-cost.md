# Rhai Market View snapshot cost smoke measurement

Issue: #83
Date: 2026-06-01

## What was measured

The runtime currently builds an owned Rhai `MarketView` snapshot before each Rhai
`on_tick(market, context)` call. The smoke harness in
`trading-runtime/tests/market_view_snapshot_cost.rs` measures that current
Strategy Tick path with a minimal Rhai strategy that reads
`market.candles().len()` and, for multi-timeframe scenarios,
`market.candles(tf).len()` before returning `decision::hold()`. Because the
snapshot is eager, the timing delta across history sizes/timeframe counts is the
current candle-history snapshot cost plus the constant Rhai/runtime tick
baseline.

The harness covers:

- typical history: 500 candles per configured timeframe
- long history: 10,000 candles per configured timeframe
- Primary-only: `1m`
- multi-timeframe: `1m` Primary plus required `1h` and `1d` Secondary
  timeframes

Rerun with:

```bash
cargo test -p trading-runtime --release --test market_view_snapshot_cost \
  measure_rhai_market_view_snapshot_cost -- --ignored --nocapture
```

## Local smoke result

Environment: Apple M1 Pro, Darwin arm64, `cargo test --release`.

| scenario | timeframes | history/timeframe | visible candles on first tick | measured ticks | elapsed | mean/tick |
|---|---:|---:|---:|---:|---:|---:|
| typical_primary_only_500 | 1 | 500 | 501 | 2,000 | 81.269ms | 40.634µs |
| long_primary_only_10k | 1 | 10,000 | 10,001 | 500 | 112.586ms | 225.171µs |
| typical_multi_timeframe_500_each | 3 | 500 | 1,501 | 2,000 | 114.885ms | 57.442µs |
| long_multi_timeframe_10k_each | 3 | 10,000 | 30,001 | 250 | 165.265ms | 661.059µs |

## Decision

No Market View snapshot optimization is required yet for expected live workloads
or ordinary backtest workloads. The measured release-mode cost stays well below a
millisecond per Strategy Tick even with 30,001 visible candles across three
configured timeframes, and typical cases are tens of microseconds per tick.

The long multi-timeframe case is still linear in visible candle count and can
become material for very large historical sweeps. For example, the local
661.059µs/tick smoke result would be noticeable if multiplied across millions of
Strategy Ticks. Treat that as a profiling trigger for a separate optimization
design issue rather than changing semantics in #83.

## If this becomes material later

Any follow-up optimization should preserve the current safety boundary: Runtime
Market State remains runtime-owned, while Rhai observes safe strategy-facing
values. Candidate designs to evaluate in a separate issue:

1. Runtime-owned per-timeframe snapshot cache invalidated when that timeframe's
   Market State history changes, still handing Rhai owned/immutable values.
2. Bounded strategy-declared history windows, if a future typed API explicitly
   defines how much history a strategy can observe.
3. Borrowed or copy-on-write Rhai view only if it can preserve Rhai safety,
   prevent script mutation of Runtime Market State, and avoid lifetime leaks.

Do not implement caching, bounded histories, or borrowed views without a new
accepted issue/design.
