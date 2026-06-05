use domain::TimeframeParseError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("SpacetimeDB SDK error: {0}")]
    Sdk(#[from] spacetimedb_sdk::Error),

    #[error("Failed to send reducer request: {0}")]
    ReducerSend(String),

    #[error("{0}")]
    PaperPersistenceInconsistency(String),

    #[error("invalid DB candle timeframe '{timeframe}' for candle '{canonical_id}' ({symbol} @ {timestamp}): {source}")]
    InvalidCandleTimeframe {
        timeframe: String,
        canonical_id: String,
        symbol: String,
        timestamp: i64,
        #[source]
        source: TimeframeParseError,
    },

    #[error("Not connected or subscription not yet applied")]
    NotReady,
}
