#[allow(clippy::all, unused)]
pub mod module_bindings;

pub mod client;
pub mod error;
pub mod models;
pub mod queries;

pub use client::SpacetimeClient;
pub use error::DbError;
pub use models::{canonical_id, db_candle_to_shared, db_position_to_shared, DbTrade};
pub use module_bindings::{Candle as DbCandle, DbConnection, LivePosition, LiveTrade};
pub use std::sync::Arc;

/// Re-export the most-used query functions at crate root.
pub use queries::{
    close_position,
    count_candles,
    delete_candles_by_symbol,
    delete_trades_by_strategy,
    get_candles,
    get_candles_before,
    get_open_position,
    get_trades,
    insert_candle,
    insert_trade,
    open_position,
};
