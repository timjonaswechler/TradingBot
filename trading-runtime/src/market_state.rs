//! Runtime-owned market history for one runtime asset.

use crate::RuntimeConfig;
use shared::{Candle, Timeframe};
use std::collections::HashMap;

/// DB-free market-data history grouped by configured timeframe.
#[derive(Debug, Clone, PartialEq)]
pub struct MarketState {
    histories: HashMap<Timeframe, Vec<Candle>>,
}

impl MarketState {
    pub(crate) fn from_config(config: &RuntimeConfig) -> Self {
        let mut histories = HashMap::new();
        histories.insert(config.primary_timeframe, Vec::new());
        for secondary in &config.secondary_timeframes {
            histories
                .entry(secondary.timeframe)
                .or_insert_with(Vec::new);
        }

        Self { histories }
    }

    /// Record accepted market input for a configured timeframe.
    ///
    /// Returns false when the timeframe is not configured, allowing compatibility
    /// wrappers to ignore unvalidated legacy input while `on_market_input` keeps
    /// returning `RuntimeInputError` before this method is reached.
    pub(crate) fn record_accepted_candle(&mut self, candle: Candle) -> bool {
        match self.histories.get_mut(&candle.timeframe) {
            Some(history) => {
                history.push(candle);
                true
            }
            None => false,
        }
    }

    /// Inspect a configured timeframe's chronological candle history.
    pub fn history(&self, timeframe: Timeframe) -> Option<&[Candle]> {
        self.histories
            .get(&timeframe)
            .map(|history| history.as_slice())
    }

    pub(crate) fn latest_completed_candle(&self, timeframe: Timeframe) -> Option<&Candle> {
        self.histories
            .get(&timeframe)
            .and_then(|history| history.last())
    }
}
