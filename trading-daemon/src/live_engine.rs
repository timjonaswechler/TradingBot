/// Live engine task: subscribes to new candles via SpacetimeDB on_insert
/// callback, ticks the Rhai engine, and dispatches signals to the executor.
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use db_layer::{count_trades, module_bindings::CandlesTableAccess, DbConnection, SpacetimeClient};
use engine::Engine;
use shared::{Candle, Context, Position};
use spacetimedb_sdk::Table;

use crate::{
    config::AssetConfig,
    order_executor::{OrderExecutor, PaperExecutor},
    warmup::warmup_engine,
};

/// Run a live engine instance for one asset/interval/strategy combination.
pub async fn run(
    client:   Arc<SpacetimeClient>,
    asset:    AssetConfig,
    interval: String,
    cancel:   CancellationToken,
) -> Result<()> {
    let symbol = asset.symbol.clone();

    info!(symbol, interval, strategy = asset.strategy, "Starting live engine");

    // ── Load strategy ──────────────────────────────────────────────────────────
    let strategy_src = std::fs::read_to_string(&asset.strategy)
        .map_err(|e| anyhow::anyhow!("Cannot read strategy '{}': {e}", asset.strategy))?;

    // ── Build engine once and reuse its AST/scope for warmup detection ─────────
    let mut engine = Engine::new(&strategy_src)?;
    let warmup_bars = engine::detect_warmup_period(engine.ast(), engine.scope());

    info!(symbol, interval, warmup_bars, "Detected warmup period");

    // ── Warmup engine ──────────────────────────────────────────────────────────
    let conn: Arc<DbConnection> = client.conn.clone();
    let warmup = warmup_engine(&conn, &mut engine, &symbol, &interval, warmup_bars)?;
    let warmup_high_water = warmup.high_water_ts;

    // ── Create paper executor (restores open position from DB cache) ───────────
    let mut executor = PaperExecutor::new(
        conn.clone(),
        asset.strategy.clone(),
        symbol.clone(),
        asset.balance,
    );

    info!(
        symbol, interval,
        balance = asset.balance,
        "Paper executor ready"
    );

    // ── Register on_insert callback ────────────────────────────────────────────
    // The SDK callback runs in the SDK thread — we bridge to Tokio via mpsc.
    let (tx, mut rx) = mpsc::channel::<Candle>(64);

    let sym_filter = symbol.clone();
    let tf_filter  = interval.clone();

    conn.db.candles().on_insert(move |_ctx, db_candle| {
        if db_candle.symbol != sym_filter || db_candle.timeframe != tf_filter {
            return;
        }
        // Drop any candle the engine already saw during warmup.
        if let Some(hw) = warmup_high_water {
            if db_candle.timestamp <= hw {
                return;
            }
        }
        let candle = Candle {
            timestamp: db_candle.timestamp,
            symbol:    db_candle.symbol.clone(),
            open:      db_candle.open,
            high:      db_candle.high,
            low:       db_candle.low,
            close:     db_candle.close,
            volume:    db_candle.volume,
            timeframe: db_candle.timeframe.clone(),
        };
        if let Err(e) = tx.try_send(candle) {
            warn!(error = %e, "Dropped candle — engine channel full");
        }
    });

    info!(symbol, interval, "on_insert callback registered — waiting for new candles");

    // ── Main loop ─────────────────────────────────────────────────────────────
    // Track the most recent candle so shutdown-liquidation has a mark price.
    let mut last_candle: Option<Candle> = None;

    loop {
        tokio::select! {
            Some(candle) = rx.recv() => {
                last_candle = Some(candle.clone());
                info!(
                    symbol = candle.symbol,
                    ts     = candle.timestamp,
                    close  = candle.close,
                    "New candle"
                );

                let context = build_context(
                    &executor,
                    candle.close,
                    &conn,
                    &asset.strategy,
                    &symbol,
                );

                match engine.tick(candle.clone(), context) {
                    Ok(decision) => {
                        info!(signal = ?decision.signal, "Strategy signal");
                        if let Err(e) = executor.handle(&candle, &decision).await {
                            error!(error = %e, "Order executor error");
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Engine tick error");
                    }
                }
            }

            _ = cancel.cancelled() => {
                info!(symbol, interval, "Live engine shutting down");
                if executor.position().is_some() {
                    if asset.liquidate_on_shutdown {
                        // Prefer the freshest observed candle; if none came in
                        // this session, fall back to the newest DB bar.
                        let mark = last_candle.clone().or_else(|| {
                            db_layer::get_candles_before(&conn, &symbol, &interval, i64::MAX, 1)
                                .into_iter()
                                .next_back()
                        });
                        match mark {
                            Some(c) => {
                                info!(symbol, mark_price = c.close, "Liquidating open position");
                                if let Err(e) = executor
                                    .liquidate(&c, "shutdown liquidation")
                                    .await
                                {
                                    error!(error = %e, "Shutdown liquidation failed");
                                }
                            }
                            None => warn!(
                                symbol,
                                "No mark price available — leaving position open for restore"
                            ),
                        }
                    } else if let Some(pos) = executor.position() {
                        warn!(
                            symbol,
                            entry_price = pos.entry_price,
                            "Open position remains — will be restored on next startup"
                        );
                    }
                }
                break;
            }
        }
    }

    Ok(())
}

/// Build a `shared::Context` reflecting current balance, mark-to-market equity,
/// open position, and realized trade count.
fn build_context(
    executor: &PaperExecutor,
    last_close: f64,
    conn: &DbConnection,
    strategy: &str,
    symbol: &str,
) -> Context {
    let balance  = executor.balance();
    let position = executor.position().cloned();
    let equity   = balance + unrealized_pnl(position.as_ref(), last_close);
    let trades_count = count_trades(conn, strategy, symbol) as u32;
    Context { balance, equity, position, trades_count }
}

fn unrealized_pnl(position: Option<&Position>, last_close: f64) -> f64 {
    match position {
        Some(p) => match p.side {
            shared::PositionSide::Long  => (last_close - p.entry_price) * p.size,
            shared::PositionSide::Short => (p.entry_price - last_close) * p.size,
        },
        None => 0.0,
    }
}
