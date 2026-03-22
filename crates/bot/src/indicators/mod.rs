pub mod ema;
pub mod macd;
pub mod slope;

pub use ema::compute as compute_ema;
pub use macd::{compute as compute_macd, MacdResult, SlopeAnalysis};
pub use slope::{compute as compute_slope, compute_acceleration, compute_atr};
