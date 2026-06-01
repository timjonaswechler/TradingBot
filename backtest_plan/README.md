# Backtest Plan examples

These examples are reusable Rhai Backtest Plans. Run them with a separate typed
Runtime strategy file:

```sh
just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/plan.rhai
just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/candle_permutation_monte_carlo.rhai
```

Examples:

- `plan.rhai` — baseline-only AAPL daily window.
- `candle_permutation_monte_carlo.rhai` — baseline plus Synthetic Market Data
  candle-permutation Monte Carlo.

Both examples assume AAPL candles have been seeded from the default
`trading-bot.toml` window (`just seed`). See
[`docs/features/backtest-plans.md`](../docs/features/backtest-plans.md) for the
plan API, Runtime-backed dataset loader contract, Markdown report guide, and the
difference between Synthetic Market Data mutation and future trade-order
resampling.
