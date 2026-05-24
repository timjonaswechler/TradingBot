//! DB-free Trading Runtime core.
//!
//! This crate is intentionally empty at first. It will grow into the shared
//! runtime boundary for market input, strategy decisions, portfolio transitions,
//! execution actions, and ordered runtime events.

pub mod decision;
pub mod execution;

pub use decision::{
    validate_opening_quantity, InvalidOpeningQuantity, StrategyDecision, StrategyDecisionIntent,
};
pub use execution::{plan_execution, ExecutionAction, IgnoredDecisionReason, PlannedExecution};
