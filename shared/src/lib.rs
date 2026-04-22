pub mod candle;
pub mod context;
pub mod executor;
pub mod position;
pub mod signal;

pub use candle::Candle;
pub use context::Context;
pub use executor::{plan_action, realized_pnl, Action};
pub use position::{Position, PositionSide};
pub use signal::{Signal, TradeDecision};
