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
// STUB for paper_trading::engine — replace with full implementation when merged
use std::collections::HashMap;

use chrono::{DateTime, Utc};

use crate::market_data::Candle;
use crate::paper_trading::tax::{self, TaxConfig};

#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
    Short,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TradeSide {
    Buy,
    Sell,
    Short,
    Cover,
    Cover, // closing a short position
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub side: TradeSide,
    pub quantity: i64,
    pub price_cents: i64,
    pub entry_price_cents: i64,
    pub timestamp: i64,
    pub pnl_cents: i64,
    pub commission_cents: i64,
}
    pub symbol: String,
    pub side: TradeSide,
    pub quantity: i64,        // shares
    pub price_cents: i64,
    pub timestamp: DateTime<Utc>,
    pub pnl_cents: i64,       // realized PnL for this trade (0 for opening trades)
    pub commission_cents: i64,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub quantity: i64,        // positive = long, negative = short
    pub avg_cost_cents: i64,  // average cost basis per share
    pub entry_timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub enum CommissionType {
    Flat,    // fixed amount per trade
    Percent, // basis points (1bp = 0.01%)
}

#[derive(Debug, Clone)]
pub struct TradingConfig {
    pub starting_capital_cents: i64,   // e.g., 10_000_00 = €10,000
    pub commission_type: CommissionType,
    pub commission_amount: i64,        // cents (Flat) or basis points (Percent)
    pub position_size_pct: f64,        // fraction of portfolio per trade [0.0..1.0], default 0.95
    pub max_short_size_pct: f64,       // max short as % of portfolio [0.0..1.0], default 0.5
    pub tax: TaxConfig,
}

impl Default for TradingConfig {
    fn default() -> Self {
        Self { starting_capital_cents: 10_000_00 } // 10 000 €
        Self {
            starting_capital_cents: 10_000_00,
            commission_type: CommissionType::Flat,
            commission_amount: 100, // €1 per trade
            position_size_pct: 0.95,
            max_short_size_pct: 0.5,
            tax: Default::default(),
        }
    }
}

impl TradingConfig {
    /// Build from the application config.
    pub fn from_app_config(cfg: &crate::config::Config) -> Self {
        Self {
            starting_capital_cents: cfg.paper_trading.starting_capital,
            commission_type: CommissionType::Flat,
            commission_amount: cfg.costs.commission_amount,
            position_size_pct: cfg.paper_trading.position_size_pct as f64 / 100.0,
            max_short_size_pct: 0.5,
            tax: crate::paper_trading::tax::TaxConfig {
                freistellungsauftrag_cents: cfg.tax.freistellungsauftrag,
                kirchensteuer: cfg.tax.kirchensteuer,
                kirchensteuer_rate: 0.09,
            },
        }
    }
}

impl From<crate::strategy::Signal> for Signal {
    fn from(s: crate::strategy::Signal) -> Self {
        match s {
            crate::strategy::Signal::Buy  => Signal::Buy,
            crate::strategy::Signal::Sell => Signal::Sell,
            crate::strategy::Signal::Hold => Signal::Hold,
        }
    }
}

pub struct PaperTradingEngine {
    pub trades:       Vec<Trade>,
    pub equity_curve: Vec<(DateTime<Utc>, i64)>,
    cfg:              TradingConfig,
    pub cfg: TradingConfig,
    pub cash_cents: i64,
    pub positions: HashMap<String, Position>,
    pub trades: Vec<Trade>,
    /// Equity curve: (timestamp, total_equity_cents)
    pub equity_curve: Vec<(DateTime<Utc>, i64)>,
    accumulated_gains_cents: i64, // for tax tracking
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
        let cash = cfg.starting_capital_cents;
        Self {
            cfg,
            cash_cents: cash,
            positions: HashMap::new(),
            trades: Vec::new(),
            equity_curve: Vec::new(),
            accumulated_gains_cents: 0,
        }
    }

    /// Execute a signal for a symbol at the given candle's close price.
    /// Called once per bar for each symbol.
    pub fn execute(&mut self, signal: &Signal, symbol: &str, candle: &Candle) {
        let price = candle.close;
        let ts = candle.timestamp;

        // If Buy signal and there is an existing short, cover first.
        if *signal == Signal::Buy {
            if self.positions.get(symbol).map(|p| p.quantity < 0).unwrap_or(false) {
                self.cover(symbol, price, ts);
            }
        }

        match signal {
            Signal::Buy => self.open_long(symbol, price, ts),
            Signal::Sell => self.close_long(symbol, price, ts),
            Signal::Short => self.open_short(symbol, price, ts),
            Signal::Hold => {}
        }
    }

    /// Total equity = cash + sum of position market values at current prices.
    /// Call this after each bar with the latest candle close price.
    pub fn snapshot_equity(
        &mut self,
        symbol: &str,
        current_price_cents: i64,
        timestamp: DateTime<Utc>,
    ) {
        let pos_value: i64 = self
            .positions
            .iter()
            .map(|(sym, pos)| {
                if sym == symbol {
                    pos.quantity * current_price_cents
                } else {
                    pos.quantity * pos.avg_cost_cents
                }
            })
            .sum();
        let equity = self.cash_cents + pos_value;
        self.equity_curve.push((timestamp, equity));
    }

    /// Returns total equity cents (most recent equity_curve entry or starting_capital if empty)
    pub fn total_equity_cents(&self) -> i64 {
        self.equity_curve
            .last()
            .map(|(_, e)| *e)
            .unwrap_or(self.cfg.starting_capital_cents)
    }

    /// Returns total return as percentage
    pub fn total_return_pct(&self) -> f64 {
        let start = self.cfg.starting_capital_cents as f64;
        let current = self.total_equity_cents() as f64;
        if start == 0.0 {
            return 0.0;
        }
        (current - start) / start * 100.0
    }

    fn compute_commission(&self, trade_value_cents: i64) -> i64 {
        match self.cfg.commission_type {
            CommissionType::Flat => self.cfg.commission_amount,
            CommissionType::Percent => trade_value_cents * self.cfg.commission_amount / 10_000,
        }
    }

    fn open_long(&mut self, symbol: &str, price: i64, ts: DateTime<Utc>) {
        // Only open if no existing long position.
        if self.positions.get(symbol).map(|p| p.quantity > 0).unwrap_or(false) {
            return;
        }

        // Account for flat commission in quantity calculation so cost never exceeds budget.
        let budget = (self.cash_cents as f64 * self.cfg.position_size_pct) as i64;
        let flat_commission = match self.cfg.commission_type {
            CommissionType::Flat => self.cfg.commission_amount,
            CommissionType::Percent => 0,
        };
        let quantity = ((budget - flat_commission).max(0) / price).max(0);
        if quantity <= 0 {
            return;
        }

        let trade_value = quantity * price;
        let commission = self.compute_commission(trade_value);
        let cost = trade_value + commission;

        if cost > self.cash_cents {
            return;
        }

        self.cash_cents -= cost;
        self.positions.insert(
            symbol.to_string(),
            Position {
                symbol: symbol.to_string(),
                quantity,
                avg_cost_cents: price,
                entry_timestamp: ts,
            },
        );
        self.trades.push(Trade {
            symbol: symbol.to_string(),
            side: TradeSide::Buy,
            quantity,
            price_cents: price,
            timestamp: ts,
            pnl_cents: 0,
            commission_cents: commission,
        });
    }

    fn close_long(&mut self, symbol: &str, price: i64, ts: DateTime<Utc>) {
        let pos = match self.positions.remove(symbol) {
            Some(p) if p.quantity > 0 => p,
            Some(p) => {
                // Put back if it wasn't a long
                self.positions.insert(symbol.to_string(), p);
                return;
            }
            None => return,
        };

        let trade_value = pos.quantity * price;
        let commission = self.compute_commission(trade_value);
        let revenue = trade_value - commission;
        let cost_basis = pos.quantity * pos.avg_cost_cents;
        let raw_gain = revenue - cost_basis;

        let tax_due = if raw_gain > 0 {
            tax::compute_tax(raw_gain, self.accumulated_gains_cents, &self.cfg.tax)
        } else {
            0
        };

        if raw_gain > 0 {
            self.accumulated_gains_cents += raw_gain;
        }

        let pnl = raw_gain - tax_due;
        self.cash_cents += revenue - tax_due;

        self.trades.push(Trade {
            symbol: symbol.to_string(),
            side: TradeSide::Sell,
            quantity: pos.quantity,
            price_cents: price,
            timestamp: ts,
            pnl_cents: pnl,
            commission_cents: commission,
        });
    }

    fn open_short(&mut self, symbol: &str, price: i64, ts: DateTime<Utc>) {
        // Only open if no position (long or short) exists.
        if self.positions.contains_key(symbol) {
            return;
        }

        let quantity =
            (self.cash_cents as f64 * self.cfg.max_short_size_pct / price as f64) as i64;
        if quantity <= 0 {
            return;
        }

        // Record short position as negative quantity.
        self.positions.insert(
            symbol.to_string(),
            Position {
                symbol: symbol.to_string(),
                quantity: -quantity,
                avg_cost_cents: price,
                entry_timestamp: ts,
            },
        );
        self.trades.push(Trade {
            symbol: symbol.to_string(),
            side: TradeSide::Short,
            quantity,
            price_cents: price,
            timestamp: ts,
            pnl_cents: 0,
            commission_cents: 0,
        });
    }

    fn cover(&mut self, symbol: &str, price: i64, ts: DateTime<Utc>) {
        let pos = match self.positions.remove(symbol) {
            Some(p) if p.quantity < 0 => p,
            Some(p) => {
                self.positions.insert(symbol.to_string(), p);
                return;
            }
            None => return,
        };

        let shares = pos.quantity.unsigned_abs() as i64;
        let trade_value = shares * price;
        let commission = self.compute_commission(trade_value);
        // Short profit: sold high (avg_cost), buy back low (current price)
        let pnl = (pos.avg_cost_cents - price) * shares - commission;

        // For short covers: cash adjusts by how much was gained/lost.
        // We treat it as: we get back our collateral adjusted by pnl.
        self.cash_cents += pnl;

        self.trades.push(Trade {
            symbol: symbol.to_string(),
            side: TradeSide::Cover,
            quantity: shares,
            price_cents: price,
            timestamp: ts,
            pnl_cents: pnl,
            commission_cents: commission,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn make_candle(price: i64) -> Candle {
        Candle {
            timestamp: Utc.with_ymd_and_hms(2024, 1, 1, 0, 0, 0).unwrap(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1000,
        }
    }

    fn make_candle_ts(price: i64, day: u32) -> Candle {
        Candle {
            timestamp: Utc.with_ymd_and_hms(2024, 1, day, 0, 0, 0).unwrap(),
            open: price,
            high: price,
            low: price,
            close: price,
            volume: 1000,
        }
    }

    #[test]
    fn test_buy_sell_pnl() {
        let cfg = TradingConfig {
            starting_capital_cents: 1_000_00, // €1,000
            commission_type: CommissionType::Flat,
            commission_amount: 100, // €1 flat
            position_size_pct: 1.0,
            max_short_size_pct: 0.5,
            tax: TaxConfig {
                freistellungsauftrag_cents: 0, // no allowance so we can compute exactly
                kirchensteuer: false,
                kirchensteuer_rate: 0.09,
            },
        };

        let mut engine = PaperTradingEngine::new(cfg);

        // Buy at €10/share: with €1,000 cash and flat €1 commission
        // budget = 100_000, flat_commission = 100
        // quantity = (100_000 - 100) / 1_000 = 99 shares
        // cost = 99 * 1_000 + 100 = 99_100 cents
        let buy_candle = make_candle(1_000); // €10.00
        engine.execute(&Signal::Buy, "AAPL", &buy_candle);

        assert_eq!(engine.positions.len(), 1);
        let pos = engine.positions.get("AAPL").unwrap();
        assert_eq!(pos.quantity, 99);
        assert_eq!(engine.cash_cents, 1_000_00 - (99 * 1_000 + 100));

        // Sell at €15/share
        // revenue = 99 * 1_500 - 100 = 148_400 cents
        // cost_basis = 99 * 1_000 = 99_000 cents
        // raw_gain = 148_400 - 99_000 = 49_400 cents
        // base_tax = 49_400 * 25/100 = 12_350; soli = 12_350 * 55/1_000 = 679
        // tax = 13_029; pnl = 49_400 - 13_029 = 36_371 cents
        let sell_candle = make_candle(1_500); // €15.00
        engine.execute(&Signal::Sell, "AAPL", &sell_candle);

        assert!(engine.positions.is_empty());
        assert_eq!(engine.trades.len(), 2);

        let sell_trade = &engine.trades[1];
        assert_eq!(sell_trade.side, TradeSide::Sell);
        assert!(sell_trade.pnl_cents > 0);
        assert_eq!(sell_trade.pnl_cents, 36_371);
    }

    #[test]
    fn test_commission_deduction() {
        let cfg = TradingConfig {
            starting_capital_cents: 1_000_00,
            commission_type: CommissionType::Flat,
            commission_amount: 500, // €5 flat
            position_size_pct: 1.0,
            max_short_size_pct: 0.5,
            tax: TaxConfig {
                freistellungsauftrag_cents: i64::MAX, // ignore tax
                kirchensteuer: false,
                kirchensteuer_rate: 0.09,
            },
        };

        let mut engine = PaperTradingEngine::new(cfg);
        let buy_candle = make_candle(1_000); // €10/share
        engine.execute(&Signal::Buy, "TEST", &buy_candle);

        let trade = &engine.trades[0];
        assert_eq!(trade.commission_cents, 500);
        // quantity = (100_000 - 500) / 1_000 = 99
        // cost = 99 * 1_000 + 500 = 99_500
        assert_eq!(engine.cash_cents, 1_000_00 - 99_500);
    }

    #[test]
    fn test_short_cover_profit() {
        let cfg = TradingConfig {
            starting_capital_cents: 1_000_00,
            commission_type: CommissionType::Flat,
            commission_amount: 0, // no commission to simplify
            position_size_pct: 1.0,
            max_short_size_pct: 0.5,
            tax: TaxConfig::default(),
        };

        let mut engine = PaperTradingEngine::new(cfg);

        // Short at €20/share, max_short_size_pct = 0.5
        // quantity = floor(100_000 * 0.5 / 2_000) = 25 shares
        let short_candle = make_candle_ts(2_000, 1); // €20/share
        engine.execute(&Signal::Short, "SPY", &short_candle);

        assert_eq!(engine.positions.get("SPY").unwrap().quantity, -25);

        // Cover at €15/share (price fell — profit for short seller)
        // pnl = (2000 - 1500) * 25 - 0 = 12_500 cents = €125
        let cover_candle = make_candle_ts(1_500, 2); // €15/share
        engine.execute(&Signal::Buy, "SPY", &cover_candle);

        // Position should be covered, then Buy opens new long
        // After cover: cash += 12_500
        let cover_trade = engine.trades.iter().find(|t| t.side == TradeSide::Cover).unwrap();
        assert_eq!(cover_trade.pnl_cents, 12_500);
    }

    #[test]
    fn test_tax_freistellungsauftrag() {
        let cfg = TaxConfig {
            freistellungsauftrag_cents: 100_100, // €1,001
            kirchensteuer: false,
            kirchensteuer_rate: 0.09,
        };

        // Gain of exactly €500 — within allowance, no tax
        let tax = tax::compute_tax(50_000, 0, &cfg);
        assert_eq!(tax, 0);

        // Gain of €500 but €800 already accumulated — only €201 remaining allowance
        // taxable = 50_000 - (100_100 - 80_000) = 50_000 - 20_100 = 29_900
        // base_tax = 29_900 * 25 / 100 = 7_475
        // soli = 7_475 * 55 / 1_000 = 411
        let tax2 = tax::compute_tax(50_000, 80_000, &cfg);
        assert_eq!(tax2, 7_886);
    }

    #[test]
    fn test_insufficient_cash_no_trade() {
        let cfg = TradingConfig {
            starting_capital_cents: 100, // only €1 — can't buy anything at €10
            ..Default::default()
        };

        let mut engine = PaperTradingEngine::new(cfg);
        let buy_candle = make_candle(1_000); // €10/share
        engine.execute(&Signal::Buy, "AAPL", &buy_candle);

        assert!(engine.positions.is_empty());
        assert!(engine.trades.is_empty());
    }

    #[test]
    fn test_snapshot_equity() {
        let cfg = TradingConfig {
            starting_capital_cents: 1_000_00,
            commission_type: CommissionType::Flat,
            commission_amount: 0,
            position_size_pct: 1.0,
            max_short_size_pct: 0.5,
            tax: TaxConfig {
                freistellungsauftrag_cents: i64::MAX,
                kirchensteuer: false,
                kirchensteuer_rate: 0.09,
            },
        };

        let mut engine = PaperTradingEngine::new(cfg);
        let buy_candle = make_candle_ts(1_000, 1);
        engine.execute(&Signal::Buy, "TEST", &buy_candle);

        // 100 shares bought at €10, no commission → cash = 0
        assert_eq!(engine.cash_cents, 0);

        // Snapshot equity with price at €12
        let ts = Utc.with_ymd_and_hms(2024, 1, 2, 0, 0, 0).unwrap();
        engine.snapshot_equity("TEST", 1_200, ts);

        // equity = 0 cash + 100 * 1200 = 120_000
        assert_eq!(engine.total_equity_cents(), 120_000);

        let ret = engine.total_return_pct();
        // (120_000 - 100_000) / 100_000 * 100 = 20%
        assert!((ret - 20.0).abs() < 0.01);
    }
}
