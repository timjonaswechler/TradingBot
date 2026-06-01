# Per-Timeframe Strategy Warmup Declarations

Strategy-facing per-timeframe warmup declarations are not planned for now.

## Why this is out of scope

The current Runtime warmup model intentionally resolves one global effective warmup requirement from auto-detected indicator usage, strategy-configured minimum warmup, and runtime minimum warmup, then assigns that same requirement to every configured timeframe. Internally, `WarmupPlan` can remain keyed by timeframe as a future-ready representation, but strategy authors do not currently need a separate per-timeframe warmup API.

Adding per-timeframe declarations without a concrete strategy or dataset need would create unnecessary merge-rule complexity:

- What wins when auto-detection sees an indicator on `market.candles(tf)` but strategy config declares a lower warmup for that timeframe?
- Can Primary and Secondary Timeframes have different requirements without surprising Strategy Tick readiness?
- How should warmup-aware dataset loading derive hidden warmup windows for each timeframe?
- How do live, backtest, and restart behavior stay deterministic?

Until a real strategy shows that the v1 global effective warmup is insufficient, the simpler model is preferred.

## Prior requests

- #87 — Strategy Config: Decide per-timeframe warmup requirements
