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
    InvalidEntryRisk,
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
    entry_price: f64,
) -> PlannedExecution {
    match decision.intent {
        StrategyDecisionIntent::Hold => PlannedExecution::noop(),
        StrategyDecisionIntent::OpenLong => {
            plan_open(decision, current_side, PositionSide::Long, entry_price)
        }
        StrategyDecisionIntent::CloseLong => match current_side {
            Some(PositionSide::Long) => PlannedExecution::action(ExecutionAction::CloseLong),
            _ => PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingLongPosition),
        },
        StrategyDecisionIntent::OpenShort => {
            plan_open(decision, current_side, PositionSide::Short, entry_price)
        }
        StrategyDecisionIntent::CloseShort => match current_side {
            Some(PositionSide::Short) => PlannedExecution::action(ExecutionAction::CloseShort),
            _ => PlannedExecution::ignored(IgnoredDecisionReason::NoMatchingShortPosition),
        },
    }
}

fn plan_open(
    decision: &StrategyDecision,
    current_side: Option<PositionSide>,
    opening_side: PositionSide,
    entry_price: f64,
) -> PlannedExecution {
    if current_side.is_some() {
        return PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen);
    }

    let quantity = match decision.validated_opening_quantity() {
        Ok(Some(quantity)) => quantity,
        Ok(None) | Err(_) => {
            return PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity)
        }
    };

    if !entry_risk_is_valid(
        opening_side,
        entry_price,
        decision.stop_loss,
        decision.take_profit,
    ) {
        return PlannedExecution::ignored(IgnoredDecisionReason::InvalidEntryRisk);
    }

    match opening_side {
        PositionSide::Long => PlannedExecution::action(ExecutionAction::OpenLong {
            quantity,
            stop_loss: decision.stop_loss,
            take_profit: decision.take_profit,
        }),
        PositionSide::Short => PlannedExecution::action(ExecutionAction::OpenShort {
            quantity,
            stop_loss: decision.stop_loss,
            take_profit: decision.take_profit,
        }),
    }
}

fn entry_risk_is_valid(
    side: PositionSide,
    entry_price: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> bool {
    let stop_loss_is_valid = match stop_loss {
        Some(stop_loss) => {
            risk_price_is_valid(stop_loss)
                && stop_loss_is_on_correct_side(side, stop_loss, entry_price)
        }
        None => true,
    };
    let take_profit_is_valid = match take_profit {
        Some(take_profit) => {
            risk_price_is_valid(take_profit)
                && take_profit_is_on_correct_side(side, take_profit, entry_price)
        }
        None => true,
    };

    stop_loss_is_valid && take_profit_is_valid
}

fn risk_price_is_valid(price: f64) -> bool {
    price.is_finite() && price > 0.0
}

fn stop_loss_is_on_correct_side(side: PositionSide, stop_loss: f64, entry_price: f64) -> bool {
    match side {
        PositionSide::Long => stop_loss < entry_price,
        PositionSide::Short => stop_loss > entry_price,
    }
}

fn take_profit_is_on_correct_side(side: PositionSide, take_profit: f64, entry_price: f64) -> bool {
    match side {
        PositionSide::Long => take_profit > entry_price,
        PositionSide::Short => take_profit < entry_price,
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
        assert_eq!(plan_execution(&decision, current_side, 100.0), expected);
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

    #[test]
    fn invalid_entry_risk_is_ignored_after_quantity_validation() {
        for decision in [
            StrategyDecision::open_long(2.0).with_entry_risk(Some(f64::NAN), None),
            StrategyDecision::open_long(2.0).with_entry_risk(Some(f64::INFINITY), None),
            StrategyDecision::open_long(2.0).with_entry_risk(Some(0.0), None),
            StrategyDecision::open_long(2.0).with_entry_risk(Some(-1.0), None),
            StrategyDecision::open_long(2.0).with_entry_risk(Some(100.0), None),
            StrategyDecision::open_long(2.0).with_entry_risk(Some(101.0), None),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(f64::NAN)),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(f64::INFINITY)),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(0.0)),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(-1.0)),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(100.0)),
            StrategyDecision::open_long(2.0).with_entry_risk(None, Some(99.0)),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(f64::NAN), None),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(f64::INFINITY), None),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(0.0), None),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(-1.0), None),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(100.0), None),
            StrategyDecision::open_short(2.0).with_entry_risk(Some(99.0), None),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(f64::NAN)),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(f64::INFINITY)),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(0.0)),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(-1.0)),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(100.0)),
            StrategyDecision::open_short(2.0).with_entry_risk(None, Some(101.0)),
        ] {
            assert_eq!(
                plan_execution(&decision, None, 100.0),
                PlannedExecution::ignored(IgnoredDecisionReason::InvalidEntryRisk),
            );
        }
    }

    #[test]
    fn opening_validation_priority_is_state_then_quantity_then_entry_risk() {
        assert_eq!(
            plan_execution(
                &StrategyDecision::open_long(0.0).with_entry_risk(Some(100.0), None),
                Some(PositionSide::Short),
                100.0,
            ),
            PlannedExecution::ignored(IgnoredDecisionReason::PositionAlreadyOpen),
        );
        assert_eq!(
            plan_execution(
                &StrategyDecision::open_long(0.0).with_entry_risk(Some(100.0), None),
                None,
                100.0,
            ),
            PlannedExecution::ignored(IgnoredDecisionReason::InvalidQuantity),
        );
    }
}
