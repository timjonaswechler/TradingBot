/// SpacetimeDB server module for TradingBot2.
///
/// Defines market-data tables, transitional live position/trade tables, and
/// dedicated Paper Trading persistence tables. The module intentionally
/// contains **no trading logic** — it only enforces storage invariants such as
/// projection idempotency and one open Paper Trading position per strategy and
/// runtime asset.
///
/// Compile & deploy via justfile:
/// ```
/// just db-generate   # build WASM + generate Rust client bindings
/// just db-deploy     # publish to local SpacetimeDB server
/// ```
use spacetimedb::{reducer, table, ReducerContext, SpacetimeType, Table};

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

/// A transitional legacy open-position row.
///
/// Runtime-backed Paper Trading uses `paper_open_positions`; this table remains
/// only for legacy storage/admin compatibility while old data paths are retired.
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

/// A transitional legacy completed-trade row.
///
/// Runtime-backed Paper Trading uses `paper_trades`; this table remains only
/// for legacy storage/admin compatibility while old data paths are retired.
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

/// Machine-readable close category for completed Paper Trading positions.
#[derive(SpacetimeType, Clone, PartialEq, Eq)]
pub enum PaperExitKind {
    StrategyExit,
    RiskExitStopLoss,
    RiskExitTakeProfit,
    ForceClose,
}

/// Currently open Paper Trading position projected from Runtime Portfolio State.
#[table(accessor = paper_open_positions, public)]
#[derive(Clone)]
pub struct PaperOpenPosition {
    /// Deterministic projection key for this runtime-local open position.
    #[primary_key]
    pub projection_key: String,

    /// Operator-owned Strategy Identity.
    pub strategy_identity: String,
    /// Canonical Runtime Asset.
    pub runtime_asset: String,
    /// `"long"` or `"short"`.
    pub side: String,
    /// Effective runtime entry price kept under the historical field name.
    pub entry_price: f64,
    pub quantity: f64,
    pub entry_time: i64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub entry_metadata: Option<String>,
    #[default(None::<f64>)]
    pub entry_base_price: Option<f64>,
    #[default(None::<f64>)]
    pub entry_effective_fill_price: Option<f64>,
    #[default(None::<f64>)]
    pub entry_spread_adjustment: Option<f64>,
    #[default(None::<f64>)]
    pub entry_fixed_fee: Option<f64>,
    #[default(None::<f64>)]
    pub entry_percent_fee: Option<f64>,
    #[default(None::<f64>)]
    pub entry_total_cost: Option<f64>,
}

/// Completed Paper Trading position projected from Runtime Portfolio State.
#[table(accessor = paper_trades, public)]
#[derive(Clone)]
pub struct PaperTrade {
    /// Deterministic projection key for this completed position.
    #[primary_key]
    pub projection_key: String,

    /// Operator-owned Strategy Identity.
    pub strategy_identity: String,
    /// Canonical Runtime Asset.
    pub runtime_asset: String,
    /// `"long"` or `"short"`.
    pub side: String,
    /// Effective runtime entry price kept under the historical field name.
    pub entry_price: f64,
    /// Effective runtime exit price kept under the historical field name.
    pub exit_price: f64,
    pub quantity: f64,
    /// Net realized PnL kept under the historical field name.
    pub realized_pnl: f64,
    pub entry_time: i64,
    pub exit_time: i64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub exit_kind: PaperExitKind,
    pub entry_metadata: Option<String>,
    pub exit_metadata: Option<String>,
    #[default(None::<f64>)]
    pub entry_base_price: Option<f64>,
    #[default(None::<f64>)]
    pub entry_effective_fill_price: Option<f64>,
    #[default(None::<f64>)]
    pub entry_spread_adjustment: Option<f64>,
    #[default(None::<f64>)]
    pub entry_fixed_fee: Option<f64>,
    #[default(None::<f64>)]
    pub entry_percent_fee: Option<f64>,
    #[default(None::<f64>)]
    pub entry_total_cost: Option<f64>,
    #[default(None::<f64>)]
    pub exit_base_price: Option<f64>,
    #[default(None::<f64>)]
    pub exit_effective_fill_price: Option<f64>,
    #[default(None::<f64>)]
    pub exit_spread_adjustment: Option<f64>,
    #[default(None::<f64>)]
    pub exit_fixed_fee: Option<f64>,
    #[default(None::<f64>)]
    pub exit_percent_fee: Option<f64>,
    #[default(None::<f64>)]
    pub exit_total_cost: Option<f64>,
    #[default(None::<f64>)]
    pub gross_pnl: Option<f64>,
    #[default(None::<f64>)]
    pub total_costs: Option<f64>,
    #[default(None::<f64>)]
    pub net_realized_pnl: Option<f64>,
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

/// Open a transitional legacy `live_positions` row.
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

/// Close (delete) a transitional legacy `live_positions` row by surrogate `id`.
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

/// Record a transitional legacy `live_trades` row.
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

/// Project a runtime-opened Paper Trading position into `paper_open_positions`.
///
/// The operation is idempotent only for the same projection key and identical
/// position data. A different existing open position for the same Strategy
/// Identity × Runtime Asset is a Paper Trading persistence inconsistency.
#[reducer]
pub fn open_paper_position(
    ctx: &ReducerContext,
    projection_key: String,
    strategy_identity: String,
    runtime_asset: String,
    side: String,
    entry_price: f64,
    quantity: f64,
    entry_time: i64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
    entry_metadata: Option<String>,
    entry_base_price: Option<f64>,
    entry_effective_fill_price: Option<f64>,
    entry_spread_adjustment: Option<f64>,
    entry_fixed_fee: Option<f64>,
    entry_percent_fee: Option<f64>,
    entry_total_cost: Option<f64>,
) -> Result<(), String> {
    if let Some(existing) = ctx
        .db
        .paper_open_positions()
        .projection_key()
        .find(&projection_key)
    {
        if paper_open_position_matches(
            &existing,
            &projection_key,
            &strategy_identity,
            &runtime_asset,
            &side,
            entry_price,
            quantity,
            entry_time,
            &stop_loss,
            &take_profit,
            &entry_metadata,
            entry_base_price,
            entry_effective_fill_price,
            entry_spread_adjustment,
            entry_fixed_fee,
            entry_percent_fee,
            entry_total_cost,
        ) {
            return Ok(());
        }

        return Err(format!(
            "paper persistence inconsistency: open paper position projection key '{projection_key}' already exists with different data"
        ));
    }

    if let Some(conflict) = ctx.db.paper_open_positions().iter().find(|position| {
        position.strategy_identity == strategy_identity && position.runtime_asset == runtime_asset
    }) {
        return Err(format!(
            "paper persistence inconsistency: open paper position already exists for strategy_identity '{strategy_identity}' and runtime_asset '{runtime_asset}' with projection key '{}'",
            conflict.projection_key
        ));
    }

    ctx.db.paper_open_positions().insert(PaperOpenPosition {
        projection_key,
        strategy_identity,
        runtime_asset,
        side,
        entry_price,
        quantity,
        entry_time,
        stop_loss,
        take_profit,
        entry_metadata,
        entry_base_price,
        entry_effective_fill_price,
        entry_spread_adjustment,
        entry_fixed_fee,
        entry_percent_fee,
        entry_total_cost,
    });

    Ok(())
}

/// Update current Position Risk Boundaries for a projected Paper Trading position.
///
/// The operation is idempotent when the same boundary state is projected again.
/// A missing or different open position for the Strategy Identity × Runtime
/// Asset is a Paper Trading persistence inconsistency.
#[reducer]
pub fn update_paper_position_risk_boundaries(
    ctx: &ReducerContext,
    open_projection_key: String,
    strategy_identity: String,
    runtime_asset: String,
    side: String,
    entry_price: f64,
    quantity: f64,
    entry_time: i64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> Result<(), String> {
    let Some(existing) = ctx
        .db
        .paper_open_positions()
        .projection_key()
        .find(&open_projection_key)
    else {
        if let Some(conflict) = ctx.db.paper_open_positions().iter().find(|position| {
            position.strategy_identity == strategy_identity && position.runtime_asset == runtime_asset
        }) {
            return Err(format!(
                "paper persistence inconsistency: open paper position for strategy_identity '{strategy_identity}' and runtime_asset '{runtime_asset}' has projection key '{}', expected '{open_projection_key}'",
                conflict.projection_key
            ));
        }

        return Err(format!(
            "paper persistence inconsistency: no matching open paper position for open projection key '{open_projection_key}'"
        ));
    };

    if !paper_open_position_identity_matches(
        &existing,
        &open_projection_key,
        &strategy_identity,
        &runtime_asset,
        &side,
        entry_price,
        quantity,
        entry_time,
    ) {
        return Err(format!(
            "paper persistence inconsistency: open paper position '{open_projection_key}' does not match projected Position Risk Update identity"
        ));
    }

    if existing.stop_loss == stop_loss && existing.take_profit == take_profit {
        return Ok(());
    }

    ctx.db
        .paper_open_positions()
        .projection_key()
        .delete(&open_projection_key);
    ctx.db.paper_open_positions().insert(PaperOpenPosition {
        projection_key: existing.projection_key,
        strategy_identity: existing.strategy_identity,
        runtime_asset: existing.runtime_asset,
        side: existing.side,
        entry_price: existing.entry_price,
        quantity: existing.quantity,
        entry_time: existing.entry_time,
        stop_loss,
        take_profit,
        entry_metadata: existing.entry_metadata,
        entry_base_price: existing.entry_base_price,
        entry_effective_fill_price: existing.entry_effective_fill_price,
        entry_spread_adjustment: existing.entry_spread_adjustment,
        entry_fixed_fee: existing.entry_fixed_fee,
        entry_percent_fee: existing.entry_percent_fee,
        entry_total_cost: existing.entry_total_cost,
    });

    Ok(())
}

/// Atomically close a projected Paper Trading position.
///
/// If the completed trade already exists with identical data, this is a no-op.
/// Otherwise the reducer requires a matching open position, removes it, and
/// inserts the completed trade in the same SpacetimeDB transaction.
#[reducer]
pub fn record_paper_position_closed(
    ctx: &ReducerContext,
    open_projection_key: String,
    trade_projection_key: String,
    strategy_identity: String,
    runtime_asset: String,
    side: String,
    entry_price: f64,
    exit_price: f64,
    quantity: f64,
    realized_pnl: f64,
    entry_time: i64,
    exit_time: i64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
    exit_kind: PaperExitKind,
    entry_metadata: Option<String>,
    exit_metadata: Option<String>,
    entry_base_price: Option<f64>,
    entry_effective_fill_price: Option<f64>,
    entry_spread_adjustment: Option<f64>,
    entry_fixed_fee: Option<f64>,
    entry_percent_fee: Option<f64>,
    entry_total_cost: Option<f64>,
    exit_base_price: Option<f64>,
    exit_effective_fill_price: Option<f64>,
    exit_spread_adjustment: Option<f64>,
    exit_fixed_fee: Option<f64>,
    exit_percent_fee: Option<f64>,
    exit_total_cost: Option<f64>,
    gross_pnl: Option<f64>,
    total_costs: Option<f64>,
    net_realized_pnl: Option<f64>,
) -> Result<(), String> {
    if let Some(existing_trade) = ctx
        .db
        .paper_trades()
        .projection_key()
        .find(&trade_projection_key)
    {
        if paper_trade_matches(
            &existing_trade,
            &trade_projection_key,
            &strategy_identity,
            &runtime_asset,
            &side,
            entry_price,
            exit_price,
            quantity,
            realized_pnl,
            entry_time,
            exit_time,
            &stop_loss,
            &take_profit,
            &exit_kind,
            &entry_metadata,
            &exit_metadata,
            entry_base_price,
            entry_effective_fill_price,
            entry_spread_adjustment,
            entry_fixed_fee,
            entry_percent_fee,
            entry_total_cost,
            exit_base_price,
            exit_effective_fill_price,
            exit_spread_adjustment,
            exit_fixed_fee,
            exit_percent_fee,
            exit_total_cost,
            gross_pnl,
            total_costs,
            net_realized_pnl,
        ) {
            if let Some(open_position) = ctx
                .db
                .paper_open_positions()
                .projection_key()
                .find(&open_projection_key)
            {
                if !paper_open_position_matches(
                    &open_position,
                    &open_projection_key,
                    &strategy_identity,
                    &runtime_asset,
                    &side,
                    entry_price,
                    quantity,
                    entry_time,
                    &stop_loss,
                    &take_profit,
                    &entry_metadata,
                    entry_base_price,
                    entry_effective_fill_price,
                    entry_spread_adjustment,
                    entry_fixed_fee,
                    entry_percent_fee,
                    entry_total_cost,
                ) {
                    return Err(format!(
                        "paper persistence inconsistency: open paper position '{open_projection_key}' does not match existing completed paper trade '{trade_projection_key}'"
                    ));
                }

                ctx.db
                    .paper_open_positions()
                    .projection_key()
                    .delete(&open_projection_key);
            }

            return Ok(());
        }

        return Err(format!(
            "paper persistence inconsistency: completed paper trade projection key '{trade_projection_key}' already exists with different data"
        ));
    }

    let Some(open_position) = ctx
        .db
        .paper_open_positions()
        .projection_key()
        .find(&open_projection_key)
    else {
        return Err(format!(
            "paper persistence inconsistency: no matching open paper position for open projection key '{open_projection_key}' and no completed paper trade for projection key '{trade_projection_key}'"
        ));
    };

    if !paper_open_position_matches(
        &open_position,
        &open_projection_key,
        &strategy_identity,
        &runtime_asset,
        &side,
        entry_price,
        quantity,
        entry_time,
        &stop_loss,
        &take_profit,
        &entry_metadata,
        entry_base_price,
        entry_effective_fill_price,
        entry_spread_adjustment,
        entry_fixed_fee,
        entry_percent_fee,
        entry_total_cost,
    ) {
        return Err(format!(
            "paper persistence inconsistency: open paper position '{open_projection_key}' does not match completed paper trade '{trade_projection_key}'"
        ));
    }

    ctx.db
        .paper_open_positions()
        .projection_key()
        .delete(&open_projection_key);
    ctx.db.paper_trades().insert(PaperTrade {
        projection_key: trade_projection_key,
        strategy_identity,
        runtime_asset,
        side,
        entry_price,
        exit_price,
        quantity,
        realized_pnl,
        entry_time,
        exit_time,
        stop_loss,
        take_profit,
        exit_kind,
        entry_metadata,
        exit_metadata,
        entry_base_price,
        entry_effective_fill_price,
        entry_spread_adjustment,
        entry_fixed_fee,
        entry_percent_fee,
        entry_total_cost,
        exit_base_price,
        exit_effective_fill_price,
        exit_spread_adjustment,
        exit_fixed_fee,
        exit_percent_fee,
        exit_total_cost,
        gross_pnl,
        total_costs,
        net_realized_pnl,
    });

    Ok(())
}

/// Delete Paper Trading rows for a strategy identity (test/admin cleanup).
#[reducer]
pub fn delete_paper_data_by_strategy_identity(ctx: &ReducerContext, strategy_identity: String) {
    let open_keys: Vec<String> = ctx
        .db
        .paper_open_positions()
        .iter()
        .filter(|position| position.strategy_identity == strategy_identity)
        .map(|position| position.projection_key)
        .collect();
    for projection_key in open_keys {
        ctx.db
            .paper_open_positions()
            .projection_key()
            .delete(&projection_key);
    }

    let trade_keys: Vec<String> = ctx
        .db
        .paper_trades()
        .iter()
        .filter(|trade| trade.strategy_identity == strategy_identity)
        .map(|trade| trade.projection_key)
        .collect();
    for projection_key in trade_keys {
        ctx.db
            .paper_trades()
            .projection_key()
            .delete(&projection_key);
    }
}

#[allow(clippy::too_many_arguments)]
fn paper_open_position_matches(
    position: &PaperOpenPosition,
    projection_key: &str,
    strategy_identity: &str,
    runtime_asset: &str,
    side: &str,
    entry_price: f64,
    quantity: f64,
    entry_time: i64,
    stop_loss: &Option<f64>,
    take_profit: &Option<f64>,
    entry_metadata: &Option<String>,
    entry_base_price: Option<f64>,
    entry_effective_fill_price: Option<f64>,
    entry_spread_adjustment: Option<f64>,
    entry_fixed_fee: Option<f64>,
    entry_percent_fee: Option<f64>,
    entry_total_cost: Option<f64>,
) -> bool {
    paper_open_position_identity_matches(
        position,
        projection_key,
        strategy_identity,
        runtime_asset,
        side,
        entry_price,
        quantity,
        entry_time,
    ) && &position.stop_loss == stop_loss
        && &position.take_profit == take_profit
        && &position.entry_metadata == entry_metadata
        && position.entry_base_price == entry_base_price
        && position.entry_effective_fill_price == entry_effective_fill_price
        && position.entry_spread_adjustment == entry_spread_adjustment
        && position.entry_fixed_fee == entry_fixed_fee
        && position.entry_percent_fee == entry_percent_fee
        && position.entry_total_cost == entry_total_cost
}

#[allow(clippy::too_many_arguments)]
fn paper_open_position_identity_matches(
    position: &PaperOpenPosition,
    projection_key: &str,
    strategy_identity: &str,
    runtime_asset: &str,
    side: &str,
    entry_price: f64,
    quantity: f64,
    entry_time: i64,
) -> bool {
    position.projection_key == projection_key
        && position.strategy_identity == strategy_identity
        && position.runtime_asset == runtime_asset
        && position.side == side
        && position.entry_price == entry_price
        && position.quantity == quantity
        && position.entry_time == entry_time
}

#[allow(clippy::too_many_arguments)]
fn paper_trade_matches(
    trade: &PaperTrade,
    projection_key: &str,
    strategy_identity: &str,
    runtime_asset: &str,
    side: &str,
    entry_price: f64,
    exit_price: f64,
    quantity: f64,
    realized_pnl: f64,
    entry_time: i64,
    exit_time: i64,
    stop_loss: &Option<f64>,
    take_profit: &Option<f64>,
    exit_kind: &PaperExitKind,
    entry_metadata: &Option<String>,
    exit_metadata: &Option<String>,
    entry_base_price: Option<f64>,
    entry_effective_fill_price: Option<f64>,
    entry_spread_adjustment: Option<f64>,
    entry_fixed_fee: Option<f64>,
    entry_percent_fee: Option<f64>,
    entry_total_cost: Option<f64>,
    exit_base_price: Option<f64>,
    exit_effective_fill_price: Option<f64>,
    exit_spread_adjustment: Option<f64>,
    exit_fixed_fee: Option<f64>,
    exit_percent_fee: Option<f64>,
    exit_total_cost: Option<f64>,
    gross_pnl: Option<f64>,
    total_costs: Option<f64>,
    net_realized_pnl: Option<f64>,
) -> bool {
    trade.projection_key == projection_key
        && trade.strategy_identity == strategy_identity
        && trade.runtime_asset == runtime_asset
        && trade.side == side
        && trade.entry_price == entry_price
        && trade.exit_price == exit_price
        && trade.quantity == quantity
        && trade.realized_pnl == realized_pnl
        && trade.entry_time == entry_time
        && trade.exit_time == exit_time
        && &trade.stop_loss == stop_loss
        && &trade.take_profit == take_profit
        && &trade.exit_kind == exit_kind
        && &trade.entry_metadata == entry_metadata
        && &trade.exit_metadata == exit_metadata
        && trade.entry_base_price == entry_base_price
        && trade.entry_effective_fill_price == entry_effective_fill_price
        && trade.entry_spread_adjustment == entry_spread_adjustment
        && trade.entry_fixed_fee == entry_fixed_fee
        && trade.entry_percent_fee == entry_percent_fee
        && trade.entry_total_cost == entry_total_cost
        && trade.exit_base_price == exit_base_price
        && trade.exit_effective_fill_price == exit_effective_fill_price
        && trade.exit_spread_adjustment == exit_spread_adjustment
        && trade.exit_fixed_fee == exit_fixed_fee
        && trade.exit_percent_fee == exit_percent_fee
        && trade.exit_total_cost == exit_total_cost
        && trade.gross_pnl == gross_pnl
        && trade.total_costs == total_costs
        && trade.net_realized_pnl == net_realized_pnl
}
