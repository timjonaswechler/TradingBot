//! Strategy-produced decisions for a tradable runtime tick.

/// Direction-aware strategy intent for one tradable tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyDecisionIntent {
    Hold,
    OpenLong,
    CloseLong,
    OpenShort,
    CloseShort,
}

impl StrategyDecisionIntent {
    /// Whether this intent opens a new position and therefore requires a valid
    /// asset-unit quantity.
    pub fn opens_position(self) -> bool {
        matches!(self, Self::OpenLong | Self::OpenShort)
    }
}

/// Why an opening decision's quantity cannot be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidOpeningQuantity {
    Missing,
    NonFinite,
    Zero,
    Negative,
}

/// The strategy-produced decision before the runtime plans execution.
#[derive(Debug, Clone, PartialEq)]
pub struct StrategyDecision {
    pub intent: StrategyDecisionIntent,
    pub reason: Option<String>,
    /// Quantity in asset units/contracts for opening decisions.
    ///
    /// This is not a balance fraction or notional percentage.
    pub quantity: Option<f64>,
    /// Entry stop-loss price for opening decisions only.
    ///
    /// This does not represent a dynamic risk update for an already-open
    /// position.
    pub stop_loss: Option<f64>,
    /// Entry take-profit price for opening decisions only.
    ///
    /// This does not represent a dynamic risk update for an already-open
    /// position.
    pub take_profit: Option<f64>,
}

impl StrategyDecision {
    pub fn new(intent: StrategyDecisionIntent) -> Self {
        Self {
            intent,
            reason: None,
            quantity: None,
            stop_loss: None,
            take_profit: None,
        }
    }

    pub fn hold() -> Self {
        Self::new(StrategyDecisionIntent::Hold)
    }

    pub fn open_long(quantity: f64) -> Self {
        Self::new(StrategyDecisionIntent::OpenLong).with_quantity(quantity)
    }

    pub fn close_long() -> Self {
        Self::new(StrategyDecisionIntent::CloseLong)
    }

    pub fn open_short(quantity: f64) -> Self {
        Self::new(StrategyDecisionIntent::OpenShort).with_quantity(quantity)
    }

    pub fn close_short() -> Self {
        Self::new(StrategyDecisionIntent::CloseShort)
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_quantity(mut self, quantity: f64) -> Self {
        self.quantity = Some(quantity);
        self
    }

    pub fn with_entry_risk(mut self, stop_loss: Option<f64>, take_profit: Option<f64>) -> Self {
        self.stop_loss = stop_loss;
        self.take_profit = take_profit;
        self
    }

    /// Validate the required asset-unit quantity for opening decisions.
    ///
    /// Non-opening decisions do not need a quantity and return `Ok(None)`.
    pub fn validated_opening_quantity(&self) -> Result<Option<f64>, InvalidOpeningQuantity> {
        if !self.intent.opens_position() {
            return Ok(None);
        }

        validate_opening_quantity(self.quantity).map(Some)
    }
}

/// Validate an opening decision quantity as asset units/contracts.
pub fn validate_opening_quantity(quantity: Option<f64>) -> Result<f64, InvalidOpeningQuantity> {
    let quantity = quantity.ok_or(InvalidOpeningQuantity::Missing)?;

    if !quantity.is_finite() {
        return Err(InvalidOpeningQuantity::NonFinite);
    }

    if quantity == 0.0 {
        return Err(InvalidOpeningQuantity::Zero);
    }

    if quantity < 0.0 {
        return Err(InvalidOpeningQuantity::Negative);
    }

    Ok(quantity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn intent_classifies_only_open_long_and_open_short_as_opening() {
        let opening_intents = [
            StrategyDecisionIntent::OpenLong,
            StrategyDecisionIntent::OpenShort,
        ];
        let non_opening_intents = [
            StrategyDecisionIntent::Hold,
            StrategyDecisionIntent::CloseLong,
            StrategyDecisionIntent::CloseShort,
        ];

        assert!(opening_intents
            .into_iter()
            .all(StrategyDecisionIntent::opens_position));
        assert!(non_opening_intents
            .into_iter()
            .all(|intent| !intent.opens_position()));
    }

    #[test]
    fn decision_wraps_intent_and_reason_without_execution_behavior() {
        let decision = StrategyDecision::close_long().with_reason("target reached");

        assert_eq!(decision.intent, StrategyDecisionIntent::CloseLong);
        assert_eq!(decision.reason.as_deref(), Some("target reached"));
    }

    #[test]
    fn opening_quantity_is_asset_units_and_can_be_valid() {
        let decision = StrategyDecision::open_long(2.5);

        assert_eq!(decision.quantity, Some(2.5));
        assert_eq!(decision.validated_opening_quantity(), Ok(Some(2.5)));
    }

    #[test]
    fn opening_quantity_classifies_missing_non_finite_zero_and_negative() {
        assert_eq!(
            validate_opening_quantity(None),
            Err(InvalidOpeningQuantity::Missing)
        );
        assert_eq!(
            validate_opening_quantity(Some(f64::NAN)),
            Err(InvalidOpeningQuantity::NonFinite)
        );
        assert_eq!(
            validate_opening_quantity(Some(f64::INFINITY)),
            Err(InvalidOpeningQuantity::NonFinite)
        );
        assert_eq!(
            validate_opening_quantity(Some(0.0)),
            Err(InvalidOpeningQuantity::Zero)
        );
        assert_eq!(
            validate_opening_quantity(Some(-1.0)),
            Err(InvalidOpeningQuantity::Negative)
        );
    }

    #[test]
    fn non_opening_decisions_do_not_require_quantity() {
        assert_eq!(
            StrategyDecision::hold().validated_opening_quantity(),
            Ok(None)
        );
        assert_eq!(
            StrategyDecision::close_short().validated_opening_quantity(),
            Ok(None)
        );
    }

    #[test]
    fn entry_risk_fields_belong_to_opening_decisions_only() {
        let decision = StrategyDecision::open_short(3.0).with_entry_risk(Some(110.0), Some(90.0));

        assert_eq!(decision.stop_loss, Some(110.0));
        assert_eq!(decision.take_profit, Some(90.0));
        assert_eq!(decision.validated_opening_quantity(), Ok(Some(3.0)));
    }
}
