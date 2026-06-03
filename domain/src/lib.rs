//! Public domain value API.
//!
//! The legacy runtime context is intentionally not part of this crate's public API:
//!
//! ```compile_fail
//! let _ = domain::Context::new(0.0);
//! ```
//!
//! ```compile_fail
//! let _ = domain::context::Context::new(0.0);
//! ```

pub mod candle;
pub mod executor;
pub mod position;
pub mod signal;
pub mod timeframe;

pub use candle::Candle;
pub use executor::{plan_action, realized_pnl, Action};
pub use position::{ClosedPosition, EntryRiskParameters, OpenPosition, PositionSide};
pub use signal::{Signal, TradeDecision};
pub use timeframe::{Timeframe, TimeframeParseError, TimeframeUnit};
