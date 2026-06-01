/// High-level query helpers for the TradingBot2 SpacetimeDB module.
///
/// All reads operate on the local client cache (`conn.db.<table>().iter()`).
/// All writes call reducers via the generated bindings (`conn.reducers.<reducer>()`).
///
/// No HTTP, no manual JSON — the SDK handles everything.
use spacetimedb_sdk::Table;

use shared::Candle;

use crate::{
    error::DbError,
    models::{candle_to_reducer_args, db_candle_to_shared},
    module_bindings::{
        // Reducer extension traits — must be in scope for .insert_candle(), etc.
        close_position,
        delete_candles_by_symbol,
        delete_trades_by_strategy,
        insert_candle,
        insert_trade,
        open_position,
        // Table access traits — must be in scope for .candles(), .live_positions(), .live_trades()
        CandlesTableAccess,
        DbConnection,
        LivePosition,
        LivePositionsTableAccess,
        LiveTrade,
        LiveTradesTableAccess,
    },
};

// ── Candle queries ────────────────────────────────────────────────────────────

/// Insert a candle (idempotent — module ignores duplicate `canonical_id`).
pub fn insert_candle(conn: &DbConnection, candle: &Candle, provider: &str) -> Result<(), DbError> {
    let (cid, ts, sym, o, h, l, c, v, tf, prov) = candle_to_reducer_args(candle, provider);
    conn.reducers
        .insert_candle(cid, ts, sym, o, h, l, c, v, tf, prov)
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}

/// Fetch up to `limit` candles for `symbol` / `timeframe` in chronological order.
/// Reads from the local cache — no network call.
pub fn get_candles(conn: &DbConnection, symbol: &str, timeframe: &str, limit: u32) -> Vec<Candle> {
    let mut candles: Vec<Candle> = conn
        .db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.timeframe == timeframe)
        .map(|c| db_candle_to_shared(c.clone()))
        .collect();

    candles.sort_by_key(|c| c.timestamp);

    let limit = limit as usize;
    if candles.len() > limit {
        candles = candles.into_iter().rev().take(limit).collect();
        candles.reverse();
    }
    candles
}

/// Fetch up to `limit` candles for `symbol` / `timeframe` with half-open
/// candle timestamps in `[start_ts, end_ts)` in chronological order.
/// Reads from the local cache — no network call.
pub fn get_candles_in_range(
    conn: &DbConnection,
    symbol: &str,
    timeframe: &str,
    start_ts: i64,
    end_ts: i64,
    limit: u32,
) -> Vec<Candle> {
    let mut candles: Vec<Candle> = conn
        .db
        .candles()
        .iter()
        .filter(|c| {
            c.symbol == symbol
                && c.timeframe == timeframe
                && c.timestamp >= start_ts
                && c.timestamp < end_ts
        })
        .map(|c| db_candle_to_shared(c.clone()))
        .collect();

    candles.sort_by_key(|c| c.timestamp);

    let limit = limit as usize;
    if candles.len() > limit {
        candles = candles.into_iter().rev().take(limit).collect();
        candles.reverse();
    }
    candles
}

/// Fetch up to `limit` candles **before** `before_ts` (exclusive) in chronological order.
/// Used for engine warmup on daemon startup. Reads from the local cache.
pub fn get_candles_before(
    conn: &DbConnection,
    symbol: &str,
    timeframe: &str,
    before_ts: i64,
    limit: u32,
) -> Vec<Candle> {
    let mut candles: Vec<Candle> = conn
        .db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.timeframe == timeframe && c.timestamp < before_ts)
        .map(|c| db_candle_to_shared(c.clone()))
        .collect();

    candles.sort_by_key(|c| c.timestamp);

    let limit = limit as usize;
    if candles.len() > limit {
        candles = candles.into_iter().rev().take(limit).collect();
        candles.reverse();
    }
    candles
}

/// Count candles for a symbol/timeframe from the local cache.
pub fn count_candles(conn: &DbConnection, symbol: &str, timeframe: &str) -> u64 {
    conn.db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.timeframe == timeframe)
        .count() as u64
}

/// All timestamps (Unix ms) present in the cache for `symbol` / `timeframe`,
/// as a `HashSet` for O(1) membership checks. Used by the seed pipeline to
/// skip candles that are already stored without re-inserting them.
pub fn get_candle_timestamps(
    conn: &DbConnection,
    symbol: &str,
    timeframe: &str,
) -> std::collections::HashSet<i64> {
    conn.db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.timeframe == timeframe)
        .map(|c| c.timestamp)
        .collect()
}

/// Most recent candle timestamp (Unix ms) for a symbol/timeframe, or `None`
/// if no candles exist. Used by the seed pipeline to fetch only new bars.
pub fn get_latest_candle_timestamp(
    conn: &DbConnection,
    symbol: &str,
    timeframe: &str,
) -> Option<i64> {
    conn.db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.timeframe == timeframe)
        .map(|c| c.timestamp)
        .max()
}

// ── Position queries ──────────────────────────────────────────────────────────

/// Open a new position in `live_positions`.
#[allow(clippy::too_many_arguments)]
pub fn open_position(
    conn: &DbConnection,
    strategy: &str,
    symbol: &str,
    side: &str,
    entry_price: f64,
    size: f64,
    stop_loss: f64,
    take_profit: f64,
    entry_time: i64,
    entry_reason: &str,
) -> Result<(), DbError> {
    conn.reducers
        .open_position(
            strategy.to_string(),
            symbol.to_string(),
            side.to_string(),
            entry_price,
            size,
            stop_loss,
            take_profit,
            entry_time,
            entry_reason.to_string(),
        )
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}

/// Close (delete) a position by its `id`.
pub fn close_position(conn: &DbConnection, position_id: u64) -> Result<(), DbError> {
    conn.reducers
        .close_position(position_id)
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}

/// Fetch the open position for a given strategy + symbol from the local cache.
pub fn get_open_position(
    conn: &DbConnection,
    strategy: &str,
    symbol: &str,
) -> Option<LivePosition> {
    conn.db
        .live_positions()
        .iter()
        .find(|p| p.strategy == strategy && p.symbol == symbol)
        .map(|p| p.clone())
}

// ── Trade queries ─────────────────────────────────────────────────────────────

/// Record a completed trade in `live_trades`.
#[allow(clippy::too_many_arguments)]
pub fn insert_trade(
    conn: &DbConnection,
    strategy: &str,
    symbol: &str,
    side: &str,
    entry_price: f64,
    exit_price: f64,
    size: f64,
    pnl: f64,
    status: &str,
    entry_time: i64,
    exit_time: i64,
    entry_reason: &str,
    exit_reason: &str,
) -> Result<(), DbError> {
    conn.reducers
        .insert_trade(
            strategy.to_string(),
            symbol.to_string(),
            side.to_string(),
            entry_price,
            exit_price,
            size,
            pnl,
            status.to_string(),
            entry_time,
            exit_time,
            entry_reason.to_string(),
            exit_reason.to_string(),
        )
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}

/// Count trades in `live_trades` for a given strategy + symbol from the local cache.
pub fn count_trades(conn: &DbConnection, strategy: &str, symbol: &str) -> u64 {
    conn.db
        .live_trades()
        .iter()
        .filter(|t| t.strategy == strategy && t.symbol == symbol)
        .count() as u64
}

/// Fetch recent trades for a strategy from the local cache (newest first, capped at `limit`).
pub fn get_trades(conn: &DbConnection, strategy: &str, limit: u32) -> Vec<LiveTrade> {
    let mut trades: Vec<LiveTrade> = conn
        .db
        .live_trades()
        .iter()
        .filter(|t| t.strategy == strategy)
        .map(|t| t.clone())
        .collect();

    trades.sort_by(|a, b| b.exit_time.cmp(&a.exit_time));
    trades.truncate(limit as usize);
    trades
}

// ── Cleanup (test teardown) ───────────────────────────────────────────────────

/// Delete all candles for a given symbol + provider via reducer.
/// Used in integration test teardown.
pub fn delete_candles_by_symbol(
    conn: &DbConnection,
    symbol: &str,
    provider: &str,
) -> Result<(), DbError> {
    conn.reducers
        .delete_candles_by_symbol(symbol.to_string(), provider.to_string())
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}

/// Delete all trades for a given strategy via reducer.
/// Used in integration test teardown.
pub fn delete_trades_by_strategy(conn: &DbConnection, strategy: &str) -> Result<(), DbError> {
    conn.reducers
        .delete_trades_by_strategy(strategy.to_string())
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}
