/// All errors that the trading engine can produce.
#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("Rhai error: {0}")]
    Rhai(#[from] Box<rhai::EvalAltResult>),

    #[error("Strategy error: {0}")]
    Strategy(String),

    #[error("Invalid signal from strategy: {0}")]
    InvalidSignal(String),

    #[error("Insufficient data: {0}")]
    InsufficientData(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
