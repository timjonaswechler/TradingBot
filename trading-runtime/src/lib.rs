//! DB-free Trading Runtime core.
//!
//! This crate is intentionally empty at first. It will grow into the shared
//! runtime boundary for market input, strategy decisions, portfolio transitions,
//! execution actions, and ordered runtime events.

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
