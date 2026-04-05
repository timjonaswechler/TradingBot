# Project Milestones Plan

## Context

This project is a professional, stateful Rust trading bot (V2). The architecture has evolved from an ephemeral cron-based setup (V1, described in `DESIGN.md`) to a long-running daemon setup (V2, described in `ARCHITECTURE.md`). This plan breaks the full `NEXT_STEPS.md` roadmap into concrete, trackable milestones. M1–M4 are complete.

---

## Milestone 1 -- Foundation: Cargo Workspace & Shared Types ✅

**Goal:** A clean, compiling workspace with all crates scaffolded and shared types defined.

### Steps
- [x] Initialize the Cargo workspace (`Cargo.toml` with all members)
- [x] Create crate stubs: `shared/`, `db-layer/`, `engine/`, `trading-daemon/`, `trading-ui/`
- [x] Define core shared types in `shared/`:
  - `Candle` (OHLCV + helper methods: `body()`, `range()`) -- `is_bullish()` / `is_bearish()` deferred to later
  - `Signal` enum (`Buy`, `Sell`, `Hold`, `Short`, `Cover`)
  - `TradeDecision` struct (`signal`, `size`, `stop_loss`, `take_profit`, `reason`)
  - `Position` / `PositionSide` structs
  - `Context` struct (`balance`, `equity`, `position`, `trades_count`)
- [x] Add all workspace-level dependencies to root `Cargo.toml` (`serde`, `thiserror`, `anyhow`, `tokio`, `reqwest`, `rhai`, `rayon`, `clap`, `tracing`) -- `chrono` omitted; timestamps handled as Unix ms (`i64`), wall-clock time via `tokio::time`
- [x] Verify: `cargo build` compiles the whole workspace cleanly

> Completed: 9/9 unit tests passing. Git repo initialised. `justfile` added with 18 recipes.

### Verification
```bash
cargo build --workspace
cargo test --workspace
```

---

## Milestone 2 -- Data Layer: SpacetimeDB Schema & DB Crate ✅

**Goal:** A running local SpacetimeDB instance with defined tables and a Rust client wrapper.

### Steps
- [x] Define SpacetimeDB schema tables in `spacetimedb-module/` (separate WASM crate, excluded from main workspace):
  - `candles` (`id` auto_inc, `canonical_id` unique, `timestamp`, `symbol`, `open`, `high`, `low`, `close`, `volume`, `timeframe`, `provider`)
  - `live_positions` (`id`, `strategy`, `symbol`, `side`, `entry_price`, `size`, `stop_loss`, `take_profit`, `entry_time`, `entry_reason`)
  - `live_trades` (`id`, `strategy`, `symbol`, `side`, `entry_price`, `exit_price`, `size`, `pnl`, `status`, `entry_time`, `exit_time`, `entry_reason`, `exit_reason`)
- [x] Implement CRUD reducers: `insert_candle` (idempotent), `open_position`, `close_position`, `insert_trade`, `delete_candles_by_symbol`, `delete_trades_by_strategy`
- [x] Implement `db-layer` crate using `spacetimedb-sdk` (WebSocket, generated bindings):
  - `error.rs` -- `DbError` enum
  - `models.rs` -- conversion helpers between `module_bindings::` types and `shared::` types
  - `client.rs` -- `SpacetimeClient`: connects via SDK, subscribes all tables on connect, blocks until cache warm (`on_applied`)
  - `queries.rs` -- `get_candles`, `get_candles_before`, `count_candles`, `insert_candle`, `get_open_position`, `open_position`, `close_position`, `insert_trade`, `get_trades` — reads from local cache, writes via reducers
  - `module_bindings/` -- auto-generated via `spacetime generate`, never edited manually
- [x] Write unit tests (no DB required) + integration tests (guarded by `SPACETIMEDB_INTEGRATION=1`), sequential (`--test-threads=1`), with teardown
- [x] Add `justfile` recipes: `db-build`, `db-deploy`, `db-deploy-clean`, `db-generate`, `db-setup`, `db-test`, `db-status`, `db-start`, `db-stop`, `db-restart`, `db-backup`, `db-candles`, `db-trades`, `db-logs`

> Completed: 134 workspace tests total, all green. 7/7 integration tests pass against live SpacetimeDB.
> `spacetimedb-module` excluded from workspace (builds to `wasm32-unknown-unknown`).
> SDK uses WebSocket + local cache — no HTTP REST, no manual JSON parsing.

### Verification
```bash
# Unit tests (no running DB needed)
cargo test -p db-layer

# Integration tests (requires running SpacetimeDB + deployed module)
just db-start        # Terminal 1 — leave running
just db-setup        # build WASM + generate bindings + deploy (once)
just db-test         # run integration tests
```

---

## Milestone 3 -- Indicators Library ✅

**Goal:** All planned technical indicators implemented as pure Rust functions with `O(1)` incremental state support.

### Steps
- [x] Set up `indicators/` crate with module structure (`trend/`, `momentum/`, `volatility/`, `volume/`, `support_resistance/`)
- [x] Implement **Trend** indicators:
  - [x] SMA, EMA, DEMA, TEMA
  - [x] MACD (`MacdResult { line, signal, histogram }`)
  - [x] Parabolic SAR
  - [x] ADX
  - [x] Ichimoku Cloud (`IchimokuResult`)
- [x] Implement **Momentum** indicators:
  - [x] RSI, CCI, Stochastic (`{k, d}`), Williams %R, ROC
- [x] Implement **Volatility** indicators:
  - [x] Bollinger Bands (`BbResult`), ATR, Keltner Channels
- [x] Implement **Volume** indicators:
  - [x] OBV, VWAP, Volume Profile, MFI
- [x] Implement **Support/Resistance**:
  - [x] Pivot Points, Fibonacci Retracements
- [x] Implement **Slope** (linear regression slope)
- [x] All functions return `Option<T>` (return `None` when insufficient data)
- [x] Unit test every indicator against known values

> Completed: 91/91 tests passing. 20 indicators across 5 categories. `ema_series` and `atr_series` helpers shared internally by DEMA/TEMA/Keltner.

### Verification
```bash
cargo test -p indicators
```

---

## Milestone 4 -- Trading Engine (Rhai Scripting + Stateful Indicator Cache) ✅

**Goal:** A working Rhai scripting engine with Rust-like syntax and O(1) incremental indicator updates for the backtester.

### Steps
- [x] Set up `engine/` crate (replaces `lua-engine/` from DESIGN.md)
- [x] `candle_wrapper.rs` -- `CandleWrapper`, `CandleList`, `ContextWrapper`, `PositionWrapper` Rhai custom types: 1-indexed (newest first), indexer, helper methods (`closes()`, `opens()`, `highs()`, `lows()`, `volumes()`)
- [x] `bindings.rs` -- register `indicators::` Rhai module with all 20+ indicators; complex results (MACD, BB, ADX...) returned as Rhai maps
- [x] `indicator_cache.rs` -- stateful O(1) incremental updates for backtesting (keyed by `(IndicatorType, period, offset)`)
- [x] `strategy_loader.rs` -- load, validate and execute `.rhai` strategy files; Rhai syntax validation + `on_tick` existence check
- [x] `vm.rs` -- `Engine`: Rhai engine lifecycle, `tick(candle, context) -> TradeDecision`, Arc/RwLock for candle state sharing, error handling:
  - Rhai exception -- log + abort run
  - missing `signal` key -- error
  - unknown signal value -- error
  - missing `size` on BUY/SELL -- default `1.0`
- [x] `warmup.rs` -- startup warmup logic: read required lookback from script, fetch N historical candles from DB, feed into engine before live ticking begins
- [x] Write unit tests for indicator cache correctness and Lua bindings

> Completed: 27/27 engine tests + 127 workspace total, all passing. Sample strategy `strategies/sma_cross.rhai` added.

### Verification
```bash
cargo test -p engine
```

---

## Milestone 5 -- Trading Daemon (Live & Paper Trading)

**Goal:** A headless, long-running Tokio service that fetches candles, ticks the engine, and executes paper trades.

### Steps
- [ ] Set up `trading-daemon/` binary crate with Tokio async runtime
- [ ] `data_fetcher.rs` -- async task: polls provider API every N minutes, filters incomplete candles (especially Yahoo Finance live candle), writes to SpacetimeDB
- [ ] Implement provider abstraction trait + Yahoo Finance provider (with incomplete candle filtering: `candle.timestamp + interval_ms < now_ms`)
- [ ] `live_engine.rs` -- keeps indicator state warm in RAM; on new candle: O(1) update, run Rhai strategy, emit signal
- [ ] `order_executor.rs` -- paper trading dummy: intercept BUY/SELL signals, simulate trades, persist to `live_trades` / `live_positions` tables in DB
- [ ] `warmup.rs` -- on startup, fetch required historical candles from SpacetimeDB and initialize engine state
- [ ] CLI args via `clap`: `--strategy`, `--symbol`, `--interval`, `--provider`
- [ ] Graceful shutdown (SIGTERM/SIGINT handling)
- [ ] Structured logging via `tracing`

### Verification
```bash
cargo build -p trading-daemon
./target/release/trading-daemon --strategy strategies/sma_cross.rhai --symbol AAPL --interval 5m --provider yahoo
```

---

## Milestone 6 -- GPUI Frontend & In-Memory Backtester

**Goal:** A desktop UI for charting, strategy development, and fast in-memory backtesting. The backtest state lives in the UI's RAM for the entire session -- switching panels and returning to the backtest panel preserves the full state (trades, PnL curve, current tick position) exactly as left.

### Steps
- [ ] Set up `trading-ui/` binary crate with GPUI
- [ ] Basic window + chart canvas
- [ ] Data fetching: connect to SpacetimeDB via SDK, subscribe to `candles` table, load into UI RAM from local cache
- [ ] **In-memory backtest runner** (uses `engine` + `indicators` crates directly, no DB writes):
  - [ ] Feed all loaded candles through a fresh engine instance
  - [ ] Backtest state (`Vec<Trade>`, PnL curve, current tick index) lives in UI RAM for the full app session -- switching panels and returning keeps everything intact
  - [ ] **Manual single-tick mode:** step through the backtest one candle at a time; each step updates the chart, trade table, and PnL in real time so you can inspect every decision
  - [ ] Auto-run mode: run all candles at once (or up to a chosen date)
  - [ ] Collect simulated trades into `Vec<Trade>` in RAM
  - [ ] Compute metrics: PnL, drawdown, win rate, Sharpe
- [ ] Visualize: PnL curve, drawdown chart, trade entry/exit markers on price chart
- [ ] **Trade & profit table panel:** live-updating table of all simulated trades (entry, exit, size, PnL per trade, cumulative PnL) -- updates on each manual tick or at run completion
- [ ] Strategy editor: list `.rhai` files in `strategies/`, simple text editor to modify and save
- [ ] Live trades panel: read `live_trades` from SpacetimeDB, display daemon activity
- [ ] **DB Viewer panel:** read-only view of SpacetimeDB state -- browse `candles`, `live_trades`, `live_positions` tables with basic filtering (symbol, timeframe, date range); shows row counts and last-updated timestamps as a health overview

### Verification
```bash
cargo build -p trading-ui
# Launch app, load AAPL candles, run backtest, verify chart renders correctly
```

---

## Reuse Notes

- `engine/` crate is shared by both `trading-daemon` (live O(1) ticking) and `trading-ui` (in-memory backtest) -- no duplication
- `indicators/` pure functions are used by `engine/` only -- no direct binary dependency
- `db-layer/` is used by both `trading-daemon` and `trading-ui` — both connect via `spacetimedb-sdk` (WebSocket + local cache)

---

## Milestone Order & Dependencies

```
M1 (Workspace + Shared Types)  DONE
  |
  +-- M2 (DB Layer)             DONE
  |
  +-- M3 (Indicators)           DONE
        |
        +-- M4 (Engine / Rhai)       DONE
              |
              +-- M5 (Daemon)
              |
              +-- M6 (UI)
```

M2 and M3 can be developed in parallel after M1.
