# ADR 0009: Use open timestamps for completed candles

- Status: accepted
- Date: 2026-06-03

TradingBot identifies every completed Candle by the open/start timestamp of its interval, not by the close/end timestamp. When runtime, backtest, or dataset-loading logic needs to reason about when a candle became complete, it derives Candle Close Time as `candle.timestamp + candle.timeframe.duration_ms()` instead of redefining the canonical Candle Timestamp. This keeps provider-backed data and stored candles aligned with common market-data APIs while preserving explicit close-boundary reasoning where needed.
