//! Side-aware mapping from a strategy `Signal` + current position to the
//! concrete action an executor should take.
//!
//! Both `PaperExecutor` (live daemon) and `InMemoryExecutor` (backtester)
//! call this so the two implementations cannot drift.
use crate::{PositionSide, Signal};

/// Concrete action derived from a strategy signal and the current position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open a new long position.
    OpenLong,
    /// Open a new short position.
    OpenShort,
    /// Close whatever position is currently open.
    Close,
    /// No-op — HOLD, or a signal that does not match the current position.
    Nothing,
}

/// Decide what to do for `signal` given the current open side (`None` = flat).
///
/// - `(Buy,   None)` → `OpenLong`
/// - `(Short, None)` → `OpenShort`
/// - `(Sell,  Some(Long))`  → `Close`
/// - `(Cover, Some(Short))` → `Close`
/// - Everything else (HOLD, SELL while flat, BUY while short, …) → `Nothing`.
pub fn plan_action(signal: &Signal, current_side: Option<PositionSide>) -> Action {
    match (signal, current_side) {
        (Signal::Buy, None) => Action::OpenLong,
        (Signal::Short, None) => Action::OpenShort,
        (Signal::Sell, Some(PositionSide::Long)) => Action::Close,
        (Signal::Cover, Some(PositionSide::Short)) => Action::Close,
        _ => Action::Nothing,
    }
}

/// Realised PnL for a closed position.
///
/// - Long:  `(exit - entry) * size`
/// - Short: `(entry - exit) * size`
pub fn realized_pnl(side: PositionSide, entry: f64, exit: f64, size: f64) -> f64 {
    match side {
        PositionSide::Long => (exit - entry) * size,
        PositionSide::Short => (entry - exit) * size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_from_flat_opens_long() {
        assert_eq!(plan_action(&Signal::Buy, None), Action::OpenLong);
    }

    #[test]
    fn short_from_flat_opens_short() {
        assert_eq!(plan_action(&Signal::Short, None), Action::OpenShort);
    }

    #[test]
    fn sell_while_long_closes() {
        assert_eq!(
            plan_action(&Signal::Sell, Some(PositionSide::Long)),
            Action::Close
        );
    }

    #[test]
    fn cover_while_short_closes() {
        assert_eq!(
            plan_action(&Signal::Cover, Some(PositionSide::Short)),
            Action::Close
        );
    }

    #[test]
    fn mismatched_signals_do_nothing() {
        // BUY while already long, SELL while flat, COVER while long, etc.
        assert_eq!(
            plan_action(&Signal::Buy, Some(PositionSide::Long)),
            Action::Nothing
        );
        assert_eq!(
            plan_action(&Signal::Buy, Some(PositionSide::Short)),
            Action::Nothing
        );
        assert_eq!(plan_action(&Signal::Sell, None), Action::Nothing);
        assert_eq!(
            plan_action(&Signal::Sell, Some(PositionSide::Short)),
            Action::Nothing
        );
        assert_eq!(
            plan_action(&Signal::Short, Some(PositionSide::Long)),
            Action::Nothing
        );
        assert_eq!(plan_action(&Signal::Cover, None), Action::Nothing);
        assert_eq!(
            plan_action(&Signal::Cover, Some(PositionSide::Long)),
            Action::Nothing
        );
        assert_eq!(plan_action(&Signal::Hold, None), Action::Nothing);
        assert_eq!(
            plan_action(&Signal::Hold, Some(PositionSide::Long)),
            Action::Nothing
        );
    }

    #[test]
    fn long_pnl_signs() {
        assert_eq!(realized_pnl(PositionSide::Long, 100.0, 110.0, 2.0), 20.0);
        assert_eq!(realized_pnl(PositionSide::Long, 100.0, 90.0, 2.0), -20.0);
    }

    #[test]
    fn short_pnl_signs() {
        assert_eq!(realized_pnl(PositionSide::Short, 100.0, 90.0, 2.0), 20.0);
        assert_eq!(realized_pnl(PositionSide::Short, 100.0, 110.0, 2.0), -20.0);
    }
}
