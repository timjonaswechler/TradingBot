#[allow(clippy::all, unused)]
pub mod module_bindings;

pub mod client;
pub mod error;
pub mod models;
pub mod queries;

pub use client::SpacetimeClient;
pub use error::DbError;
pub use models::{
    canonical_id, db_candle_to_domain_candle, db_position_to_shared, DbTrade, PaperExitKind,
    PaperOpenPosition, PaperTrade,
};
pub use module_bindings::{Candle as DbCandle, DbConnection, LivePosition, LiveTrade};
pub use std::sync::Arc;

/// Re-export the most-used query functions at crate root.
pub use queries::{
    close_position, count_candles, count_trades, delete_candles_by_symbol,
    delete_paper_data_by_strategy_identity, delete_trades_by_strategy, get_candle_timestamps,
    get_candles, get_candles_before, get_candles_in_range, get_latest_candle_timestamp,
    get_open_position, get_paper_open_position, get_paper_trades, get_trades, insert_candle,
    insert_trade, open_paper_position, open_position, record_paper_position_closed,
};
