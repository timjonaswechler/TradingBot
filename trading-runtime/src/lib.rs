//! DB-free Trading Runtime core.
//!
//! This crate is the first runtime-core slice for one runtime asset. It owns
//! strategy-decision planning, runtime-local portfolio state, realized-cash
//! portfolio transitions, explicit force-close commands, warmup progression,
//! ordered runner-neutral runtime events, Rhai strategy loading/hook validation,
//! typed Rhai decisions, grouped Rhai Strategy Context/Strategy State, typed
//! Rhai Market View access for configured Primary/Secondary timeframes, and
//! [`RuntimeStep`] return values.
//!
//! This slice intentionally does not include database persistence, live-daemon or
//! backtester wiring, real broker execution, Position Risk Update Rhai API /
//! persistence projection, or the full live/backtester migration.
//! The old `engine` crate remains only donor material for strategy-handling
//! behavior; this crate must stay independent from it.

pub mod anchored;
pub mod decision;
pub mod events;
pub mod execution;
pub mod market_input;
pub mod market_state;
pub mod portfolio;
pub mod rhai_strategy;
pub mod risk_exit;
pub mod runtime;
mod secondary_context;
pub mod step;
pub mod strategy;
pub mod strategy_config;
pub mod warmup;

pub use anchored::{
    AnchoredConfiguration, AnchoredConfigurationError, AnchoredDetectorSpec,
    AnchoredEvaluatorConfiguration, AnchoredEvaluatorSpec, AnchoredOutput, AnchoredOutputs,
    AnchoredRuntime, PivotDetectorConfiguration, PivotEvent, PivotSide, StructureConfiguration,
    StructureConfigurationError, StructureObjectConfiguration, StructureObjectRegistry,
    StructurePointRegistry, StructurePointSource,
};
pub use decision::{
    validate_opening_quantity, InvalidOpeningQuantity, PositionRiskBoundaryChanges,
    RiskBoundaryChange, StrategyDecision, StrategyDecisionIntent,
};
pub use domain::{ClosedPosition, Timeframe, TimeframeParseError, TimeframeUnit};
pub use events::{
    AppliedPositionRiskBoundaryChange, BlockedSecondaryContext, ExitKind, ForceCloseIgnoredReason,
    PositionRiskBoundaryChangeRejectionReason, PositionRiskBoundaryKind, PositionRiskUpdateResult,
    RejectedPositionRiskBoundaryChange, RuntimeEvent, SecondaryContextUnavailableReason,
};
pub use execution::{plan_execution, ExecutionAction, IgnoredDecisionReason, PlannedExecution};
pub use market_input::{
    MarketInput, RuntimeConfig, RuntimeInputError, SecondaryReadiness, SecondaryTimeframeConfig,
};
pub use market_state::MarketState;
pub use portfolio::{PortfolioState, PortfolioTransitionError, RuntimePortfolioSnapshot};
pub use rhai_strategy::{RhaiStrategy, RhaiStrategyHooks, RhaiStrategyLoadError};
pub use risk_exit::{evaluate_risk_exit, RiskExitKind, RiskExitTriggered};
pub use runtime::TradingRuntime;
pub use step::RuntimeStep;
pub use strategy::{
    MarketView, PredeterminedStrategyHandler, StrategyContext, StrategyError, StrategyHandler,
    StrategyState, StrategyStateValue, StrategyTickInput, StrategyTickResult,
};
pub use strategy_config::{StrategyConfiguration, StrategyConfigurationError};
pub use warmup::{detect_auto_warmup, resolve_effective_warmup, resolve_warmup_plan, WarmupPlan};
