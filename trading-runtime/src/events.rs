//! Ordered, runner-neutral events emitted by the trading runtime.

use crate::{
    ClosedPosition, ExecutionAction, IgnoredDecisionReason, RiskExitKind, RiskExitTriggered,
    RuntimePortfolioSnapshot, SecondaryReadiness, StrategyDecision, StrategyError,
};
use shared::{Candle, Position, Timeframe};

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

/// Why Secondary-Timeframe context is unavailable for a Primary Strategy Tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryContextUnavailableReason {
    Missing,
    Stale,
}

/// Required Secondary-Timeframe context that blocked a Primary Strategy Tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockedSecondaryContext {
    pub timeframe: Timeframe,
    pub reason: SecondaryContextUnavailableReason,
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
    TradableCandleAccepted {
        candle: Candle,
    },
    StrategyTickStarted {
        candle: Candle,
    },
    StrategyTickBlocked {
        candle: Candle,
        blocked_contexts: Vec<BlockedSecondaryContext>,
    },
    SecondaryContextUnavailable {
        candle: Candle,
        timeframe: Timeframe,
        readiness: SecondaryReadiness,
        reason: SecondaryContextUnavailableReason,
    },
    StrategyDecisionProduced {
        decision: StrategyDecision,
    },
    StrategyError {
        candle: Candle,
        error: StrategyError,
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
    StrategyTickCompleted,
    TradableCandleCompleted,
    WarmupAdvanced {
        timeframe: Timeframe,
        current_warmup_input_count: usize,
        required_warmup_inputs: usize,
    },
    WarmupCompleted {
        completed_timeframes: Vec<Timeframe>,
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
