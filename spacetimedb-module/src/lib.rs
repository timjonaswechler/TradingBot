/// SpacetimeDB server module for TradingBot2.
///
/// Defines three tables (candles, live_positions, live_trades) and minimal
/// CRUD reducers.  The module intentionally contains **no trading logic** —
/// it is a pure, fast data-lake.
///
/// Compile & deploy via justfile:
/// ```
/// just db-generate   # build WASM + generate Rust client bindings
/// just db-deploy     # publish to local SpacetimeDB server
/// ```
use spacetimedb::{reducer, table, ReducerContext, Table};

// ── Tables ───────────────────────────────────────────────────────────────────

/// One OHLCV candlestick.
///
/// `canonical_id` is a deterministic dedup key: `"{symbol}_{timeframe}_{timestamp_ms}"`
/// so re-inserting the same candle (e.g. after daemon restart) is safe.
#[table(accessor = candles, public)]
#[derive(Clone)]
pub struct Candle {
    /// Auto-incrementing surrogate key.
    #[primary_key]
    #[auto_inc]
    pub id: u64,

    /// Dedup key: `"{symbol}_{timeframe}_{timestamp_ms}"`.
    #[unique]
    pub canonical_id: String,

    /// Candle open time (Unix milliseconds).
    pub timestamp: i64,
    pub symbol: String,
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,
    /// E.g. `"1m"`, `"5m"`, `"1h"`, `"1d"`.
    pub timeframe: String,
    /// E.g. `"yahoo"`, `"binance"`.
    pub provider: String,
}

/// An open (paper or live) trading position managed by the daemon.
#[table(accessor = live_positions, public)]
#[derive(Clone)]
pub struct LivePosition {
    #[primary_key]
    #[auto_inc]
    pub id: u64,

    pub strategy: String,
    pub symbol: String,
    /// `"long"` or `"short"`.
    pub side: String,
    pub entry_price: f64,
    pub size: f64,
    pub stop_loss: f64,
    pub take_profit: f64,
    pub entry_time: i64,
    pub entry_reason: String,
}

/// A completed (closed) trade recorded by the daemon.
#[table(accessor = live_trades, public)]
#[derive(Clone)]
pub struct LiveTrade {
    #[primary_key]
    #[auto_inc]
    pub id: u64,

    pub strategy: String,
    pub symbol: String,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub size: f64,
    pub pnl: f64,
    /// `"open"` or `"closed"`.
    pub status: String,
    pub entry_time: i64,
    pub exit_time: i64,
    pub entry_reason: String,
    pub exit_reason: String,
}

// ── Reducers ─────────────────────────────────────────────────────────────────

/// Insert a candle; silently ignores duplicate `canonical_id` (idempotent).
#[reducer]
pub fn insert_candle(
    ctx: &ReducerContext,
    canonical_id: String,
    timestamp: i64,
    symbol: String,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: f64,
    timeframe: String,
    provider: String,
) {
    // Skip if already present.
    if ctx
        .db
        .candles()
        .canonical_id()
        .find(&canonical_id)
        .is_some()
    {
        return;
    }
    ctx.db.candles().insert(Candle {
        id: 0,
        canonical_id,
        timestamp,
        symbol,
        open,
        high,
        low,
        close,
        volume,
        timeframe,
        provider,
    });
}

/// Open a new position.
#[reducer]
pub fn open_position(
    ctx: &ReducerContext,
    strategy: String,
    symbol: String,
    side: String,
    entry_price: f64,
    size: f64,
    stop_loss: f64,
    take_profit: f64,
    entry_time: i64,
    entry_reason: String,
) {
    ctx.db.live_positions().insert(LivePosition {
        id: 0,
        strategy,
        symbol,
        side,
        entry_price,
        size,
        stop_loss,
        take_profit,
        entry_time,
        entry_reason,
    });
}

/// Close (delete) an open position by its surrogate `id`.
#[reducer]
pub fn close_position(ctx: &ReducerContext, position_id: u64) {
    ctx.db.live_positions().id().delete(&position_id);
}

/// Delete all candles for a given symbol + provider (used for test teardown).
#[reducer]
pub fn delete_candles_by_symbol(ctx: &ReducerContext, symbol: String, provider: String) {
    let ids: Vec<u64> = ctx
        .db
        .candles()
        .iter()
        .filter(|c| c.symbol == symbol && c.provider == provider)
        .map(|c| c.id)
        .collect();
    for id in ids {
        ctx.db.candles().id().delete(&id);
    }
}

/// Delete all trades for a given strategy (used for test teardown).
#[reducer]
pub fn delete_trades_by_strategy(ctx: &ReducerContext, strategy: String) {
    let ids: Vec<u64> = ctx
        .db
        .live_trades()
        .iter()
        .filter(|t| t.strategy == strategy)
        .map(|t| t.id)
        .collect();
    for id in ids {
        ctx.db.live_trades().id().delete(&id);
    }
}

/// Record a completed trade.
#[reducer]
pub fn insert_trade(
    ctx: &ReducerContext,
    strategy: String,
    symbol: String,
    side: String,
    entry_price: f64,
    exit_price: f64,
    size: f64,
    pnl: f64,
    status: String,
    entry_time: i64,
    exit_time: i64,
    entry_reason: String,
    exit_reason: String,
) {
    ctx.db.live_trades().insert(LiveTrade {
        id: 0,
        strategy,
        symbol,
        side,
        entry_price,
        exit_price,
        size,
        pnl,
        status,
        entry_time,
        exit_time,
        entry_reason,
        exit_reason,
    });
}
