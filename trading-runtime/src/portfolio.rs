//! Runtime-local portfolio state and snapshots.

use shared::{realized_pnl, Candle, Position, PositionSide};

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

    pub fn open_long_from_flat(
        &mut self,
        candle: &Candle,
        quantity: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<(), PortfolioTransitionError> {
        self.open_from_flat(PositionSide::Long, candle, quantity, stop_loss, take_profit)
    }

    pub fn close_long(
        &mut self,
        candle: &Candle,
    ) -> Result<ClosedPosition, PortfolioTransitionError> {
        self.close_long_at_price(candle, candle.close)
    }

    pub fn close_long_at_price(
        &mut self,
        candle: &Candle,
        exit_price: f64,
    ) -> Result<ClosedPosition, PortfolioTransitionError> {
        self.close_matching_side(PositionSide::Long, candle, exit_price)
    }

    pub fn open_short_from_flat(
        &mut self,
        candle: &Candle,
        quantity: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<(), PortfolioTransitionError> {
        self.open_from_flat(
            PositionSide::Short,
            candle,
            quantity,
            stop_loss,
            take_profit,
        )
    }

    pub fn close_short(
        &mut self,
        candle: &Candle,
    ) -> Result<ClosedPosition, PortfolioTransitionError> {
        self.close_short_at_price(candle, candle.close)
    }

    pub fn close_short_at_price(
        &mut self,
        candle: &Candle,
        exit_price: f64,
    ) -> Result<ClosedPosition, PortfolioTransitionError> {
        self.close_matching_side(PositionSide::Short, candle, exit_price)
    }

    fn open_from_flat(
        &mut self,
        side: PositionSide,
        candle: &Candle,
        quantity: f64,
        stop_loss: Option<f64>,
        take_profit: Option<f64>,
    ) -> Result<(), PortfolioTransitionError> {
        if self.open_position.is_some() {
            return Err(PortfolioTransitionError::PositionAlreadyOpen);
        }

        self.open_position = Some(Position {
            symbol: candle.symbol.clone(),
            side,
            entry_price: candle.close,
            size: quantity,
            entry_time: candle.timestamp,
            stop_loss,
            take_profit,
        });

        Ok(())
    }

    fn close_matching_side(
        &mut self,
        expected_side: PositionSide,
        candle: &Candle,
        exit_price: f64,
    ) -> Result<ClosedPosition, PortfolioTransitionError> {
        let position = self
            .open_position
            .take()
            .ok_or(PortfolioTransitionError::NoOpenPosition)?;

        if position.side != expected_side {
            self.open_position = Some(position);
            return Err(PortfolioTransitionError::PositionSideMismatch);
        }

        let pnl = realized_pnl(
            position.side,
            position.entry_price,
            exit_price,
            position.size,
        );
        self.realized_cash_balance += pnl;
        self.completed_trade_count += 1;

        Ok(ClosedPosition {
            position,
            exit_price,
            exit_time: candle.timestamp,
            realized_pnl: pnl,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClosedPosition {
    pub position: Position,
    pub exit_price: f64,
    pub exit_time: i64,
    pub realized_pnl: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortfolioTransitionError {
    PositionAlreadyOpen,
    NoOpenPosition,
    PositionSideMismatch,
}

/// Point-in-time portfolio view returned by runtime steps.
#[derive(Debug, Clone, PartialEq)]
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

    fn candle(timestamp: i64, close: f64) -> shared::Candle {
        shared::Candle {
            timestamp,
            symbol: "BTC-USD".into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            timeframe: "1m".into(),
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

    #[test]
    fn opening_long_from_flat_creates_position_without_reducing_realized_cash() {
        let mut state = PortfolioState::new(1_000.0);
        let candle = candle(1, 100.0);

        state
            .open_long_from_flat(&candle, 2.0, Some(90.0), Some(120.0))
            .unwrap();

        let position = state.open_position.as_ref().unwrap();
        assert_eq!(state.realized_cash_balance, 1_000.0);
        assert_eq!(state.completed_trade_count, 0);
        assert_eq!(position.symbol, "BTC-USD");
        assert_eq!(position.side, PositionSide::Long);
        assert_eq!(position.entry_price, 100.0);
        assert_eq!(position.size, 2.0);
        assert_eq!(position.entry_time, 1);
        assert_eq!(position.stop_loss, Some(90.0));
        assert_eq!(position.take_profit, Some(120.0));
    }

    #[test]
    fn closing_long_applies_realized_pnl_and_increments_completed_trade_count() {
        let mut state = PortfolioState::new(1_000.0);
        state
            .open_long_from_flat(&candle(1, 100.0), 2.0, None, None)
            .unwrap();

        let closed = state.close_long(&candle(2, 115.0)).unwrap();

        assert!(state.open_position.is_none());
        assert_eq!(state.realized_cash_balance, 1_030.0);
        assert_eq!(state.completed_trade_count, 1);
        assert_eq!(closed.position.side, PositionSide::Long);
        assert_eq!(closed.exit_price, 115.0);
        assert_eq!(closed.exit_time, 2);
        assert_eq!(closed.realized_pnl, 30.0);
    }

    #[test]
    fn closing_long_at_explicit_exit_price_uses_that_price_for_pnl_and_snapshot() {
        let mut state = PortfolioState::new(1_000.0);
        state
            .open_long_from_flat(&candle(1, 100.0), 2.0, None, None)
            .unwrap();
        let exit_candle = candle(2, 115.0);

        let closed = state.close_long_at_price(&exit_candle, 90.0).unwrap();
        let snapshot = state.snapshot(exit_candle.close);

        assert!(state.open_position.is_none());
        assert_eq!(state.realized_cash_balance, 980.0);
        assert_eq!(state.completed_trade_count, 1);
        assert_eq!(closed.position.side, PositionSide::Long);
        assert_eq!(closed.exit_price, 90.0);
        assert_eq!(closed.exit_time, exit_candle.timestamp);
        assert_eq!(closed.realized_pnl, -20.0);
        assert!(snapshot.open_position.is_none());
        assert_eq!(snapshot.current_equity, snapshot.realized_cash_balance);
    }

    #[test]
    fn opening_short_from_flat_creates_position_without_reducing_realized_cash() {
        let mut state = PortfolioState::new(1_000.0);

        state
            .open_short_from_flat(&candle(1, 100.0), 2.0, Some(110.0), Some(80.0))
            .unwrap();

        let position = state.open_position.as_ref().unwrap();
        assert_eq!(state.realized_cash_balance, 1_000.0);
        assert_eq!(state.completed_trade_count, 0);
        assert_eq!(position.side, PositionSide::Short);
        assert_eq!(position.entry_price, 100.0);
        assert_eq!(position.size, 2.0);
        assert_eq!(position.stop_loss, Some(110.0));
        assert_eq!(position.take_profit, Some(80.0));
    }

    #[test]
    fn closing_short_applies_realized_pnl_and_increments_completed_trade_count() {
        let mut state = PortfolioState::new(1_000.0);
        state
            .open_short_from_flat(&candle(1, 100.0), 2.0, None, None)
            .unwrap();

        let closed = state.close_short(&candle(2, 85.0)).unwrap();

        assert!(state.open_position.is_none());
        assert_eq!(state.realized_cash_balance, 1_030.0);
        assert_eq!(state.completed_trade_count, 1);
        assert_eq!(closed.position.side, PositionSide::Short);
        assert_eq!(closed.exit_price, 85.0);
        assert_eq!(closed.exit_time, 2);
        assert_eq!(closed.realized_pnl, 30.0);
    }

    #[test]
    fn closing_short_at_explicit_exit_price_uses_that_price_for_pnl_and_snapshot() {
        let mut state = PortfolioState::new(1_000.0);
        state
            .open_short_from_flat(&candle(1, 100.0), 2.0, None, None)
            .unwrap();
        let exit_candle = candle(2, 85.0);

        let closed = state.close_short_at_price(&exit_candle, 110.0).unwrap();
        let snapshot = state.snapshot(exit_candle.close);

        assert!(state.open_position.is_none());
        assert_eq!(state.realized_cash_balance, 980.0);
        assert_eq!(state.completed_trade_count, 1);
        assert_eq!(closed.position.side, PositionSide::Short);
        assert_eq!(closed.exit_price, 110.0);
        assert_eq!(closed.exit_time, exit_candle.timestamp);
        assert_eq!(closed.realized_pnl, -20.0);
        assert!(snapshot.open_position.is_none());
        assert_eq!(snapshot.current_equity, snapshot.realized_cash_balance);
    }
}
