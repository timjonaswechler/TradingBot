use serde::{Deserialize, Serialize};

/// Trading signal emitted by a Rhai strategy's `on_tick`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Signal {
    Buy,
    Sell,
    Hold,
    /// Open a short position.
    Short,
    /// Close an existing short position.
    Cover,
}

impl std::fmt::Display for Signal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Signal::Buy   => write!(f, "BUY"),
            Signal::Sell  => write!(f, "SELL"),
            Signal::Hold  => write!(f, "HOLD"),
            Signal::Short => write!(f, "SHORT"),
            Signal::Cover => write!(f, "COVER"),
        }
    }
}

impl std::str::FromStr for Signal {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "BUY"   => Ok(Signal::Buy),
            "SELL"  => Ok(Signal::Sell),
            "HOLD"  => Ok(Signal::Hold),
            "SHORT" => Ok(Signal::Short),
            "COVER" => Ok(Signal::Cover),
            other   => Err(format!("unknown signal: {other}")),
        }
    }
}

/// Full decision returned by a strategy for one candle tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeDecision {
    pub signal: Signal,

    /// Portfolio fraction to deploy (0.0–1.0).  
    /// `paper_trader` converts this to shares: `shares = (balance * size) / entry_price`.  
    /// Defaults to `1.0` (100 % of available capital) when omitted by the strategy.
    pub size: f64,

    /// Optional hard stop-loss price.
    pub stop_loss: Option<f64>,

    /// Optional take-profit price.
    pub take_profit: Option<f64>,

    /// Human-readable reason logged alongside the trade (e.g. `"RSI oversold + EMA cross"`).
    pub reason: Option<String>,
}

impl TradeDecision {
    /// Convenience constructor for a `HOLD` with no extra metadata.
    pub fn hold() -> Self {
        Self {
            signal:      Signal::Hold,
            size:        0.0,
            stop_loss:   None,
            take_profit: None,
            reason:      None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn round_trip_display_parse() {
        for s in [Signal::Buy, Signal::Sell, Signal::Hold, Signal::Short, Signal::Cover] {
            let parsed = Signal::from_str(&s.to_string()).unwrap();
            assert_eq!(parsed, s);
        }
    }

    #[test]
    fn unknown_signal_returns_err() {
        assert!(Signal::from_str("MOON").is_err());
    }

    #[test]
    fn hold_decision_has_zero_size() {
        assert_eq!(TradeDecision::hold().size, 0.0);
    }
}
