//! Runtime-local portfolio state and snapshots.

use shared::Position;

/// Runtime-local portfolio state for one trading session.
///
/// This uses realized-cash semantics: opening a position does not subtract or
/// reserve notional from cash, and equity is derived in snapshots from the
/// current mark price.
#[derive(Debug, Clone)]
pub struct PortfolioState {
    pub realized_cash_balance: f64,
    pub open_position: Option<Position>,
    pub completed_trade_count: usize,
}

impl PortfolioState {
    pub fn new(realized_cash_balance: f64) -> Self {
        Self {
            realized_cash_balance,
            open_position: None,
            completed_trade_count: 0,
        }
    }

    pub fn snapshot(&self, mark_price: f64) -> RuntimePortfolioSnapshot {
        RuntimePortfolioSnapshot::from_state(self, mark_price)
    }
}

/// Point-in-time portfolio view returned by runtime steps.
#[derive(Debug, Clone)]
pub struct RuntimePortfolioSnapshot {
    pub realized_cash_balance: f64,
    pub open_position: Option<Position>,
    pub completed_trade_count: usize,
    pub current_equity: f64,
}

impl RuntimePortfolioSnapshot {
    pub fn from_state(state: &PortfolioState, mark_price: f64) -> Self {
        let unrealized_pnl = state
            .open_position
            .as_ref()
            .map(|position| position.unrealised_pnl(mark_price))
            .unwrap_or(0.0);

        Self {
            realized_cash_balance: state.realized_cash_balance,
            open_position: state.open_position.clone(),
            completed_trade_count: state.completed_trade_count,
            current_equity: state.realized_cash_balance + unrealized_pnl,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::PositionSide;

    fn position(side: PositionSide) -> Position {
        Position {
            symbol: "BTC-USD".into(),
            side,
            entry_price: 100.0,
            size: 2.0,
            entry_time: 1_700_000_000_000,
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn new_portfolio_state_starts_flat_with_realized_cash() {
        let state = PortfolioState::new(1_000.0);

        assert_eq!(state.realized_cash_balance, 1_000.0);
        assert!(state.open_position.is_none());
        assert_eq!(state.completed_trade_count, 0);
    }

    #[test]
    fn snapshot_equity_while_flat_is_realized_cash_balance() {
        let state = PortfolioState::new(1_000.0);

        let snapshot = state.snapshot(120.0);

        assert_eq!(snapshot.realized_cash_balance, 1_000.0);
        assert!(snapshot.open_position.is_none());
        assert_eq!(snapshot.completed_trade_count, 0);
        assert_eq!(snapshot.current_equity, 1_000.0);
    }

    #[test]
    fn snapshot_equity_while_long_includes_unrealized_pnl_at_mark_price() {
        let mut state = PortfolioState::new(1_000.0);
        state.open_position = Some(position(PositionSide::Long));

        let snapshot = state.snapshot(115.0);

        assert_eq!(snapshot.realized_cash_balance, 1_000.0);
        assert_eq!(
            snapshot.open_position.as_ref().map(|p| p.side),
            Some(PositionSide::Long)
        );
        assert_eq!(snapshot.current_equity, 1_030.0);
    }

    #[test]
    fn snapshot_equity_while_short_includes_unrealized_pnl_at_mark_price() {
        let mut state = PortfolioState::new(1_000.0);
        state.open_position = Some(position(PositionSide::Short));

        let snapshot = state.snapshot(85.0);

        assert_eq!(snapshot.realized_cash_balance, 1_000.0);
        assert_eq!(
            snapshot.open_position.as_ref().map(|p| p.side),
            Some(PositionSide::Short)
        );
        assert_eq!(snapshot.current_equity, 1_030.0);
    }
}
