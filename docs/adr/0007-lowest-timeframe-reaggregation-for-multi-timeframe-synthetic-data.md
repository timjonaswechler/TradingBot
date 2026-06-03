# ADR 0007: Use lowest-timeframe reaggregation for multi-timeframe Synthetic Market Data

- Status: accepted
- Date: 2026-05-31

## Context

Backtest Plan Monte Carlo work adds Synthetic Market Data procedures that mutate copied historical candle datasets before replaying them through the ordinary Trading Runtime path.

Single-timeframe mutations such as candle permutation, ATR-scaled OHLC noise, and log-difference bar permutation are straightforward: they transform one candle series and then replay that series.

Multi-timeframe strategies are harder. A strategy can declare a Primary Timeframe plus Secondary Timeframes, and those candle series should remain coherent enough that Market View context is meaningful. A naive implementation could mutate each timeframe independently, but that can create impossible or misleading combinations where the higher timeframe no longer corresponds to the lower timeframe path.

During triage of #92 and #93, we considered grouped coarsest-timeframe block permutation: define blocks by the largest configured timeframe and move lower-timeframe candles with those blocks. That approach is sometimes useful as a regime/block permutation model, but it created unresolved methodology questions around real market calendars, holidays, session gaps, variable block lengths, timestamp rebasing, and report interpretation. The original #92 issue was therefore closed as not planned.

## Decision

For planned multi-timeframe Synthetic Market Data, the preferred consistency model is **lowest-timeframe-derived reaggregation**:

1. Identify the smallest configured timeframe from the effective RuntimeConfig derived from Strategy Configuration.
2. Mutate that smallest timeframe as the source-of-truth synthetic market path.
3. Regenerate every larger configured timeframe from the mutated smallest timeframe by deterministic OHLCV aggregation.
4. Replay the regenerated multi-timeframe dataset through the ordinary Runtime-backed backtest path.

Aggregation for each larger timeframe uses its existing target Candle Timestamps as the slot calendar. Candle Timestamps are open/start timestamps for completed candle intervals. For a target candle with open timestamp `T` and duration `D`, lower-timeframe candles with open timestamps in `[T, T + D)` are aggregated:

```text
open      = first lower candle open by timestamp
high      = max(lower.high)
low       = min(lower.low)
close     = last lower candle close by timestamp
volume    = sum(lower.volume)
timestamp = T
```

Every target slot must have at least one lower-timeframe candle. The implementation should not invent candles, drop target slots, or fall back to original higher-timeframe candles. It also should not require a fixed expected lower-candle count, because real market sessions, holidays, and data gaps can make that invalid.

The Trading Runtime remains unaware of Synthetic Market Data. These mutations and regenerations belong in `backtester` as research data preparation before Runtime replay.

## Consequences

### Positive

- Higher timeframe candles remain derived from the same synthetic lower timeframe path seen by the strategy.
- Multi-timeframe Synthetic Market Data avoids the incoherence of independent per-timeframe mutation.
- The model is easier to explain than grouped block timestamp rebasing: mutate the finest path, then rebuild coarser context from it.
- It composes naturally with future mutation pipelines: apply stages to the smallest timeframe, then reaggregate.
- The Trading Runtime boundary stays clean; it receives ordinary market input.

### Negative

- Multi-timeframe synthetic runs require adequate smallest-timeframe data for every larger target slot.
- If the smallest configured timeframe is unavailable or sparse, the procedure can fail instead of producing a partial result.
- Some regime/block-resampling use cases are not covered by this decision.
- Reaggregation may not preserve original higher-timeframe volume/session artifacts except through the available lower-timeframe data.

## Rejected alternatives

### Independently mutate each configured timeframe

Rejected as the default because it can create incoherent multi-timeframe market views. A strategy could see a Secondary-Timeframe candle that no longer corresponds to the lower timeframe path that produced the Primary ticks.

Independent per-timeframe mutation may still be allowed only when explicitly scoped and documented as a weaker method.

### Grouped coarsest-timeframe block permutation

Rejected for now and closed in #92 as not planned. The simple timestamp-slot mapping breaks when blocks contain different numbers of lower-timeframe candles, which is common with holidays, market sessions, and data gaps. Timestamp rebasing is technically possible, but introduces enough methodology and interpretation questions that it should require a new accepted decision before implementation.

### Generate higher-timeframe target timestamps from duration math

Rejected because the original higher-timeframe series already carries the market/session calendar. Generating new timestamps from duration math would push calendar/session logic into the backtester and could invent candles outside the loaded dataset.
