use thiserror::Error;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("SpacetimeDB SDK error: {0}")]
    Sdk(#[from] spacetimedb_sdk::Error),

    #[error("Failed to send reducer request: {0}")]
    ReducerSend(String),

    #[error("Not connected or subscription not yet applied")]
    NotReady,
}
