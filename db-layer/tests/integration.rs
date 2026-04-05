/// Integration tests for `db-layer` — SDK-based (WebSocket).
///
/// Requires a running SpacetimeDB instance with the `trading-bot` module deployed.
/// Skipped automatically when `SPACETIMEDB_INTEGRATION` is not set to `1`.
///
/// Every test cleans up its own data so the DB stays empty after the suite.
///
/// Run with:
/// ```bash
/// SPACETIMEDB_INTEGRATION=1 cargo test -p db-layer --test integration -- --nocapture
/// ```
use db_layer::{
    close_position, count_candles,
    delete_candles_by_symbol, delete_trades_by_strategy,
    get_candles, get_candles_before, get_open_position, get_trades,
    insert_candle, insert_trade, open_position,
    SpacetimeClient,
};
use shared::Candle;

fn integration_enabled() -> bool {
    std::env::var("SPACETIMEDB_INTEGRATION").as_deref() == Ok("1")
}

fn connect() -> SpacetimeClient {
    SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB")
}

// Unique prefixes so tests don't interfere with real data.
const SYM:   &str = "__TEST_CANDLE__";
const TF:    &str = "1d";
const PROV:  &str = "__test__";
const STRAT: &str = "__test_strat__";

fn make_candle(ts: i64, close: f64) -> Candle {
    Candle {
        timestamp: ts,
        symbol:    SYM.into(),
        open:      close - 0.5,
        high:      close + 1.0,
        low:       close - 1.0,
        close,
        volume:    1000.0,
        timeframe: TF.into(),
    }
}

// ── Candle tests ──────────────────────────────────────────────────────────────

#[test]
fn test_insert_and_fetch_candles() {
    if !integration_enabled() {
        eprintln!("skipping (set SPACETIMEDB_INTEGRATION=1)");
        return;
    }

    let client  = connect();
    let conn    = &*client.conn;
    let ts_base = 1_700_000_000_000_i64;

    // Insert 5 candles.
    for i in 0..5_i64 {
        let c = make_candle(ts_base + i * 86_400_000, 100.0 + i as f64);
        insert_candle(conn, &c, PROV).unwrap();
    }

    // Give the server a moment to process and push back the inserts.
    std::thread::sleep(std::time::Duration::from_millis(200));

    // Idempotent re-insert must not duplicate.
    insert_candle(conn, &make_candle(ts_base, 100.0), PROV).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));

    let count = count_candles(conn, SYM, TF);
    assert!(count >= 5, "expected ≥5 candles, got {count}");

    let candles = get_candles(conn, SYM, TF, 3);
    assert_eq!(candles.len(), 3);
    assert!(candles[0].timestamp <= candles[1].timestamp, "not chronological");

    // ── teardown ──
    delete_candles_by_symbol(conn, SYM, PROV).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert_eq!(count_candles(conn, SYM, TF), 0, "teardown left rows");
}

#[test]
fn test_get_candles_before() {
    if !integration_enabled() { return; }

    let client  = connect();
    let conn    = &*client.conn;
    let ts_base = 1_700_000_000_000_i64;
    let cutoff  = ts_base + 3 * 86_400_000;

    for i in 0..5_i64 {
        let c = make_candle(ts_base + i * 86_400_000, 100.0 + i as f64);
        insert_candle(conn, &c, PROV).unwrap();
    }
    std::thread::sleep(std::time::Duration::from_millis(200));

    let candles = get_candles_before(conn, SYM, TF, cutoff, 10);
    for c in &candles {
        assert!(c.timestamp < cutoff, "ts {} ≥ cutoff {cutoff}", c.timestamp);
    }
    for w in candles.windows(2) {
        assert!(w[0].timestamp <= w[1].timestamp, "not chronological");
    }

    // ── teardown ──
    delete_candles_by_symbol(conn, SYM, PROV).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(100));
}

// ── Position tests ────────────────────────────────────────────────────────────

#[test]
fn test_position_lifecycle() {
    if !integration_enabled() { return; }

    let client = connect();
    let conn   = &*client.conn;
    let symbol = "__TEST_POS__";

    open_position(conn, STRAT, symbol, "long",
        100.0, 1.0, 95.0, 115.0,
        1_700_000_000_000, "integration test",
    ).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let pos = get_open_position(conn, STRAT, symbol)
        .expect("position should exist");
    assert_eq!(pos.symbol, symbol);
    assert_eq!(pos.side, "long");
    assert!((pos.entry_price - 100.0).abs() < f64::EPSILON);

    // close_position is the teardown (deletes the row).
    close_position(conn, pos.id).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(
        get_open_position(conn, STRAT, symbol).is_none(),
        "position should be gone after close"
    );
}

// ── Trade tests ───────────────────────────────────────────────────────────────

#[test]
fn test_insert_and_fetch_trade() {
    if !integration_enabled() { return; }

    let client = connect();
    let conn   = &*client.conn;

    insert_trade(conn, STRAT, "__TEST_TRADE__", "long",
        100.0, 110.0, 1.0, 10.0, "closed",
        1_700_000_000_000, 1_700_086_400_000,
        "integration buy", "integration sell",
    ).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));

    let trades = get_trades(conn, STRAT, 10);
    assert!(!trades.is_empty());
    assert!((trades[0].pnl - 10.0).abs() < f64::EPSILON);

    // ── teardown ──
    delete_trades_by_strategy(conn, STRAT).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(get_trades(conn, STRAT, 10).is_empty(), "teardown left trades");
}

// ── Unit tests (no DB required) ───────────────────────────────────────────────

#[test]
fn canonical_id_is_deterministic() {
    use db_layer::canonical_id;
    let a = canonical_id("AAPL", "1d", 1_700_000_000_000);
    let b = canonical_id("AAPL", "1d", 1_700_000_000_000);
    assert_eq!(a, b);
    assert_eq!(a, "AAPL_1d_1700000000000");
}

#[test]
fn db_candle_converts_to_shared() {
    use db_layer::{db_candle_to_shared, DbCandle};
    let db = DbCandle {
        id:           1,
        canonical_id: "AAPL_1d_1700000000000".into(),
        timestamp:    1_700_000_000_000,
        symbol:       "AAPL".into(),
        open:         149.5,
        high:         151.0,
        low:          149.0,
        close:        150.0,
        volume:       1_000_000.0,
        timeframe:    "1d".into(),
        provider:     "yahoo".into(),
    };
    let shared = db_candle_to_shared(db);
    assert_eq!(shared.symbol, "AAPL");
    assert!((shared.close - 150.0).abs() < f64::EPSILON);
}

#[test]
fn db_position_converts_to_shared() {
    use db_layer::{db_position_to_shared, LivePosition};
    use shared::PositionSide;

    let db = LivePosition {
        id:           1,
        strategy:     "sma_cross".into(),
        symbol:       "AAPL".into(),
        side:         "long".into(),
        entry_price:  100.0,
        size:         5.0,
        stop_loss:    95.0,
        take_profit:  115.0,
        entry_time:   1_700_000_000_000,
        entry_reason: "test".into(),
    };
    let (id, strategy, pos) = db_position_to_shared(db);
    assert_eq!(id, 1);
    assert_eq!(strategy, "sma_cross");
    assert_eq!(pos.side, PositionSide::Long);
    assert!((pos.entry_price - 100.0).abs() < f64::EPSILON);
    assert_eq!(pos.stop_loss, Some(95.0));
}
