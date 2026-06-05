/// High-level query helpers for the TradingBot2 SpacetimeDB module.
///
/// All reads operate on the local client cache (`conn.db.<table>().iter()`).
/// All writes call reducers via the generated bindings (`conn.reducers.<reducer>()`).
///
/// No HTTP, no manual JSON — the SDK handles everything.
use std::time::Duration;

use spacetimedb_sdk::Table;

use domain::Candle;

use crate::{
    error::DbError,
    models::{candle_to_reducer_args, db_candle_to_domain_candle},
    module_bindings::{
        // Reducer extension traits — must be in scope for .insert_candle(), etc.
        close_position,
        delete_candles_by_symbol,
        delete_paper_data_by_strategy_identity as delete_paper_data_by_strategy_identity_reducer,
        delete_trades_by_strategy,
        insert_candle,
        insert_trade,
        open_paper_position as open_paper_position_reducer,
        open_position,
        record_paper_position_closed as record_paper_position_closed_reducer,
        // Table access traits — must be in scope for table handles.
        CandlesTableAccess,
        DbConnection,
        LivePosition,
        LivePositionsTableAccess,
        LiveTrade,
        LiveTradesTableAccess,
        PaperOpenPosition,
        PaperOpenPositionsTableAccess,
        PaperTrade,
        PaperTradesTableAccess,
    },
};

const PAPER_REDUCER_TIMEOUT: Duration = Duration::from_secs(2);

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
        .map(|c| db_candle_to_domain_candle(c.clone()))
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
        .map(|c| db_candle_to_domain_candle(c.clone()))
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
        .map(|c| db_candle_to_domain_candle(c.clone()))
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

// ── Transitional legacy live position queries ────────────────────────────────
//
// These helpers operate on the old `live_positions` table. They are retained
// for legacy storage/admin coverage only and are not the target Paper Trading
// persistence path; runtime-backed Paper Trading uses the paper_* helpers below.

/// Open a new transitional legacy position in `live_positions`.
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
}

// ── Paper Trading persistence queries ────────────────────────────────────────

/// Project a runtime-opened Paper Trading position into `paper_open_positions`.
///
/// This helper waits for reducer completion so idempotency/conflict errors are
/// surfaced to the caller instead of being hidden as asynchronous reducer logs.
pub fn open_paper_position(
    conn: &DbConnection,
    position: &PaperOpenPosition,
) -> Result<(), DbError> {
    let (tx, rx) = std::sync::mpsc::channel();
    conn.reducers
        .open_paper_position_then(
            position.projection_key.clone(),
            position.strategy_identity.clone(),
            position.runtime_asset.clone(),
            position.side.clone(),
            position.entry_price,
            position.quantity,
            position.entry_time,
            position.stop_loss,
            position.take_profit,
            position.entry_metadata.clone(),
            move |_ctx, result| {
                let _ = tx.send(result.map_err(|error| format!("{error:?}")));
            },
        )
        .map_err(|e| DbError::ReducerSend(e.to_string()))?;

    wait_for_paper_reducer("open_paper_position", rx)
}

/// Atomically record a completed Paper Trading position.
///
/// The reducer deduplicates existing completed trades and otherwise requires a
/// matching open paper position to remove before inserting the trade.
pub fn record_paper_position_closed(
    conn: &DbConnection,
    open_projection_key: &str,
    trade: &PaperTrade,
) -> Result<(), DbError> {
    let (tx, rx) = std::sync::mpsc::channel();
    conn.reducers
        .record_paper_position_closed_then(
            open_projection_key.to_string(),
            trade.projection_key.clone(),
            trade.strategy_identity.clone(),
            trade.runtime_asset.clone(),
            trade.side.clone(),
            trade.entry_price,
            trade.exit_price,
            trade.quantity,
            trade.realized_pnl,
            trade.entry_time,
            trade.exit_time,
            trade.stop_loss,
            trade.take_profit,
            trade.exit_kind,
            trade.entry_metadata.clone(),
            trade.exit_metadata.clone(),
            move |_ctx, result| {
                let _ = tx.send(result.map_err(|error| format!("{error:?}")));
            },
        )
        .map_err(|e| DbError::ReducerSend(e.to_string()))?;

    wait_for_paper_reducer("record_paper_position_closed", rx)
}

fn wait_for_paper_reducer(
    reducer_name: &str,
    rx: std::sync::mpsc::Receiver<Result<Result<(), String>, String>>,
) -> Result<(), DbError> {
    match rx.recv_timeout(PAPER_REDUCER_TIMEOUT) {
        Ok(Ok(Ok(()))) => Ok(()),
        Ok(Ok(Err(message))) => Err(DbError::PaperPersistenceInconsistency(message)),
        Ok(Err(message)) => Err(DbError::ReducerSend(format!(
            "{reducer_name} reducer callback failed: {message}"
        ))),
        Err(error) => Err(DbError::ReducerSend(format!(
            "{reducer_name} reducer did not confirm within {:?}: {error}",
            PAPER_REDUCER_TIMEOUT
        ))),
    }
}

/// Fetch the open Paper Trading position for a Strategy Identity × Runtime Asset.
pub fn get_paper_open_position(
    conn: &DbConnection,
    strategy_identity: &str,
    runtime_asset: &str,
) -> Option<PaperOpenPosition> {
    conn.db.paper_open_positions().iter().find(|position| {
        position.strategy_identity == strategy_identity && position.runtime_asset == runtime_asset
    })
}

/// Fetch completed Paper Trading positions for a Strategy Identity × Runtime Asset.
pub fn get_paper_trades(
    conn: &DbConnection,
    strategy_identity: &str,
    runtime_asset: &str,
) -> Vec<PaperTrade> {
    let mut trades: Vec<PaperTrade> = conn
        .db
        .paper_trades()
        .iter()
        .filter(|trade| {
            trade.strategy_identity == strategy_identity && trade.runtime_asset == runtime_asset
        })
        .collect();

    trades.sort_by_key(|trade| trade.exit_time);
    trades
}

// ── Transitional legacy live trade queries ───────────────────────────────────
//
// These helpers operate on the old `live_trades` table. They are retained for
// legacy storage/admin coverage only and are not the target Paper Trading
// persistence path.

/// Record a completed transitional legacy trade in `live_trades`.
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

/// Delete all Paper Trading rows for a strategy identity via reducer.
/// Used in integration test teardown.
pub fn delete_paper_data_by_strategy_identity(
    conn: &DbConnection,
    strategy_identity: &str,
) -> Result<(), DbError> {
    conn.reducers
        .delete_paper_data_by_strategy_identity(strategy_identity.to_string())
        .map_err(|e| DbError::ReducerSend(e.to_string()))
}
