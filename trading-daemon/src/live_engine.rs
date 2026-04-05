/// Live engine task: subscribes to new candles via SpacetimeDB on_insert
/// callback, ticks the Rhai engine, and dispatches signals to the executor.
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use db_layer::{module_bindings::CandlesTableAccess, DbConnection, SpacetimeClient};
use engine::Engine;
use shared::{Candle, Context};
use spacetimedb_sdk::Table;

use crate::{
    config::AssetConfig,
    order_executor::{OrderExecutor, PaperExecutor},
    warmup::warmup_engine,
};

/// Run a live engine instance for one asset/interval/strategy combination.
///
/// This function:
/// 1. Loads the strategy and detects warmup period automatically
/// 2. Warms up the engine with historical DB candles
/// 3. Registers an `on_insert` callback on the candles table
/// 4. Processes incoming candles via an mpsc channel
/// 5. Runs until the CancellationToken is cancelled
pub async fn run(
    client:  Arc<SpacetimeClient>,
    asset:   AssetConfig,
    cancel:  CancellationToken,
) -> Result<()> {
    let symbol   = asset.symbol.clone();
    let interval = asset.intervals.first()
        .cloned()
        .unwrap_or_else(|| "1d".into());

    info!(symbol, interval, strategy = asset.strategy, "Starting live engine");

    // ── Load strategy ──────────────────────────────────────────────────────────
    let strategy_src = std::fs::read_to_string(&asset.strategy)
        .map_err(|e| anyhow::anyhow!("Cannot read strategy '{}': {e}", asset.strategy))?;

    // ── Detect warmup period automatically from AST ────────────────────────────
    let mut tmp_engine = Engine::new(&strategy_src)?;
    let warmup_bars = {
        // Build a temporary Rhai engine to compile + get AST/scope
        use rhai::{Engine as RhaiEngine, Scope};
        use engine::{bindings::register_all, candle_wrapper::register_types};
        let mut rhai = RhaiEngine::new();
        register_types(&mut rhai);
        register_all(&mut rhai);
        let ast = rhai.compile(&strategy_src).unwrap_or_default();
        let mut scope = Scope::new();
        let _ = rhai.run_ast_with_scope(&mut scope, &ast);
        engine::detect_warmup_period(&ast, &scope)
    };

    info!(symbol, interval, warmup_bars, "Detected warmup period");

    // ── Warmup engine ──────────────────────────────────────────────────────────
    // conn is Arc<DbConnection> — clone the Arc for shared ownership across tasks.
    let conn: Arc<DbConnection> = client.conn.clone();
    warmup_engine(&conn, &mut tmp_engine, &symbol, &interval, warmup_bars)?;

    // ── Create paper executor ─────────────────────────────────────────────────
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

    let sym_filter  = symbol.clone();
    let tf_filter   = interval.clone();

    conn.db.candles().on_insert(move |_ctx, db_candle| {
        // Only forward candles matching our symbol + interval.
        if db_candle.symbol != sym_filter || db_candle.timeframe != tf_filter {
            return;
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
        if tx.try_send(candle).is_err() {
            // Channel full — this would only happen if processing is very slow.
        }
    });

    info!(symbol, interval, "on_insert callback registered — waiting for new candles");

    // ── Main loop ─────────────────────────────────────────────────────────────
    let mut engine = tmp_engine;

    loop {
        tokio::select! {
            // New candle arrived
            Some(candle) = rx.recv() => {
                info!(
                    symbol  = candle.symbol,
                    ts      = candle.timestamp,
                    close   = candle.close,
                    "New candle"
                );

                // Build context from current executor state.
                let context = build_context(&executor);

                // Tick the engine.
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

            // Shutdown signal
            _ = cancel.cancelled() => {
                info!(symbol, interval, "Live engine shutting down");
                if let Some(pos) = executor.position() {
                    warn!(
                        symbol,
                        entry_price = pos.entry_price,
                        "Open position remains — will be restored on next startup"
                    );
                }
                break;
            }
        }
    }

    Ok(())
}

/// Build a `shared::Context` from the current executor state.
fn build_context(executor: &PaperExecutor) -> Context {
    let balance = executor.balance();
    let position = executor.position().cloned();
    let equity = match &position {
        Some(_) => balance, // simplified: equity = balance for now
        None    => balance,
    };
    Context { balance, equity, position, trades_count: 0 }
}
