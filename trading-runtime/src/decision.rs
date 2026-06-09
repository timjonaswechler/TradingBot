//! Strategy-produced decisions for a Strategy Tick.

/// Direction-aware strategy intent for one Strategy Tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrategyDecisionIntent {
    Hold,
    OpenLong,
    CloseLong,
    OpenShort,
    CloseShort,
    UpdatePositionRisk,
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

/// Requested change for one current Position Risk Boundary.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum RiskBoundaryChange {
    #[default]
    Unchanged,
    Set(f64),
    Clear,
}

/// Requested Position Risk Boundary changes for a Position Risk Update decision.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PositionRiskBoundaryChanges {
    pub stop_loss: RiskBoundaryChange,
    pub take_profit: RiskBoundaryChange,
}

impl PositionRiskBoundaryChanges {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_stop_loss(mut self, price: f64) -> Self {
        self.stop_loss = RiskBoundaryChange::Set(price);
        self
    }

    pub fn clear_stop_loss(mut self) -> Self {
        self.stop_loss = RiskBoundaryChange::Clear;
        self
    }

    pub fn set_take_profit(mut self, price: f64) -> Self {
        self.take_profit = RiskBoundaryChange::Set(price);
        self
    }

    pub fn clear_take_profit(mut self) -> Self {
        self.take_profit = RiskBoundaryChange::Clear;
        self
    }
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
    /// Position Risk Boundary changes for update decisions only.
    ///
    /// Omitted boundary changes default to [`RiskBoundaryChange::Unchanged`].
    pub position_risk_changes: PositionRiskBoundaryChanges,
}

impl StrategyDecision {
    pub fn new(intent: StrategyDecisionIntent) -> Self {
        Self {
            intent,
            reason: None,
            quantity: None,
            stop_loss: None,
            take_profit: None,
            position_risk_changes: PositionRiskBoundaryChanges::default(),
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

    pub fn update_position_risk() -> Self {
        Self::new(StrategyDecisionIntent::UpdatePositionRisk)
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

    pub fn with_position_risk_changes(mut self, changes: PositionRiskBoundaryChanges) -> Self {
        self.position_risk_changes = changes;
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
            StrategyDecisionIntent::UpdatePositionRisk,
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
        assert_eq!(
            decision.position_risk_changes,
            PositionRiskBoundaryChanges::default()
        );
        assert_eq!(decision.validated_opening_quantity(), Ok(Some(3.0)));
    }

    #[test]
    fn position_risk_update_decision_defaults_omitted_boundaries_to_unchanged() {
        let decision = StrategyDecision::update_position_risk();

        assert_eq!(decision.intent, StrategyDecisionIntent::UpdatePositionRisk);
        assert_eq!(
            decision.position_risk_changes,
            PositionRiskBoundaryChanges {
                stop_loss: RiskBoundaryChange::Unchanged,
                take_profit: RiskBoundaryChange::Unchanged,
            }
        );
        assert_eq!(decision.validated_opening_quantity(), Ok(None));
    }

    #[test]
    fn position_risk_update_decision_carries_explicit_set_and_clear_changes() {
        let changes = PositionRiskBoundaryChanges::new()
            .set_stop_loss(95.0)
            .clear_take_profit();
        let decision = StrategyDecision::update_position_risk().with_position_risk_changes(changes);

        assert_eq!(
            decision.position_risk_changes.stop_loss,
            RiskBoundaryChange::Set(95.0)
        );
        assert_eq!(
            decision.position_risk_changes.take_profit,
            RiskBoundaryChange::Clear
        );
        assert_eq!(decision.stop_loss, None);
        assert_eq!(decision.take_profit, None);
    }
}
