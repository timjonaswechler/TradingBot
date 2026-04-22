/// Order execution abstraction + Paper Trading implementation.
///
/// The `OrderExecutor` trait is designed for future broker integration —
/// `PaperExecutor` simulates trades locally and persists them to SpacetimeDB.
use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use tracing::{info, warn};

use db_layer::{close_position, insert_trade, open_position, get_open_position, DbConnection};
use shared::{plan_action, realized_pnl as compute_realized_pnl, Action, Candle, Position, PositionSide, TradeDecision};

/// How long to wait for an `open_position` reducer's row to propagate into the
/// local SDK cache before giving up. Reducers typically land in < 50 ms; we
/// allow ~1 s of slack.
const OPEN_POSITION_POLL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);
const OPEN_POSITION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);

/// Block until an open position for `(strategy, symbol)` is visible in the
/// local cache, returning its `id`. Returns `None` on timeout.
fn wait_for_open_position(
    conn: &DbConnection,
    strategy: &str,
    symbol: &str,
) -> Option<u64> {
    let deadline = std::time::Instant::now() + OPEN_POSITION_POLL_TIMEOUT;
    loop {
        if let Some(p) = get_open_position(conn, strategy, symbol) {
            return Some(p.id);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(OPEN_POSITION_POLL_INTERVAL);
    }
}

fn side_str(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long  => "long",
        PositionSide::Short => "short",
    }
}

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
        let (position, position_id) =
            match get_open_position(&conn, &strategy, &symbol) {
                Some(db_pos) => {
                    let id = db_pos.id;
                    let (_, _, pos) = db_layer::db_position_to_shared(db_pos);
                    info!(strategy, symbol, side = ?pos.side, "Restored open position from DB");
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

    async fn open_new_position(
        &mut self,
        side: PositionSide,
        candle: &Candle,
        decision: &TradeDecision,
    ) -> Result<()> {
        if self.position.is_some() {
            warn!(
                symbol = self.symbol,
                signal = ?decision.signal,
                "Open signal but position already open — ignoring"
            );
            return Ok(());
        }

        let size         = self.balance * decision.size / candle.close;
        let entry_price  = candle.close;
        let stop_loss    = decision.stop_loss.unwrap_or(0.0);
        let take_profit  = decision.take_profit.unwrap_or(0.0);
        let entry_reason = decision.reason.clone().unwrap_or_default();
        let entry_time   = candle.timestamp;
        let side_s       = side_str(side).to_string();
        let strategy     = self.strategy.clone();
        let symbol       = self.symbol.clone();
        let conn         = self.conn.clone();

        let pos_id = tokio::task::spawn_blocking(move || {
            open_position(
                &conn, &strategy, &symbol, &side_s,
                entry_price, size, stop_loss, take_profit,
                entry_time, &entry_reason,
            )?;
            Ok::<Option<u64>, anyhow::Error>(
                wait_for_open_position(&conn, &strategy, &symbol),
            )
        }).await??;

        if pos_id.is_none() {
            warn!(
                symbol = self.symbol,
                "Open-position reducer fired but row never appeared in cache — \
                 close will rely on (strategy,symbol) lookup"
            );
        }

        self.position = Some(Position {
            symbol:      self.symbol.clone(),
            side,
            entry_price,
            size,
            entry_time:  candle.timestamp,
            stop_loss:   decision.stop_loss,
            take_profit: decision.take_profit,
        });
        self.position_id = pos_id;

        let tag = match side {
            PositionSide::Long  => "📈 BUY",
            PositionSide::Short => "📉 SHORT",
        };
        info!(
            symbol  = self.symbol,
            price   = entry_price,
            size,
            balance = self.balance,
            reason  = decision.reason.as_deref().unwrap_or(""),
            "{tag}"
        );
        Ok(())
    }

    /// Force-close the currently open position at `candle.close`.
    /// Used by the daemon on graceful shutdown when `liquidate_on_shutdown = true`.
    pub async fn liquidate(&mut self, candle: &Candle, reason: &str) -> Result<()> {
        if self.position.is_none() {
            return Ok(());
        }
        self.close_current_position(candle, reason).await
    }

    async fn close_current_position(&mut self, candle: &Candle, reason: &str) -> Result<()> {
        let pos = match self.position.take() {
            Some(p) => p,
            None => {
                warn!(symbol = self.symbol, "Close signal but no open position — ignoring");
                return Ok(());
            }
        };

        let exit_price   = candle.close;
        let pnl          = compute_realized_pnl(pos.side, pos.entry_price, exit_price, pos.size);
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
        let side_s       = side_str(pos.side).to_string();

        tokio::task::spawn_blocking(move || {
            let id_to_close = position_id
                .or_else(|| get_open_position(&conn, &strategy, &symbol).map(|p| p.id));
            if let Some(id) = id_to_close {
                close_position(&conn, id)?;
            } else {
                warn!(symbol, strategy, "No live_positions row found to close");
            }
            insert_trade(
                &conn, &strategy, &symbol, &side_s,
                entry_price, exit_price, size, pnl, "closed",
                entry_time, exit_time,
                &entry_reason, &exit_reason,
            )
        }).await??;

        self.balance += pnl;

        let tag = match pos.side {
            PositionSide::Long  => "📉 SELL",
            PositionSide::Short => "📈 COVER",
        };
        info!(
            symbol  = self.symbol,
            entry   = pos.entry_price,
            exit    = exit_price,
            pnl,
            balance = self.balance,
            reason,
            "{tag}"
        );
        Ok(())
    }

    /// Check stop-loss and take-profit on the current candle.
    /// Long: SL on low ≤ sl, TP on high ≥ tp.
    /// Short: SL on high ≥ sl, TP on low ≤ tp.
    async fn check_stops(&mut self, candle: &Candle) -> Result<()> {
        let (hit_sl, hit_tp) = match &self.position {
            None => return Ok(()),
            Some(pos) => match pos.side {
                PositionSide::Long => {
                    let hit_sl = pos.stop_loss  .map(|sl| candle.low  <= sl).unwrap_or(false);
                    let hit_tp = pos.take_profit.map(|tp| candle.high >= tp).unwrap_or(false);
                    (hit_sl, hit_tp)
                }
                PositionSide::Short => {
                    let hit_sl = pos.stop_loss  .map(|sl| candle.high >= sl).unwrap_or(false);
                    let hit_tp = pos.take_profit.map(|tp| candle.low  <= tp).unwrap_or(false);
                    (hit_sl, hit_tp)
                }
            },
        };

        if hit_sl {
            self.close_current_position(candle, "stop-loss triggered").await?;
        } else if hit_tp {
            self.close_current_position(candle, "take-profit triggered").await?;
        }
        Ok(())
    }
}

#[async_trait]
impl OrderExecutor for PaperExecutor {
    async fn handle(&mut self, candle: &Candle, decision: &TradeDecision) -> Result<()> {
        // Check stops first — but only on a position that existed *before*
        // this candle, so a fresh entry can't hit its own wick.
        let had_position_at_start = self.position.is_some();
        if had_position_at_start {
            self.check_stops(candle).await?;
        }

        let current_side = self.position.as_ref().map(|p| p.side);
        let action       = plan_action(&decision.signal, current_side);

        match action {
            Action::OpenLong  => self.open_new_position(PositionSide::Long,  candle, decision).await?,
            Action::OpenShort => self.open_new_position(PositionSide::Short, candle, decision).await?,
            Action::Close     => {
                let reason = decision.reason.clone().unwrap_or_else(|| "strategy close".into());
                self.close_current_position(candle, &reason).await?;
            }
            Action::Nothing   => {
                // HOLD is the common case; only warn on a real mismatch
                // (e.g. SELL while flat, BUY while already short).
                if !matches!(decision.signal, shared::Signal::Hold) {
                    warn!(
                        symbol = self.symbol,
                        signal = ?decision.signal,
                        side   = ?current_side,
                        "Signal does not match current position — ignoring",
                    );
                }
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
