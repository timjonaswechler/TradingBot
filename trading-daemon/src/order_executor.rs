/// Order execution abstraction + Paper Trading implementation.
///
/// The `OrderExecutor` trait is designed for future broker integration —
/// `PaperExecutor` simulates trades locally and persists them to SpacetimeDB.
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};

use db_layer::{close_position, insert_trade, open_position, get_open_position, DbConnection};
use shared::{Candle, Position, PositionSide, Signal, TradeDecision};

// ── Trait ─────────────────────────────────────────────────────────────────────

#[async_trait]
pub trait OrderExecutor: Send + Sync {
    async fn handle(&mut self, candle: &Candle, decision: &TradeDecision) -> Result<()>;
    fn balance(&self) -> f64;
    fn position(&self) -> Option<&Position>;
}

// ── PaperExecutor ─────────────────────────────────────────────────────────────

/// Simulates trades without a real broker.
/// Persists open positions and completed trades to SpacetimeDB.
pub struct PaperExecutor {
    balance:     f64,
    position:    Option<Position>,
    position_id: Option<u64>,
    conn:        Arc<DbConnection>,
    strategy:    String,
    symbol:      String,
}

impl PaperExecutor {
    /// Create a new paper executor and restore any open position from SpacetimeDB.
    pub fn new(
        conn: Arc<DbConnection>,
        strategy: String,
        symbol: String,
        balance: f64,
    ) -> Self {
        // Try to restore an open position from the DB cache.
        let (position, position_id) =
            match get_open_position(&conn, &strategy, &symbol) {
                Some(db_pos) => {
                    let id = db_pos.id;
                    let (_, _, pos) = db_layer::db_position_to_shared(db_pos);
                    info!(strategy, symbol, "Restored open position from DB");
                    (Some(pos), Some(id))
                }
                None => (None, None),
            };

        Self {
            balance,
            position,
            position_id,
            conn,
            strategy,
            symbol,
        }
    }

    #[allow(dead_code)]
    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }

    async fn open_long_position(&mut self, candle: &Candle, decision: &TradeDecision) -> Result<()> {
        if self.position.is_some() {
            warn!(symbol = self.symbol, "BUY signal but position already open — ignoring");
            return Ok(());
        }

        let size         = self.balance * decision.size / candle.close;
        let entry_price  = candle.close;
        let stop_loss    = decision.stop_loss.unwrap_or(0.0);
        let take_profit  = decision.take_profit.unwrap_or(0.0);
        let entry_reason = decision.reason.clone().unwrap_or_default();
        let entry_time   = candle.timestamp;
        let strategy     = self.strategy.clone();
        let symbol       = self.symbol.clone();
        let conn         = self.conn.clone();

        let pos_id = tokio::task::spawn_blocking(move || {
            open_position(
                &conn, &strategy, &symbol, "long",
                entry_price, size, stop_loss, take_profit,
                entry_time, &entry_reason,
            )?;
            // Get the ID back from cache
            Ok::<Option<u64>, anyhow::Error>(
                get_open_position(&conn, &strategy, &symbol).map(|p| p.id)
            )
        }).await??;

        self.position = Some(Position {
            symbol:      self.symbol.clone(),
            side:        PositionSide::Long,
            entry_price,
            size,
            entry_time:  candle.timestamp,
            stop_loss:   decision.stop_loss,
            take_profit: decision.take_profit,
        });
        self.position_id = pos_id;

        info!(
            symbol    = self.symbol,
            price     = entry_price,
            size,
            balance   = self.balance,
            reason    = decision.reason.as_deref().unwrap_or(""),
            "📈 BUY"
        );
        Ok(())
    }

    async fn close_long_position(&mut self, candle: &Candle, _decision: &TradeDecision, reason: &str) -> Result<()> {
        let pos = match self.position.take() {
            Some(p) => p,
            None => {
                warn!(symbol = self.symbol, "SELL signal but no open position — ignoring");
                return Ok(());
            }
        };

        let exit_price   = candle.close;
        let pnl          = (exit_price - pos.entry_price) * pos.size;
        let exit_time    = candle.timestamp;
        let position_id  = self.position_id.take();
        let strategy     = self.strategy.clone();
        let symbol       = self.symbol.clone();
        let conn         = self.conn.clone();
        let entry_reason = String::new();
        let exit_reason  = reason.to_string();
        let entry_price  = pos.entry_price;
        let size         = pos.size;
        let entry_time   = pos.entry_time;

        tokio::task::spawn_blocking(move || {
            if let Some(id) = position_id {
                close_position(&conn, id)?;
            }
            insert_trade(
                &conn, &strategy, &symbol, "long",
                entry_price, exit_price, size, pnl, "closed",
                entry_time, exit_time,
                &entry_reason, &exit_reason,
            )
        }).await??;

        self.balance += pnl;

        info!(
            symbol    = self.symbol,
            entry     = pos.entry_price,
            exit      = exit_price,
            pnl,
            balance   = self.balance,
            reason,
            "📉 SELL"
        );
        Ok(())
    }

    /// Check stop-loss and take-profit on the current candle.
    async fn check_stops(&mut self, candle: &Candle) -> Result<()> {
        let (hit_sl, hit_tp) = match &self.position {
            None => return Ok(()),
            Some(pos) => {
                let hit_sl = pos.stop_loss
                    .map(|sl| candle.low <= sl)
                    .unwrap_or(false);
                let hit_tp = pos.take_profit
                    .map(|tp| candle.high >= tp)
                    .unwrap_or(false);
                (hit_sl, hit_tp)
            }
        };

        if hit_sl {
            let reason = "stop-loss triggered".to_string();
            let decision = TradeDecision::hold();
            self.close_long_position(candle, &decision, &reason).await?;
        } else if hit_tp {
            let reason = "take-profit triggered".to_string();
            let decision = TradeDecision::hold();
            self.close_long_position(candle, &decision, &reason).await?;
        }

        Ok(())
    }
}

#[async_trait]
impl OrderExecutor for PaperExecutor {
    async fn handle(&mut self, candle: &Candle, decision: &TradeDecision) -> Result<()> {
        // Check stops first — they take priority over strategy signals.
        self.check_stops(candle).await?;

        match decision.signal {
            Signal::Buy   => self.open_long_position(candle, decision).await?,
            Signal::Sell  => {
                let reason = decision.reason.clone().unwrap_or_else(|| "strategy sell".into());
                self.close_long_position(candle, decision, &reason).await?
            }
            Signal::Hold  => {}
            Signal::Short | Signal::Cover => {
                // Short selling not yet implemented in paper executor.
                warn!(signal = ?decision.signal, "Short/Cover not yet supported in PaperExecutor");
            }
        }

        Ok(())
    }

    fn balance(&self) -> f64 {
        self.balance
    }

    fn position(&self) -> Option<&Position> {
        self.position.as_ref()
    }
}
