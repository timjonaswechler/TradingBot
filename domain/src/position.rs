use serde::{Deserialize, Serialize};

/// Whether an open position has long or short market exposure.
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

/// Current runtime-managed risk boundaries attached to an open position.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct PositionRiskBoundaries {
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

/// Passive value describing active market exposure.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpenPosition {
    pub symbol: String,
    pub side: PositionSide,
    /// Price at which the position was entered.
    pub entry_price: f64,
    /// Asset units / contracts / coin units held.
    pub quantity: f64,
    /// Unix ms timestamp of entry.
    pub entry_time: i64,
    pub risk_boundaries: PositionRiskBoundaries,
}

/// Passive result value for an open position that has been closed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedPosition {
    pub position: OpenPosition,
    pub exit_price: f64,
    pub exit_time: i64,
    pub realized_pnl: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_and_closed_positions_are_passive_quantity_values() {
        let position = OpenPosition {
            symbol: "AAPL".into(),
            side: PositionSide::Long,
            entry_price: 100.0,
            quantity: 10.0,
            entry_time: 0,
            risk_boundaries: PositionRiskBoundaries {
                stop_loss: Some(90.0),
                take_profit: Some(120.0),
            },
        };

        let closed = ClosedPosition {
            position: position.clone(),
            exit_price: 110.0,
            exit_time: 60_000,
            realized_pnl: 100.0,
        };

        assert_eq!(position.quantity, 10.0);
        assert_eq!(position.risk_boundaries.stop_loss, Some(90.0));
        assert_eq!(closed.position, position);
        assert_eq!(closed.realized_pnl, 100.0);
    }

    #[test]
    fn open_position_serializes_current_risk_boundaries_language() {
        let position = OpenPosition {
            symbol: "AAPL".into(),
            side: PositionSide::Long,
            entry_price: 100.0,
            quantity: 10.0,
            entry_time: 0,
            risk_boundaries: PositionRiskBoundaries {
                stop_loss: Some(90.0),
                take_profit: None,
            },
        };

        let value = serde_json::to_value(&position).expect("position should serialize");

        assert!(value.get("risk_boundaries").is_some());
        assert!(value.get("entry_risk").is_none());
    }

    #[test]
    fn position_side_formats_as_passive_side_language() {
        assert_eq!(PositionSide::Long.to_string(), "long");
        assert_eq!(PositionSide::Short.to_string(), "short");
    }
}
