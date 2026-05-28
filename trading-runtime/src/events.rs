//! Ordered, runner-neutral events emitted by the trading runtime.

use crate::{
    ClosedPosition, ExecutionAction, IgnoredDecisionReason, RiskExitKind, RiskExitTriggered,
    RuntimePortfolioSnapshot, StrategyDecision,
};
use shared::{Candle, Position};

/// Why an explicit runner force-close command did not close a position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceCloseIgnoredReason {
    NoOpenPosition,
}

/// Machine-readable category for a position-closing portfolio transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitKind {
    StrategyExit,
    RiskExit { selected: RiskExitKind },
    ForceClose,
}

/// A runner-neutral occurrence emitted by the trading runtime.
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeEvent {
    MarketInputAccepted {
        candle: Candle,
    },
    WarmupInputAccepted {
        candle: Candle,
    },
    TradableTickStarted {
        candle: Candle,
    },
    StrategyDecisionProduced {
        decision: StrategyDecision,
    },
    ExecutionActionPlanned {
        action: ExecutionAction,
    },
    StrategyDecisionIgnored {
        decision: StrategyDecision,
        reason: IgnoredDecisionReason,
    },
    RiskExitTriggered {
        risk_exit: RiskExitTriggered,
    },
    PositionOpened {
        position: Position,
    },
    PositionClosed {
        closed_position: ClosedPosition,
        exit_kind: ExitKind,
    },
    PortfolioUpdated {
        snapshot: RuntimePortfolioSnapshot,
    },
    TradableTickCompleted,
    WarmupAdvanced {
        timeframe: String,
        current_warmup_input_count: usize,
        required_warmup_inputs: usize,
    },
    WarmupCompleted {
        completed_timeframes: Vec<String>,
        required_warmup_inputs: usize,
    },
    ForceCloseRequested {
        candle: Candle,
        reason: String,
    },
    ForceCloseIgnored {
        reason: ForceCloseIgnoredReason,
    },
    ForceCloseCompleted,
}
