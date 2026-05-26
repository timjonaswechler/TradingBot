//! Ordered, runner-neutral events emitted by the trading runtime.

use crate::{
    ClosedPosition, ExecutionAction, IgnoredDecisionReason, RiskExitKind, RuntimePortfolioSnapshot,
    StrategyDecision,
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
        current_primary_candle_count: usize,
        required_warmup_candles: usize,
    },
    WarmupCompleted {
        completed_primary_candle_count: usize,
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
