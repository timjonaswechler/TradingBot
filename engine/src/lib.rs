pub mod bindings;
pub mod candle_wrapper;
pub mod error;
pub mod indicator_cache;
pub mod strategy_loader;
pub mod vm;
pub mod warmup;
pub mod warmup_detector;

pub use error::EngineError;
pub use strategy_loader::StrategyConfig;
pub use vm::Engine;
pub use warmup_detector::{detect_warmup_period, DEFAULT_WARMUP};
