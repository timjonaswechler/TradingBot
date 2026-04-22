# Code Review — Logic Errors & Ghost Code

Scope: M5 trading-daemon + engine glue. M1–M4 libraries are solid; findings concentrate in the daemon wiring layer where live execution was bolted on.

**Status legend:** ✅ done · ⏳ pending

---

## 1. Logic Errors

### 1.1 Only the first interval per asset is ever run ✅ done
`trading-daemon/src/live_engine.rs:34`
```rust
let interval = asset.intervals.first().cloned().unwrap_or_else(|| "1d".into());
```
`main.rs:74` spawns one task **per asset**, then `live_engine::run` picks only `intervals.first()`. Configuring `intervals = ["1d", "1h"]` silently drops `1h`.

**Fix to hit the goal (PLAN: "one task per asset/interval"):** in `main.rs`, iterate `for asset in config.assets { for interval in &asset.intervals { spawn(...) } }` and change `live_engine::run` to take a single `interval: String` instead of pulling it out of `AssetConfig`.

---

### 1.2 Race condition when reading `position_id` back ✅ done (cache-poll + close-time fallback)
`trading-daemon/src/order_executor.rs:89-99`
```rust
open_position(&conn, ...)?;                        // async reducer — fire-and-forget
get_open_position(&conn, &strategy, &symbol)       // read local cache immediately
    .map(|p| p.id)
```
The reducer is async; the inserted row is not guaranteed to be in the SDK cache by the time `get_open_position` runs. Result: `self.position_id = None`, and on the corresponding `close_long_position` the `if let Some(id)` branch is skipped — **the `live_positions` row is never deleted**. The position stays "open" in DB forever, and on restart `PaperExecutor::new` will restore a ghost position.

**Fix:** have the `open_position` reducer return/emit the new `id`, or poll the cache (`on_insert` + oneshot) until the row appears before capturing the id. Alternative: generate the id client-side (UUID/ULID column) so the writer already knows it.

---

### 1.3 `build_context` lies to the strategy ✅ done (real equity MTM + live `trades_count`)
`trading-daemon/src/live_engine.rs:159-167`
```rust
let equity = match &position {
    Some(_) => balance,   // simplified: equity = balance for now
    None    => balance,
};
Context { balance, equity, position, trades_count: 0 }
```
Both arms are identical (dead match), `equity` never reflects unrealized PnL, and `trades_count` is hard-coded `0`. Any strategy reading `context.equity` or `context.trades_count` gets wrong data.

**Fix:** `equity = balance + position.map(|p| (last_close - p.entry_price) * p.size).unwrap_or(0.0)`. `trades_count` should come from `db_layer::get_trades(&conn, strategy, u32::MAX).len()` (cached once, incremented on each close).

---

### 1.4 Seed is non-incremental ✅ done (`get_latest_candle_timestamp` + `effective_from`)
`trading-daemon/src/seed/mod.rs:109-117`
```rust
let existing = count_candles(&*conn, symbol, interval);
// Find the latest timestamp we already have to avoid re-fetching.
// For simplicity, always fetch from from_ms (idempotent insert handles duplicates).
```
`existing` is logged but unused. On every seed run the full history is re-downloaded from Yahoo. Correct thanks to idempotent `canonical_id`, but wasteful and rate-limit-prone.

**Fix:** add `db-layer::get_latest_timestamp(symbol, timeframe) -> Option<i64>`, and in `seed_one` use `max(from_ms, latest + interval_ms)` as the effective start.

---

### 1.5 Duplicate Rhai engine compile in live_engine ✅ done (Engine exposes `ast()` / `scope()`)
`trading-daemon/src/live_engine.rs:45-57`
```rust
let mut tmp_engine = Engine::new(&strategy_src)?;   // compiles once
...
let mut rhai = RhaiEngine::new();
register_types(&mut rhai);
register_all(&mut rhai);
let ast = rhai.compile(&strategy_src).unwrap_or_default();
let mut scope = Scope::new();
let _ = rhai.run_ast_with_scope(&mut scope, &ast);
engine::detect_warmup_period(&ast, &scope)
```
The strategy is compiled twice, top-level code run twice, in two *different* engines with two *different* scopes. `.unwrap_or_default()` silently swallows compile errors (but `Engine::new` would already have returned them, so this is dead error handling). `run_ast_with_scope` return value is `let _ =` → any runtime panic in top-level code is ignored on the second pass.

**Fix:** expose the AST + scope on `engine::Engine` (e.g. `engine.ast() -> &AST`, `engine.scope() -> &Scope`) and call `detect_warmup_period(engine.ast(), engine.scope())` directly. One compile, one scope.

---

### 1.6 `check_stops` ordering vs. incoming candle ✅ done (snapshot `had_position_at_start` before entry)
`trading-daemon/src/order_executor.rs:200-217` calls `check_stops(candle)` before `open_long_position`. If `candle.low <= SL` on the *same* candle that also produces a BUY signal (after a prior SELL intra-candle), stops close a fresh position that was opened moments ago at `candle.close`. For paper trading using close-only fills this is fine today, but the OHLC-based stop check is inconsistent with close-based entries — you can hit a stop on the wick of the entry candle.

**Fix:** only run `check_stops` when the position existed **before** this candle (i.e. cache `had_position_at_tick_start` at the top of `handle`).

---

## 2. Ghost Code

| Status | Location | Item |
|---|---|---|
| ✅ | `engine/src/vm.rs:51-54` | Dead `call_fn::<Dynamic>` probe — deleted. |
| ✅ | `trading-daemon/src/order_executor.rs` | `fn now_ms()` deleted. |
| ✅ | `trading-daemon/src/order_executor.rs` | `Signal::Short` / `Signal::Cover` fully implemented in `PaperExecutor` (side-aware entry/exit, sign-flipped PnL, side-specific stop/TP trigger, `live_trades.side = "short"`). |
| ✅ | `trading-daemon/src/live_engine.rs` | Dead identical-arm `match` replaced with real equity MTM. |
| ✅ | `engine/src/warmup.rs` | `required_warmup_bars` deleted (+ its test). |
| ✅ | `trading-daemon/src/seed/mod.rs` | `existing` now joined by `latest` → drives `effective_from`. |
| ✅ | `trading-daemon/src/live_engine.rs` | `try_send` drop now emits a `warn!`. |
| ✅ | `trading-daemon/src/live_engine.rs` | `tmp_engine` shadow removed; `engine` built once. |
| ✅ | `justfile` | `just seed` + `just run` recipes added. |

---

## 3. Missing Pieces to Hit the M5 Goal

Per `PLAN.md` "Milestone 5" goal ("react to new candles … tick Rhai strategies … execute paper trades") and its own trailing punch-list:

1. ✅ **Multi-interval task fan-out** (§1.1).
2. ✅ **Reliable `position_id` capture** (§1.2) — poll + fallback.
3. ✅ **Real `Context` fields** (§1.3) — equity MTM + live `trades_count`.
4. ✅ **Incremental seed** (§1.4).
5. ✅ **Short/Cover** implemented end-to-end — covered by `short_cover_cycle_profits_on_price_drop` integration test (asserts: short row written with `side="short"`, stray SELL while short is a no-op, COVER flattens, PnL = `(entry - exit) * size`, balance updates).
6. ✅ **`just seed` + `just run` recipes** added, `RUST_LOG=trading_daemon=info,db_layer=info` preset on `daemon` / `seed` / `run` (overridable via env).
7. ✅ **End-to-end integration test** for the daemon — `trading-daemon/tests/integration.rs` drives `PaperExecutor` through a BUY → HOLD → SELL cycle and asserts the live_positions row appears/disappears and live_trades/PnL/balance are correct (gated on `SPACETIMEDB_INTEGRATION=1`).
8. ✅ **Graceful shutdown of open positions** — on cancel, liquidate at the freshest observed candle close (fallback: newest DB bar). Behind `liquidate_on_shutdown` (default `true`) in `AssetConfig`; `false` preserves the persist-and-restore path. `PaperExecutor::liquidate` is covered by an integration test.
9. ✅ **`on_insert` backpressure** — drops now `warn!`-logged (not eliminated, but observable).
10. ✅ **Warmup double-count risk** — `warmup_engine` now returns a `WarmupResult { loaded, high_water_ts }`; `on_insert` drops any candle whose timestamp is ≤ `high_water_ts`, so replayed bars can't be ticked twice.

---

## 4. Suggested Order of Fixes

1. ✅ §1.5 (expose AST on Engine) — unblocks §1.1 cleanup.
2. ✅ §1.1 (multi-interval) + justfile recipes.
3. ✅ §1.2 (position_id race) — correctness-critical.
4. ✅ §1.3 (real Context) — strategy API correctness.
5. ✅ §1.4 (incremental seed).
6. ✅ Integration test covering the above (§3.7).
7. ✅ Ghost code from §2 removed (`required_warmup_bars`, Short/Cover warn-branch, dead `match`, shadow `tmp_engine`, silent `try_send` drop, missing `now_ms`).
