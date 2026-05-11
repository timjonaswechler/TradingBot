use serde::{Deserialize, Serialize};

/// Whether the position bets on price going up or down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PositionSide {
    Long,
    Short,
}

impl std::fmt::Display for PositionSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PositionSide::Long => write!(f, "long"),
            PositionSide::Short => write!(f, "short"),
        }
    }
}

/// An open trading position (paper or live).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,
    /// Price at which the position was entered.
    pub entry_price: f64,
    /// Number of shares / contracts / coin units held.
    pub size: f64,
    /// Unix ms timestamp of entry.
    pub entry_time: i64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

impl Position {
    /// Unrealised PnL given a current market price.
    ///
    /// - Long:  `(current_price - entry_price) * size`
    /// - Short: `(entry_price - current_price) * size`
    pub fn unrealised_pnl(&self, current_price: f64) -> f64 {
        match self.side {
            PositionSide::Long => (current_price - self.entry_price) * self.size,
            PositionSide::Short => (self.entry_price - current_price) * self.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_pos() -> Position {
        Position {
            symbol: "AAPL".into(),
            side: PositionSide::Long,
            entry_price: 100.0,
            size: 10.0,
            entry_time: 0,
            stop_loss: None,
            take_profit: None,
        }
    }

    #[test]
    fn long_pnl_positive_when_price_rises() {
        assert_eq!(long_pos().unrealised_pnl(110.0), 100.0);
    }

    #[test]
    fn long_pnl_negative_when_price_falls() {
        assert_eq!(long_pos().unrealised_pnl(90.0), -100.0);
    }

    #[test]
    fn short_pnl_positive_when_price_falls() {
        let pos = Position {
            side: PositionSide::Short,
            ..long_pos()
        };
        assert_eq!(pos.unrealised_pnl(90.0), 100.0);
    }
}
