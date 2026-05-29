//! DB-free Trading Runtime core.
//!
//! This crate is the first runtime-core slice for one runtime asset. It owns
//! strategy-decision planning, runtime-local portfolio state, realized-cash
//! portfolio transitions, explicit force-close commands, warmup progression,
//! ordered runner-neutral runtime events, Rhai strategy loading/hook validation,
//! typed Rhai decisions, grouped Rhai Strategy Context/Strategy State, and
//! [`RuntimeStep`] return values.
//!
//! This slice intentionally does not include database persistence, live-daemon or
//! backtester wiring, real broker execution, dynamic risk updates, or the full
//! typed Market View Rhai API.
//! The old `engine` crate remains only donor material for strategy-handling
//! behavior; this crate must stay independent from it.

pub mod decision;
pub mod events;
pub mod execution;
pub mod market_input;
pub mod market_state;
pub mod portfolio;
pub mod rhai_strategy;
pub mod risk_exit;
pub mod runtime;
pub mod step;
pub mod strategy;
pub mod strategy_config;

pub use decision::{
    validate_opening_quantity, InvalidOpeningQuantity, StrategyDecision, StrategyDecisionIntent,
};
pub use events::{
    BlockedSecondaryContext, ExitKind, ForceCloseIgnoredReason, RuntimeEvent,
    SecondaryContextUnavailableReason,
};
pub use execution::{plan_execution, ExecutionAction, IgnoredDecisionReason, PlannedExecution};
pub use market_input::{
    MarketInput, RuntimeConfig, RuntimeInputError, SecondaryReadiness, SecondaryTimeframeConfig,
};
pub use market_state::MarketState;
pub use portfolio::{
    ClosedPosition, PortfolioState, PortfolioTransitionError, RuntimePortfolioSnapshot,
};
pub use rhai_strategy::{
    AnchoredConfiguration, RhaiStrategy, RhaiStrategyHooks, RhaiStrategyLoadError,
};
pub use risk_exit::{evaluate_risk_exit, RiskExitKind, RiskExitTriggered};
pub use runtime::TradingRuntime;
pub use shared::{Timeframe, TimeframeParseError, TimeframeUnit};
pub use step::RuntimeStep;
pub use strategy::{
    MarketView, PredeterminedStrategyHandler, StrategyContext, StrategyError, StrategyHandler,
    StrategyState, StrategyStateValue, StrategyTickInput, StrategyTickResult,
};
pub use strategy_config::StrategyConfiguration;
