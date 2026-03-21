// STUB — replace when merging with the real paper_trading::engine implementation.
use chrono::{DateTime, Utc};
use crate::market_data::Candle;
use crate::strategy::Signal;

#[derive(Debug, Clone)]
pub enum TradeSide { Buy, Sell, Short, Cover }

#[derive(Debug, Clone)]
pub struct Trade {
    pub side:             TradeSide,
    pub quantity:         i64,
    pub price_cents:      i64,
    pub timestamp:        DateTime<Utc>,
    pub pnl_cents:        i64,
    pub commission_cents: i64,
}

#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub starting_capital_cents: i64,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self { starting_capital_cents: 10_000_00 } // 10 000 €
    }
}

pub struct PaperTradingEngine {
    pub trades:       Vec<Trade>,
    pub equity_curve: Vec<(DateTime<Utc>, i64)>,
    cfg:              TradingConfig,
}

impl PaperTradingEngine {
    pub fn new(cfg: TradingConfig) -> Self {
        Self { trades: vec![], equity_curve: vec![], cfg }
    }

    pub fn execute(&mut self, _sig: &Signal, _sym: &str, _candle: &Candle) {}

    pub fn snapshot_equity(&mut self, _sym: &str, price: i64, ts: DateTime<Utc>) {
        self.equity_curve.push((ts, price));
    }

    pub fn total_equity_cents(&self) -> i64 {
        self.cfg.starting_capital_cents
    }
}
