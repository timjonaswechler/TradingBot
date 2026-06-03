use crate::Position;
use serde::{Deserialize, Serialize};

/// Runtime context passed to the Rhai `on_tick` function on every candle.
///
/// Gives the strategy read-only visibility into the current portfolio state
/// so it can make informed decisions (e.g. skip BUY when already in a position).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Context {
    /// Cash available for new trades.
    pub balance: f64,

    /// `balance + value of open position at current market price`.
    pub equity: f64,

    /// Currently open position, or `None` if flat.
    pub position: Option<Position>,

    /// Total number of completed trades so far in this session.
    pub trades_count: u32,
}

impl Context {
    /// Create a fresh context with the given starting capital and no open position.
    pub fn new(initial_balance: f64) -> Self {
        Self {
            balance: initial_balance,
            equity: initial_balance,
            position: None,
            trades_count: 0,
        }
    }

    /// `true` when there is an active open position.
    pub fn has_position(&self) -> bool {
        self.position.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_context_is_flat() {
        let ctx = Context::new(10_000.0);
        assert!(!ctx.has_position());
        assert_eq!(ctx.balance, 10_000.0);
        assert_eq!(ctx.equity, 10_000.0);
        assert_eq!(ctx.trades_count, 0);
    }
}
