//! Runtime step return values.

use crate::{RuntimeEvent, RuntimePortfolioSnapshot};

/// The DB-free result of one runtime entrypoint call.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeStep {
    /// Events in the exact order they occurred during this step.
    pub events: Vec<RuntimeEvent>,
    /// Current portfolio snapshot after the step completed.
    pub portfolio_snapshot: RuntimePortfolioSnapshot,
}

impl RuntimeStep {
    pub fn new(events: Vec<RuntimeEvent>, portfolio_snapshot: RuntimePortfolioSnapshot) -> Self {
        Self {
            events,
            portfolio_snapshot,
        }
    }
}
