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
//!
//! Legacy executor planning helpers are intentionally not part of this crate's public API:
//!
//! ```compile_fail
//! let _ = domain::Action::Nothing;
//! ```
//!
//! ```compile_fail
//! let _ = domain::plan_action;
//! ```
//!
//! ```compile_fail
//! let _ = domain::realized_pnl;
//! ```
//!
//! ```compile_fail
//! let _ = domain::executor::Action::Nothing;
//! ```

pub mod candle;
pub mod position;
pub mod signal;
pub mod timeframe;

pub use candle::Candle;
pub use position::{ClosedPosition, EntryRiskParameters, OpenPosition, PositionSide};
pub use signal::{Signal, TradeDecision};
pub use timeframe::{Timeframe, TimeframeParseError, TimeframeUnit};
