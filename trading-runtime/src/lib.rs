//! DB-free Trading Runtime core.
//!
//! This crate is the first runtime-core slice for one runtime asset. It owns
//! strategy-decision planning, runtime-local portfolio state, realized-cash
//! portfolio transitions, explicit force-close commands, warmup progression,
//! ordered runner-neutral runtime events, and [`RuntimeStep`] return values.
//!
//! This slice intentionally does not include Rhai strategy execution, database
//! persistence, live-daemon or backtester wiring, real broker execution,
//! secondary timeframe market views, dynamic risk updates, or stop-loss /
//! take-profit trigger rules. The old `engine` crate remains only a future donor
//! for strategy-handling behavior; this crate must stay independent from it.

pub mod decision;
pub mod events;
pub mod execution;
pub mod portfolio;
pub mod runtime;
pub mod step;
pub mod strategy;

pub use decision::{
    validate_opening_quantity, InvalidOpeningQuantity, StrategyDecision, StrategyDecisionIntent,
};
pub use events::{ForceCloseIgnoredReason, RuntimeEvent};
pub use execution::{plan_execution, ExecutionAction, IgnoredDecisionReason, PlannedExecution};
pub use portfolio::{
    ClosedPosition, PortfolioState, PortfolioTransitionError, RuntimePortfolioSnapshot,
};
pub use runtime::TradingRuntime;
pub use step::RuntimeStep;
pub use strategy::{PredeterminedStrategyHandler, StrategyHandler};
