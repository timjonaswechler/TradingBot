pub mod engine;
pub mod tax;
// STUB — replace when merging with the real paper_trading::engine.
pub mod engine;
pub use engine::*;
pub use tax::*;

use anyhow::Result;
use chrono::Utc;

use crate::config::{CostsConfig, TaxConfig};
use crate::strategy::Signal;
use tax::calculate_tax;

#[derive(Debug, Clone)]
pub struct Position {
    pub asset:         String,
    pub quantity:      i64, // Stückzahl
    pub avg_buy_price: i64, // Durchschnittlicher Kaufpreis in Cent
}

#[derive(Debug, Clone)]
pub struct Trade {
    pub asset:          String,
    pub side:           TradeSide,
    pub quantity:       i64,
    pub price:          i64,           // in Cent
    pub fee:            i64,           // Broker-Kosten in Cent
    pub timestamp:      i64,           // Unix timestamp
    pub strategy:       String,
    pub gain_loss:      Option<i64>,   // realisierter Gewinn/Verlust in Cent (nur Sell)
    pub gain_loss_pct:  Option<f64>,   // Gewinn/Verlust in % des eingesetzten Kapitals (nur Sell)
    pub tax:            Option<i64>,   // Steuer in Cent (nur Sell)
}

#[derive(Debug, Clone, PartialEq)]
pub enum TradeSide {
    Buy,
    Sell,
}

pub struct PaperTradingEngine {
    pub cash:                i64,         // verfügbares Cash in Cent
    pub positions:           Vec<Position>,
    pub trades:              Vec<Trade>,
    pub exemption_remaining: i64,         // verbleibender Freistellungsauftrag in Cent
    costs:                   CostsConfig,
    tax_cfg:                 TaxConfig,
    position_size_pct:       u8,          // % des Cashs der pro Trade investiert wird
}

impl PaperTradingEngine {
    pub fn new(
        cash:                i64,
        exemption_remaining: i64,
        positions:           Vec<Position>,
        costs:               CostsConfig,
        tax_cfg:             TaxConfig,
        position_size_pct:   u8,
    ) -> Self {
        Self {
            cash,
            positions,
            trades: Vec::new(),
            exemption_remaining,
            costs,
            tax_cfg,
            position_size_pct,
        }
    }

    /// Verarbeitet ein Signal und führt ggf. einen Paper-Trade aus.
    pub fn execute(
        &mut self,
        signal:        &Signal,
        asset:         &str,
        current_price: i64,
        strategy_name: &str,
    ) -> Result<Option<Trade>> {
        match signal {
            Signal::Buy   => self.buy(asset, current_price, strategy_name),
            Signal::Sell  => self.sell(asset, current_price, strategy_name),
            Signal::Hold  => Ok(None),
            Signal::Short => Ok(None), // short selling not supported in this engine
            Signal::Buy  => self.buy(asset, current_price, strategy_name),
            Signal::Sell | Signal::Short => self.sell(asset, current_price, strategy_name),
            Signal::Hold => Ok(None),
        }
    }

    fn buy(&mut self, asset: &str, price: i64, strategy: &str) -> Result<Option<Trade>> {
        // Wieviel Cash darf in diese Position investiert werden?
        let budget   = self.cash * self.position_size_pct as i64 / 100;
        let fee_each = self.fee(price);
        // Wie viele ganze Stücke passen ins Budget?
        let quantity = if price + fee_each > 0 {
            (budget / (price + fee_each)).max(0)
        } else {
            0
        };

        if quantity == 0 {
            log::info!(
                "SKIP BUY {asset}: Budget {:.2}€ reicht nicht für 1 Stück à {:.2}€",
                budget as f64 / 100.0,
                price  as f64 / 100.0,
            );
            return Ok(None);
        }

        let fee   = self.fee(price * quantity); // flat = pro Order, percent = % vom Ordervolumen
        let total = (price * quantity) + fee;
        self.cash -= total;

        // Position aufstocken oder neu anlegen
        if let Some(pos) = self.positions.iter_mut().find(|p| p.asset == asset) {
            let new_qty   = pos.quantity + quantity;
            pos.avg_buy_price =
                (pos.avg_buy_price * pos.quantity + price * quantity) / new_qty;
            pos.quantity = new_qty;
        } else {
            self.positions.push(Position {
                asset:         asset.to_string(),
                quantity,
                avg_buy_price: price,
            });
        }

        let trade = Trade {
            asset:         asset.to_string(),
            side:          TradeSide::Buy,
            quantity,
            price,
            fee,
            timestamp:     Utc::now().timestamp(),
            strategy:      strategy.to_string(),
            gain_loss:     None,
            gain_loss_pct: None,
            tax:           None,
        };
        self.trades.push(trade.clone());
        log::info!(
            "BUY  {} x {asset} @ {:.2}€  fee {:.2}€  cash {:.2}€",
            quantity,
            price     as f64 / 100.0,
            fee       as f64 / 100.0,
            self.cash as f64 / 100.0
        );
        Ok(Some(trade))
    }

    fn sell(&mut self, asset: &str, price: i64, strategy: &str) -> Result<Option<Trade>> {
        let idx = self.positions.iter().position(|p| p.asset == asset);
        let Some(idx) = idx else {
            log::info!("SKIP SELL {asset}: Keine Position vorhanden");
            return Ok(None);
        };

        let pos      = self.positions.remove(idx);
        let fee      = self.fee(price * pos.quantity); // flat pro Order, nicht pro Stück
        let proceeds = price * pos.quantity - fee;
        let cost     = pos.avg_buy_price * pos.quantity;
        let gain     = proceeds - cost;

        let tax_result = if gain > 0 {
            let r = calculate_tax(gain, &self.tax_cfg, self.exemption_remaining);
            self.exemption_remaining -= r.exemption_used;
            r
        } else {
            tax::TaxResult { tax: 0, exemption_used: 0 }
        };

        self.cash += proceeds - tax_result.tax;

        // Prozentsatz des Gewinns/Verlusts relativ zum eingesetzten Kapital
        let gain_loss_pct = if cost > 0 {
            Some(gain as f64 / cost as f64 * 100.0)
        } else {
            None
        };

        let trade = Trade {
            asset:         asset.to_string(),
            side:          TradeSide::Sell,
            quantity:      pos.quantity,
            price,
            fee,
            timestamp:     Utc::now().timestamp(),
            strategy:      strategy.to_string(),
            gain_loss:     Some(gain),
            gain_loss_pct,
            tax:           Some(tax_result.tax),
        };
        self.trades.push(trade.clone());
        log::info!(
            "SELL {asset} @ {:.2}€  G/L {:.2}€  Steuer {:.2}€  cash {:.2}€",
            price              as f64 / 100.0,
            gain               as f64 / 100.0,
            tax_result.tax     as f64 / 100.0,
            self.cash          as f64 / 100.0
        );
        Ok(Some(trade))
    }

    /// Berechnet die Ordergebühr.
    /// flat:    fixer Betrag pro Order (unabhängig von Stückzahl/Volumen)
    /// percent: Prozentsatz des Ordervolumens (in Basispunkten, 100bp = 1%)
    fn fee(&self, order_volume: i64) -> i64 {
        match self.costs.commission_type.as_str() {
            "percent" => order_volume * self.costs.commission_amount / 10_000,
            _         => self.costs.commission_amount, // flat pro Order
        }
    }

    /// Gesamtwert des Portfolios (Cash + Marktwert aller Positionen).
    pub fn total_value(&self, prices: &std::collections::HashMap<String, i64>) -> i64 {
        let pos_value: i64 = self
            .positions
            .iter()
            .map(|p| prices.get(&p.asset).copied().unwrap_or(0) * p.quantity)
            .sum();
        self.cash + pos_value
    }
}
