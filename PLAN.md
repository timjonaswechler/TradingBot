# Project Milestones Plan

## Context

This project is a professional, stateful Rust trading bot (V2). The architecture has evolved from an ephemeral cron-based setup (V1, described in `DESIGN.md`) to a long-running daemon setup (V2, described in `ARCHITECTURE.md`). The V2 implementation has not yet been started — no Cargo workspace exists yet. This plan breaks the full `NEXT_STEPS.md` roadmap into concrete, trackable milestones.

---

## Milestone 1 — Foundation: Cargo Workspace & Shared Types

**Goal:** A clean, compiling workspace with all crates scaffolded and shared types defined.

### Steps
- [ ] Initialize the Cargo workspace (`Cargo.toml` with all members)
- [ ] Create crate stubs: `shared/`, `db-layer/`, `engine/`, `trading-daemon/`, `trading-ui/`
- [ ] Define core shared types in `shared/`:
  - `Candle` (OHLCV + helper methods: `body()`, `range()`) — `is_bullish()` / `is_bearish()` deferred to later
  - `Signal` enum (`Buy`, `Sell`, `Hold`, `Short`, `Cover`)
  - `TradeDecision` struct (`signal`, `size`, `stop_loss`, `take_profit`, `reason`)
  - `Position` / `PositionSide` structs
  - `Context` struct (`balance`, `equity`, `position`, `trades_count`)
- [ ] Add all workspace-level dependencies to root `Cargo.toml` (`serde`, `thiserror`, `anyhow`, `tokio`, `reqwest`, `mlua`, `rayon`, `clap`, `tracing`) — `chrono` omitted; timestamps handled as Unix ms (`i64`), wall-clock time via `tokio::time`
- [ ] Verify: `cargo build` compiles the whole workspace cleanly

### Verification
```bash
cargo build --workspace
cargo test --workspace
```

---

## Milestone 2 — Data Layer: SpacetimeDB Schema & DB Crate

**Goal:** A running local SpacetimeDB instance with defined tables and a Rust client wrapper.

### Steps
- [ ] Define SpacetimeDB schema tables (in `db-layer/` or a dedicated `spacetimedb-module/`):
  - `candles` (`id`, `canonical_id`, `timestamp`, `symbol`, `open`, `high`, `low`, `close`, `volume`, `timeframe`, `provider`)
  - `live_positions` / `positions` (`id`, `strategy`, `symbol`, `side`, `entry_price`, `size`, `stop_loss`, `take_profit`, `entry_time`)
  - `live_trades` / `trades` (`id`, `timestamp`, `strategy`, `symbol`, `side`, `entry_price`, `exit_price`, `size`, `pnl`, `status`, `entry_reason`, `exit_reason`)
- [ ] Deploy schema to a local SpacetimeDB server
- [ ] Implement `db-layer` crate:
  - `client.rs` — SpacetimeDB SDK connection setup
  - `queries.rs` — helpers: `get_candles(symbol, timeframe, limit)`, `get_candles_before(symbol, timeframe, before_ts, limit)`, `insert_candle()`, `get_open_position()`, `insert_trade()`
- [ ] Write integration tests against local SpacetimeDB

### Verification
```bash
# SpacetimeDB server running locally
spacetimedb start
cargo test -p db-layer
```

---

## Milestone 3 — Indicators Library

**Goal:** All planned technical indicators implemented as pure Rust functions with `O(1)` incremental state support.

### Steps
- [ ] Set up `indicators/` crate with module structure (`trend/`, `momentum/`, `volatility/`, `volume/`, `support_resistance/`)
- [ ] Implement **Trend** indicators:
  - [ ] SMA, EMA, DEMA, TEMA
  - [ ] MACD (`MacdResult { line, signal, histogram }`)
  - [ ] Parabolic SAR
  - [ ] ADX
  - [ ] Ichimoku Cloud (`IchimokuResult`)
- [ ] Implement **Momentum** indicators:
  - [ ] RSI, CCI, Stochastic (`{k, d}`), Williams %R, ROC
- [ ] Implement **Volatility** indicators:
  - [ ] Bollinger Bands (`BbResult`), ATR, Keltner Channels
- [ ] Implement **Volume** indicators:
  - [ ] OBV, VWAP, Volume Profile, MFI
- [ ] Implement **Support/Resistance**:
  - [ ] Pivot Points, Fibonacci Retracements
- [ ] Implement **Slope** (linear regression slope)
- [ ] All functions return `Option<T>` (return `None` when insufficient data)
- [ ] Unit test every indicator against known values

### Verification
```bash
cargo test -p indicators
```

---

## Milestone 4 — Trading Engine (Lua Scripting + Stateful Indicator Cache)

**Goal:** A working Lua scripting runtime with PineScript-like semantics and O(1) incremental indicator updates for the backtester.

### Steps
- [ ] Set up `engine/` crate (replaces `lua-engine/` from DESIGN.md)
- [ ] `candle_wrapper.rs` — `LuaCandles` UserData: 1-indexed (newest first), `__index`, `__len`, helper methods (`closes()`, `opens()`, `highs()`, `lows()`, `volumes()`)
- [ ] `bindings.rs` — register `indicators.*` Lua table, bridge `LuaCandles → &[f64] / &[Candle]` for each indicator
- [ ] `indicator_cache.rs` — stateful O(1) incremental updates for backtesting (keyed by `(IndicatorType, period, offset)`)
- [ ] `strategy_loader.rs` — load, validate and execute `.lua` strategy files
- [ ] `vm.rs` — Lua VM lifecycle, `on_tick(candles, context)` call, error handling table:
  - `on_tick` exception → log + abort run
  - missing `signal` key → error
  - unknown signal value → error
  - missing `size` on BUY/SELL → default `1.0`
- [ ] `warmup.rs` — startup warmup logic: read required lookback from script, fetch N historical candles from DB, feed into engine before live ticking begins
- [ ] Write unit tests for indicator cache correctness and Lua bindings

### Verification
```bash
cargo test -p engine
# Run example strategy against sample candle data
```

---

## Milestone 5 — Trading Daemon (Live & Paper Trading)

**Goal:** A headless, long-running Tokio service that fetches candles, ticks the engine, and executes paper trades.

### Steps
- [ ] Set up `trading-daemon/` binary crate with Tokio async runtime
- [ ] `data_fetcher.rs` — async task: polls provider API every N minutes, filters incomplete candles (especially Yahoo Finance live candle), writes to SpacetimeDB
- [ ] Implement provider abstraction trait + Yahoo Finance provider (with incomplete candle filtering: `candle.timestamp + interval_ms < now_ms`)
- [ ] `live_engine.rs` — keeps indicator state warm in RAM; on new candle: O(1) update → run Lua script → emit signal
- [ ] `order_executor.rs` — paper trading dummy: intercept BUY/SELL signals, simulate trades, persist to `live_trades` / `live_positions` tables in DB
- [ ] `warmup.rs` — on startup, fetch required historical candles from SpacetimeDB and initialize engine state
- [ ] CLI args via `clap`: `--strategy`, `--symbol`, `--interval`, `--provider`
- [ ] Graceful shutdown (SIGTERM/SIGINT handling)
- [ ] Structured logging via `tracing`

### Verification
```bash
cargo build -p trading-daemon
./target/release/trading-daemon --strategy strategies/sma_cross.lua --symbol AAPL --interval 5m --provider yahoo
# Observe: candles written to DB, paper trades logged
```

---

## Milestone 6 — GPUI Frontend & In-Memory Backtester

**Goal:** A desktop UI for charting, strategy development, and fast in-memory backtesting. The backtest state lives in the UI's RAM for the entire session — switching panels and returning to the backtest panel preserves the full state (trades, PnL curve, current tick position) exactly as left.

### Steps
- [ ] Set up `trading-ui/` binary crate with GPUI
- [ ] Basic window + chart canvas
- [ ] Data fetching: HTTP/SQL query to SpacetimeDB, load historical candles into UI RAM
- [ ] **In-memory backtest runner** (uses `engine` + `indicators` crates directly, no DB writes):
  - [ ] Feed all loaded candles through a fresh engine instance
  - [ ] Backtest state (`Vec<Trade>`, PnL curve, current tick index) lives in UI RAM for the full app session — switching panels and returning keeps everything intact
  - [ ] **Manual single-tick mode:** step through the backtest one candle at a time; each step updates the chart, trade table, and PnL in real time so you can inspect every decision
  - [ ] Auto-run mode: run all candles at once (or up to a chosen date)
  - [ ] Collect simulated trades into `Vec<Trade>` in RAM
  - [ ] Compute metrics: PnL, drawdown, win rate, Sharpe
- [ ] Visualize: PnL curve, drawdown chart, trade entry/exit markers on price chart
- [ ] **Trade & profit table panel:** live-updating table of all simulated trades (entry, exit, size, PnL per trade, cumulative PnL) — updates on each manual tick or at run completion
- [ ] Strategy editor: list `.lua` files in `strategies/`, simple text editor to modify and save
- [ ] Live trades panel: read `live_trades` from SpacetimeDB, display daemon activity
- [ ] **DB Viewer panel:** read-only view of SpacetimeDB state — browse `candles`, `live_trades`, `live_positions` tables with basic filtering (symbol, timeframe, date range); shows row counts and last-updated timestamps as a health overview

### Verification
```bash
cargo build -p trading-ui
# Launch app, load AAPL candles, run backtest, verify chart renders correctly
```

---

## Files to Create (No existing code yet)

| Path | Purpose |
|---|---|
| `Cargo.toml` | Workspace root |
| `shared/` | Shared types (Candle, Signal, Position, Context) |
| `db-layer/` | SpacetimeDB client & query helpers |
| `engine/` | Lua VM, indicator cache, strategy loader |
| `trading-daemon/` | Live/paper trading daemon binary |
| `trading-ui/` | GPUI desktop app binary |
| `strategies/` | Sample Lua strategy files |

---

## Reuse Notes

- `engine/` crate is shared by both `trading-daemon` (live O(1) ticking) and `trading-ui` (in-memory backtest) — no duplication
- `indicators/` pure functions are used by `engine/` only — no direct binary dependency
- `db-layer/` is used by `trading-daemon`; the UI reads DB directly via HTTP/SQL

---

## Milestone Order & Dependencies

```
M1 (Workspace + Shared Types)
  └─► M2 (DB Layer)
  └─► M3 (Indicators)
        └─► M4 (Engine / Lua)
              └─► M5 (Daemon)
              └─► M6 (UI)
```

M2 and M3 can be developed in parallel after M1.
