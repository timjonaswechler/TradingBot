/// All errors that the trading engine can produce.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Invalid signal from strategy: {0}")]
    InvalidSignal(String),

    #[error("Insufficient data: {0}")]
    InsufficientData(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
