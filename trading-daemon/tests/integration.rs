//! End-to-end integration test for the trading daemon's paper executor.
//!
//! Drives a `PaperExecutor` against a real SpacetimeDB instance through a
//! BUY → (hold) → SELL sequence, then asserts:
//!
//! 1. After BUY: a `live_positions` row is visible and the executor captured
//!    its `id` (proves §1.2 `wait_for_open_position` works, no orphan row).
//! 2. After SELL: a `live_trades` row exists with the expected PnL and the
//!    `live_positions` row is gone (proves §1.2 close path + §1.3 balance update).
//!
//! Requires a running SpacetimeDB with the `trading-bot` module deployed.
//! Skipped automatically unless `SPACETIMEDB_INTEGRATION=1`.
//!
//! Run with:
//! ```bash
//! SPACETIMEDB_INTEGRATION=1 cargo test -p trading-daemon --test integration -- --nocapture
//! ```
use std::sync::Arc;

use db_layer::{
    count_trades, delete_trades_by_strategy, get_open_position, get_trades, SpacetimeClient,
};
use shared::{Candle, Signal, TradeDecision};
use trading_daemon::order_executor::{OrderExecutor, PaperExecutor};

fn integration_enabled() -> bool {
    std::env::var("SPACETIMEDB_INTEGRATION").as_deref() == Ok("1")
}

const STRAT: &str = "__daemon_it_strat__";
const SYMBOL: &str = "__DAEMON_IT__";

fn make_candle(ts: i64, close: f64) -> Candle {
    Candle {
        timestamp: ts,
        symbol: SYMBOL.into(),
        open: close - 0.5,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1000.0,
        timeframe: "1d".into(),
    }
}

fn buy(reason: &str) -> TradeDecision {
    TradeDecision {
        signal: Signal::Buy,
        size: 1.0,
        stop_loss: None,
        take_profit: None,
        reason: Some(reason.into()),
    }
}

fn sell(reason: &str) -> TradeDecision {
    TradeDecision {
        signal: Signal::Sell,
        size: 0.0,
        stop_loss: None,
        take_profit: None,
        reason: Some(reason.into()),
    }
}

fn short(reason: &str) -> TradeDecision {
    TradeDecision {
        signal: Signal::Short,
        size: 1.0,
        stop_loss: None,
        take_profit: None,
        reason: Some(reason.into()),
    }
}

fn cover(reason: &str) -> TradeDecision {
    TradeDecision {
        signal: Signal::Cover,
        size: 0.0,
        stop_loss: None,
        take_profit: None,
        reason: Some(reason.into()),
    }
}

fn hold() -> TradeDecision {
    TradeDecision::hold()
}

/// Teardown: nuke any leftover state for our test strategy/symbol.
fn teardown(conn: &db_layer::DbConnection) {
    if let Some(p) = get_open_position(conn, STRAT, SYMBOL) {
        let _ = db_layer::close_position(conn, p.id);
    }
    let _ = delete_trades_by_strategy(conn, STRAT);
    std::thread::sleep(std::time::Duration::from_millis(200));
}

#[tokio::test(flavor = "current_thread")]
async fn paper_executor_full_buy_sell_cycle() {
    if !integration_enabled() {
        eprintln!("skipping (set SPACETIMEDB_INTEGRATION=1)");
        return;
    }

    let client = SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB");
    let conn: Arc<db_layer::DbConnection> = client.conn.clone();

    // Clean slate.
    teardown(&conn);
    let trades_before = count_trades(&conn, STRAT, SYMBOL);

    // ── Executor under test ───────────────────────────────────────────────────
    let mut executor = PaperExecutor::new(
        conn.clone(),
        STRAT.to_string(),
        SYMBOL.to_string(),
        10_000.0,
    );

    assert!(
        executor.position().is_none(),
        "fresh executor should be flat"
    );
    let start_balance = executor.balance();

    // ── Candle 1: BUY @ 100 ───────────────────────────────────────────────────
    let c1 = make_candle(1_700_000_000_000, 100.0);
    executor
        .handle(&c1, &buy("open"))
        .await
        .expect("BUY failed");

    // Position must be set locally.
    let pos = executor.position().cloned().expect("position after BUY");
    assert_eq!(pos.entry_price, 100.0);
    assert!(pos.size > 0.0);

    // §1.2 proof: live_positions row must be visible in cache.
    let db_pos =
        get_open_position(&conn, STRAT, SYMBOL).expect("live_positions row should exist after BUY");
    assert_eq!(db_pos.side, "long");
    assert!((db_pos.entry_price - 100.0).abs() < f64::EPSILON);

    // ── Candle 2: HOLD @ 105 (no-op) ──────────────────────────────────────────
    let c2 = make_candle(1_700_086_400_000, 105.0);
    executor.handle(&c2, &hold()).await.expect("HOLD failed");
    assert!(
        executor.position().is_some(),
        "position should survive HOLD"
    );

    // ── Candle 3: SELL @ 110 ──────────────────────────────────────────────────
    let c3 = make_candle(1_700_172_800_000, 110.0);
    executor
        .handle(&c3, &sell("close"))
        .await
        .expect("SELL failed");

    assert!(
        executor.position().is_none(),
        "position should be cleared after SELL"
    );

    // live_positions must be empty — proves §1.2 close path did its job.
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        get_open_position(&conn, STRAT, SYMBOL).is_none(),
        "live_positions row should be gone after SELL"
    );

    // live_trades must have +1 row with PnL = (110 - 100) * size.
    let trades_after = count_trades(&conn, STRAT, SYMBOL);
    assert_eq!(
        trades_after,
        trades_before + 1,
        "expected exactly one new trade row"
    );

    let trades = get_trades(&conn, STRAT, 10);
    let t = trades
        .iter()
        .find(|t| t.symbol == SYMBOL)
        .expect("trade for test symbol");
    let expected_pnl = (110.0 - 100.0) * pos.size;
    assert!(
        (t.pnl - expected_pnl).abs() < 1e-6,
        "pnl mismatch: got {}, expected {expected_pnl}",
        t.pnl,
    );
    assert_eq!(t.exit_price, 110.0);
    assert_eq!(t.entry_price, 100.0);

    // §1.3 proof: balance must reflect realized PnL.
    let expected_balance = start_balance + expected_pnl;
    assert!(
        (executor.balance() - expected_balance).abs() < 1e-6,
        "balance not updated: got {}, expected {expected_balance}",
        executor.balance(),
    );

    teardown(&conn);
}

/// §3.8 proof: `PaperExecutor::liquidate` force-closes the open position at
/// the supplied mark price and records a single `live_trades` row.
#[tokio::test(flavor = "current_thread")]
async fn shutdown_liquidation_closes_open_position() {
    if !integration_enabled() {
        return;
    }

    let client = SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB");
    let conn: Arc<db_layer::DbConnection> = client.conn.clone();

    teardown(&conn);
    let trades_before = count_trades(&conn, STRAT, SYMBOL);

    let mut executor =
        PaperExecutor::new(conn.clone(), STRAT.to_string(), SYMBOL.to_string(), 5_000.0);

    // Open a position, then liquidate at a different price (simulating shutdown).
    let entry = make_candle(1_700_000_000_000, 80.0);
    executor
        .handle(&entry, &buy("pre-shutdown"))
        .await
        .expect("BUY");
    assert!(executor.position().is_some());

    let mark = make_candle(1_700_086_400_000, 88.0);
    executor
        .liquidate(&mark, "shutdown liquidation")
        .await
        .expect("liquidate");

    assert!(
        executor.position().is_none(),
        "position must be flat after liquidate"
    );

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        get_open_position(&conn, STRAT, SYMBOL).is_none(),
        "live_positions row must be gone after liquidation",
    );
    assert_eq!(
        count_trades(&conn, STRAT, SYMBOL),
        trades_before + 1,
        "liquidation should record exactly one trade",
    );

    // Idempotency: calling liquidate again on a flat executor is a no-op.
    executor
        .liquidate(&mark, "second call")
        .await
        .expect("noop liquidate");
    std::thread::sleep(std::time::Duration::from_millis(100));
    assert_eq!(
        count_trades(&conn, STRAT, SYMBOL),
        trades_before + 1,
        "second liquidate must not insert another trade",
    );

    teardown(&conn);
}

/// §3.5 proof: SHORT opens a short position, COVER closes it, PnL is
/// `(entry - exit) * size` (price drop → profit).
#[tokio::test(flavor = "current_thread")]
async fn short_cover_cycle_profits_on_price_drop() {
    if !integration_enabled() {
        return;
    }

    let client = SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB");
    let conn: Arc<db_layer::DbConnection> = client.conn.clone();

    teardown(&conn);
    let trades_before = count_trades(&conn, STRAT, SYMBOL);
    let start_balance = 4_000.0;

    let mut executor = PaperExecutor::new(
        conn.clone(),
        STRAT.to_string(),
        SYMBOL.to_string(),
        start_balance,
    );

    // Enter short at 200, cover at 180 → profit.
    let entry = make_candle(1_700_000_000_000, 200.0);
    executor
        .handle(&entry, &short("short entry"))
        .await
        .expect("SHORT");

    let pos = executor.position().cloned().expect("position after SHORT");
    assert_eq!(pos.side, shared::PositionSide::Short);
    assert!(pos.size > 0.0);

    let db_pos = get_open_position(&conn, STRAT, SYMBOL).expect("live_positions row");
    assert_eq!(db_pos.side, "short");

    // A stray SELL while short must be a no-op.
    let mid = make_candle(1_700_086_400_000, 190.0);
    executor
        .handle(&mid, &sell("stray sell"))
        .await
        .expect("SELL no-op");
    assert!(
        executor.position().is_some(),
        "SELL must not close a short position"
    );

    let exit = make_candle(1_700_172_800_000, 180.0);
    executor
        .handle(&exit, &cover("cover"))
        .await
        .expect("COVER");
    assert!(executor.position().is_none(), "COVER must flatten");

    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(get_open_position(&conn, STRAT, SYMBOL).is_none());

    let trades = get_trades(&conn, STRAT, 10);
    let t = trades
        .iter()
        .find(|t| t.symbol == SYMBOL && t.side == "short")
        .expect("short trade row");
    let expected_pnl = (200.0 - 180.0) * pos.size;
    assert!(
        (t.pnl - expected_pnl).abs() < 1e-6,
        "short pnl mismatch: got {}, expected {expected_pnl}",
        t.pnl,
    );
    assert_eq!(count_trades(&conn, STRAT, SYMBOL), trades_before + 1,);
    assert!(
        (executor.balance() - (start_balance + expected_pnl)).abs() < 1e-6,
        "balance not updated on cover",
    );

    teardown(&conn);
}

/// §1.2 narrow test: immediately after `open_position` the executor must have
/// captured a non-`None` position id (via `wait_for_open_position` polling).
#[tokio::test(flavor = "current_thread")]
async fn open_position_id_is_captured_before_close() {
    if !integration_enabled() {
        return;
    }

    let client = SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB");
    let conn: Arc<db_layer::DbConnection> = client.conn.clone();

    teardown(&conn);

    let mut executor =
        PaperExecutor::new(conn.clone(), STRAT.to_string(), SYMBOL.to_string(), 1_000.0);

    let c = make_candle(1_700_000_000_000, 50.0);
    executor.handle(&c, &buy("race probe")).await.expect("BUY");

    // If the race in §1.2 regressed, the row-id poll would time out and the
    // subsequent close would leave an orphaned live_positions row.
    let pos = get_open_position(&conn, STRAT, SYMBOL);
    assert!(pos.is_some(), "live_positions row must be visible");

    // Close it and confirm no orphan.
    let c2 = make_candle(1_700_086_400_000, 55.0);
    executor
        .handle(&c2, &sell("race probe close"))
        .await
        .expect("SELL");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        get_open_position(&conn, STRAT, SYMBOL).is_none(),
        "orphan live_positions row after SELL — §1.2 regression",
    );

    teardown(&conn);
}
