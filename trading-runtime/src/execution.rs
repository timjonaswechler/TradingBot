//! Pure planning from strategy decisions to runtime execution actions.

use crate::{StrategyDecision, StrategyDecisionIntent};
use shared::PositionSide;

/// Runtime interpretation of a strategy decision or explicit runner command.
#[derive(Debug, Clone, PartialEq)]
pub enum ExecutionAction {
    Noop,
    OpenLong {
        quantity: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    },
    CloseLong,
    OpenShort {
        quantity: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    },
    CloseShort,
    ForceClose,
}

/// Stable reason why a strategy decision did not produce a portfolio action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IgnoredDecisionReason {
    InvalidQuantity,
    NoMatchingLongPosition,
    NoMatchingShortPosition,
    PositionAlreadyOpen,
}

/// Result of pure execution planning.
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedExecution {
    pub action: ExecutionAction,
    pub ignored_reason: Option<IgnoredDecisionReason>,
}

impl PlannedExecution {
    pub fn action(action: ExecutionAction) -> Self {
        Self {
            action,
            ignored_reason: None,
        }
    }

    pub fn noop() -> Self {
        Self::action(ExecutionAction::Noop)
    }

    pub fn ignored(reason: IgnoredDecisionReason) -> Self {
        Self {
            action: ExecutionAction::Noop,
            ignored_reason: Some(reason),
        }
    }
}

/// Plan a strategy decision against the currently open position side.
///
/// This function is pure: it does not inspect or mutate portfolio state beyond
/// the provided side classification.
pub fn plan_execution(
    decision: &StrategyDecision,
    current_side: Option<PositionSide>,
) -> PlannedExecution {
    match decision.intent {
        StrategyDecisionIntent::Hold => PlannedExecution::noop(),
        StrategyDecisionIntent::OpenLong => plan_open_long(decision, current_side),
        StrategyDecisionIntent::CloseLong => match current_side {
            Some(PositionSide::Long) => PlannedExecution::action(ExecutionAction::CloseLong),
            _ => PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingLongPosition),
        },
        StrategyDecisionIntent::OpenShort => plan_open_short(decision, current_side),
        StrategyDecisionIntent::CloseShort => match current_side {
            Some(PositionSide::Short) => PlannedExecution::action(ExecutionAction::CloseShort),
            _ => PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingShortPosition),
        },
    }
}

fn plan_open_long(
    decision: &StrategyDecision,
    current_side: Option<PositionSide>,
) -> PlannedExecution {
    if current_side.is_some() {
        return PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen);
    }

    match decision.validated_opening_quantity() {
        Ok(Some(quantity)) => PlannedExecution::action(ExecutionAction::OpenLong {
            quantity,
            stop_loss: decision.stop_loss,
            take_profit: decision.take_profit,
        }),
        Ok(None) => PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
        Err(_) => PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
    }
}

fn plan_open_short(
    decision: &StrategyDecision,
    current_side: Option<PositionSide>,
) -> PlannedExecution {
    if current_side.is_some() {
        return PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen);
    }

    match decision.validated_opening_quantity() {
        Ok(Some(quantity)) => PlannedExecution::action(ExecutionAction::OpenShort {
            quantity,
            stop_loss: decision.stop_loss,
            take_profit: decision.take_profit,
        }),
        Ok(None) => PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
        Err(_) => PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_plan(
        decision: StrategyDecision,
        current_side: Option<PositionSide>,
        expected: PlannedExecution,
    ) {
        assert_eq!(plan_execution(&decision, current_side), expected);
    }

    #[test]
    fn hold_plans_noop_without_ignored_reason() {
        assert_plan(StrategyDecision::hold(), None, PlannedExecution::noop());
        assert_plan(
            StrategyDecision::hold(),
            Some(PositionSide::Long),
            PlannedExecution::noop(),
        );
    }

    #[test]
    fn valid_open_long_from_flat_plans_open_long() {
        assert_plan(
            StrategyDecision::open_long(2.0).with_entry_risk(Some(90.0), Some(120.0)),
            None,
            PlannedExecution::action(ExecutionAction::OpenLong {
                quantity: 2.0,
                stop_loss: Some(90.0),
                take_profit: Some(120.0),
            }),
        );
    }

    #[test]
    fn valid_open_short_from_flat_plans_open_short() {
        assert_plan(
            StrategyDecision::open_short(3.0).with_entry_risk(Some(110.0), Some(80.0)),
            None,
            PlannedExecution::action(ExecutionAction::OpenShort {
                quantity: 3.0,
                stop_loss: Some(110.0),
                take_profit: Some(80.0),
            }),
        );
    }

    #[test]
    fn matching_close_decisions_plan_closes() {
        assert_plan(
            StrategyDecision::close_long(),
            Some(PositionSide::Long),
            PlannedExecution::action(ExecutionAction::CloseLong),
        );
        assert_plan(
            StrategyDecision::close_short(),
            Some(PositionSide::Short),
            PlannedExecution::action(ExecutionAction::CloseShort),
        );
    }

    #[test]
    fn duplicate_open_decisions_are_ignored() {
        assert_plan(
            StrategyDecision::open_long(2.0),
            Some(PositionSide::Long),
            PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen),
        );
        assert_plan(
            StrategyDecision::open_short(2.0),
            Some(PositionSide::Short),
            PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen),
        );
    }

    #[test]
    fn opposite_side_open_decisions_are_ignored_while_in_position() {
        assert_plan(
            StrategyDecision::open_short(2.0),
            Some(PositionSide::Long),
            PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen),
        );
        assert_plan(
            StrategyDecision::open_long(2.0),
            Some(PositionSide::Short),
            PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen),
        );
    }

    #[test]
    fn mismatched_close_decisions_are_ignored() {
        assert_plan(
            StrategyDecision::close_long(),
            None,
            PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingLongPosition),
        );
        assert_plan(
            StrategyDecision::close_long(),
            Some(PositionSide::Short),
            PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingLongPosition),
        );
        assert_plan(
            StrategyDecision::close_short(),
            None,
            PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingShortPosition),
        );
        assert_plan(
            StrategyDecision::close_short(),
            Some(PositionSide::Long),
            PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingShortPosition),
        );
    }

    #[test]
    fn invalid_opening_quantities_are_ignored() {
        for decision in [
            StrategyDecision::new(StrategyDecisionIntent::OpenLong),
            StrategyDecision::open_long(0.0),
            StrategyDecision::open_long(-1.0),
            StrategyDecision::open_long(f64::NAN),
            StrategyDecision::new(StrategyDecisionIntent::OpenShort),
            StrategyDecision::open_short(0.0),
            StrategyDecision::open_short(-1.0),
            StrategyDecision::open_short(f64::INFINITY),
        ] {
            assert_plan(
                decision,
                None,
                PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
            );
        }
    }
}
