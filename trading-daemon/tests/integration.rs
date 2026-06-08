//! End-to-end integration tests for runtime-backed Paper Trading persistence.
//!
//! These tests drive `trading-runtime` with deterministic Strategy Decisions and
//! project the resulting RuntimeSteps through the daemon Paper Trading
//! Persistence Adapter into the dedicated `paper_open_positions` and
//! `paper_trades` tables. They intentionally avoid the retired legacy
//! `PaperExecutor` / `live_positions` / `live_trades` path.
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
    delete_paper_data_by_strategy_identity, get_paper_open_position, get_paper_trades,
    DbConnection, PaperExitKind, SpacetimeClient,
};
use domain::{Candle, Timeframe};
use trading_daemon::paper_trading_persistence::PaperTradingPersistenceAdapter;
use trading_runtime::{
    MarketInput, PortfolioState, PositionRiskBoundaryChanges, PredeterminedStrategyHandler,
    RuntimeConfig, RuntimeEvent, RuntimeStep, StrategyDecision, TradingRuntime,
};

fn integration_enabled() -> bool {
    std::env::var("SPACETIMEDB_INTEGRATION").as_deref() == Ok("1")
}

fn connect() -> SpacetimeClient {
    SpacetimeClient::connect("http://127.0.0.1:3000", "trading-bot")
        .expect("Failed to connect to SpacetimeDB")
}

const RUNTIME_ASSET: &str = "__DAEMON_PAPER_RUNTIME_ASSET__";
const TIMEFRAME: &str = "1d";
const LONG_STRATEGY_IDENTITY: &str = "__daemon_paper_long_it__";
const FORCE_CLOSE_STRATEGY_IDENTITY: &str = "__daemon_paper_force_close_it__";
const SHORT_STRATEGY_IDENTITY: &str = "__daemon_paper_short_it__";
const RISK_UPDATE_STRATEGY_IDENTITY: &str = "__daemon_paper_risk_update_it__";

fn make_candle(ts: i64, close: f64) -> Candle {
    Candle {
        timestamp: ts,
        symbol: RUNTIME_ASSET.into(),
        open: close - 0.5,
        high: close + 1.0,
        low: close - 1.0,
        close,
        volume: 1000.0,
        timeframe: TIMEFRAME.parse::<Timeframe>().unwrap(),
    }
}

fn runtime(
    balance: f64,
    decisions: impl IntoIterator<Item = StrategyDecision>,
) -> TradingRuntime<PredeterminedStrategyHandler> {
    TradingRuntime::with_config(
        RuntimeConfig::single_timeframe(RUNTIME_ASSET, TIMEFRAME.parse::<Timeframe>().unwrap()),
        PortfolioState::new(balance),
        0,
        PredeterminedStrategyHandler::from_decisions(decisions),
    )
}

fn adapter(
    conn: Arc<DbConnection>,
    strategy_identity: &str,
) -> PaperTradingPersistenceAdapter<Arc<DbConnection>> {
    PaperTradingPersistenceAdapter::new(conn, strategy_identity, RUNTIME_ASSET)
}

fn project_completed_candle(
    runtime: &mut TradingRuntime<PredeterminedStrategyHandler>,
    adapter: &PaperTradingPersistenceAdapter<Arc<DbConnection>>,
    candle: Candle,
) -> RuntimeStep {
    let step = runtime
        .on_market_input(MarketInput::CompletedCandle(candle))
        .expect("runtime should accept configured completed candle");
    adapter
        .project_step(&step)
        .expect("paper projection should be confirmed");
    step
}

fn teardown(conn: &DbConnection, strategy_identity: &str) {
    let _ = delete_paper_data_by_strategy_identity(conn, strategy_identity);
    std::thread::sleep(std::time::Duration::from_millis(200));
}

#[test]
fn runtime_backed_paper_trading_projects_open_hold_close_cycle() {
    if !integration_enabled() {
        eprintln!("skipping (set SPACETIMEDB_INTEGRATION=1)");
        return;
    }

    let client = connect();
    let conn = client.conn.clone();
    teardown(&conn, LONG_STRATEGY_IDENTITY);
    let adapter = adapter(conn.clone(), LONG_STRATEGY_IDENTITY);
    let mut runtime = runtime(
        10_000.0,
        [
            StrategyDecision::open_long(2.0).with_reason("open"),
            StrategyDecision::hold().with_reason("hold"),
            StrategyDecision::close_long().with_reason("close"),
        ],
    );

    let open_step = project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_000_000_000, 100.0),
    );
    assert!(open_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PositionOpened { .. })));
    std::thread::sleep(std::time::Duration::from_millis(200));

    let open = get_paper_open_position(&conn, LONG_STRATEGY_IDENTITY, RUNTIME_ASSET)
        .expect("paper_open_positions row should exist after runtime PositionOpened");
    assert_eq!(open.side, "long");
    assert_eq!(open.entry_price, 100.0);
    assert_eq!(open.quantity, 2.0);
    assert_eq!(open.stop_loss, None);
    assert_eq!(open.take_profit, None);
    assert!(get_paper_trades(&conn, LONG_STRATEGY_IDENTITY, RUNTIME_ASSET).is_empty());

    let hold_step = project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_086_400_000, 105.0),
    );
    assert!(hold_step.events.iter().all(|event| !matches!(
        event,
        RuntimeEvent::PositionOpened { .. } | RuntimeEvent::PositionClosed { .. }
    )));
    assert!(get_paper_trades(&conn, LONG_STRATEGY_IDENTITY, RUNTIME_ASSET).is_empty());

    let close_step = project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_172_800_000, 110.0),
    );
    assert!(close_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PositionClosed { .. })));
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(get_paper_open_position(&conn, LONG_STRATEGY_IDENTITY, RUNTIME_ASSET).is_none());
    let trades = get_paper_trades(&conn, LONG_STRATEGY_IDENTITY, RUNTIME_ASSET);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].side, "long");
    assert_eq!(trades[0].entry_price, 100.0);
    assert_eq!(trades[0].exit_price, 110.0);
    assert_eq!(trades[0].quantity, 2.0);
    assert_eq!(trades[0].realized_pnl, 20.0);
    assert_eq!(trades[0].exit_kind, PaperExitKind::StrategyExit);

    let restored = adapter
        .restore_portfolio_state(10_000.0)
        .expect("paper restore should use projected trade PnL");
    assert_eq!(restored.realized_cash_balance, 10_020.0);
    assert_eq!(restored.completed_trade_count, 1);
    assert!(restored.open_position.is_none());

    teardown(&conn, LONG_STRATEGY_IDENTITY);
}

#[test]
fn runtime_backed_paper_trading_projects_position_risk_update_and_restore_uses_updated_boundaries()
{
    if !integration_enabled() {
        return;
    }

    let client = connect();
    let conn = client.conn.clone();
    teardown(&conn, RISK_UPDATE_STRATEGY_IDENTITY);
    let adapter = adapter(conn.clone(), RISK_UPDATE_STRATEGY_IDENTITY);
    let mut runtime = runtime(
        10_000.0,
        [
            StrategyDecision::open_long(2.0)
                .with_entry_risk(Some(95.0), Some(120.0))
                .with_reason("open"),
            StrategyDecision::update_position_risk()
                .with_position_risk_changes(
                    PositionRiskBoundaryChanges::new()
                        .set_stop_loss(100.0)
                        .clear_take_profit(),
                )
                .with_reason("trail"),
        ],
    );

    project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_000_000_000, 100.0),
    );
    let update_step = project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_086_400_000, 105.0),
    );
    assert!(update_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::PositionRiskUpdateEvaluated { .. })));
    std::thread::sleep(std::time::Duration::from_millis(200));

    let open = get_paper_open_position(&conn, RISK_UPDATE_STRATEGY_IDENTITY, RUNTIME_ASSET)
        .expect("paper_open_positions row should remain after risk update");
    assert_eq!(open.side, "long");
    assert_eq!(open.entry_price, 100.0);
    assert_eq!(open.quantity, 2.0);
    assert_eq!(open.stop_loss, Some(100.0));
    assert_eq!(open.take_profit, None);

    let restored = adapter
        .restore_portfolio_state(10_000.0)
        .expect("paper restore should use projected risk boundaries");
    let restored_position = restored
        .open_position
        .expect("updated open position should restore");
    assert_eq!(restored_position.risk_boundaries.stop_loss, Some(100.0));
    assert_eq!(restored_position.risk_boundaries.take_profit, None);
    assert!(get_paper_trades(&conn, RISK_UPDATE_STRATEGY_IDENTITY, RUNTIME_ASSET).is_empty());

    teardown(&conn, RISK_UPDATE_STRATEGY_IDENTITY);
}

#[test]
fn runtime_backed_force_close_projects_one_paper_trade() {
    if !integration_enabled() {
        return;
    }

    let client = connect();
    let conn = client.conn.clone();
    teardown(&conn, FORCE_CLOSE_STRATEGY_IDENTITY);
    let adapter = adapter(conn.clone(), FORCE_CLOSE_STRATEGY_IDENTITY);
    let mut runtime = runtime(
        5_000.0,
        [StrategyDecision::open_long(1.0).with_reason("pre-shutdown")],
    );

    project_completed_candle(&mut runtime, &adapter, make_candle(1_700_000_000_000, 80.0));

    let force_close_step =
        runtime.force_close(make_candle(1_700_086_400_000, 88.0), "shutdown liquidation");
    adapter
        .project_step(&force_close_step)
        .expect("force-close projection should be confirmed");

    let second_force_close =
        runtime.force_close(make_candle(1_700_086_400_000, 88.0), "second call");
    adapter
        .project_step(&second_force_close)
        .expect("flat force-close step should be a persistence no-op");
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(get_paper_open_position(&conn, FORCE_CLOSE_STRATEGY_IDENTITY, RUNTIME_ASSET).is_none());
    let trades = get_paper_trades(&conn, FORCE_CLOSE_STRATEGY_IDENTITY, RUNTIME_ASSET);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].exit_kind, PaperExitKind::ForceClose);
    assert_eq!(trades[0].entry_price, 80.0);
    assert_eq!(trades[0].exit_price, 88.0);
    assert_eq!(trades[0].quantity, 1.0);
    assert_eq!(trades[0].realized_pnl, 8.0);

    teardown(&conn, FORCE_CLOSE_STRATEGY_IDENTITY);
}

#[test]
fn runtime_backed_short_cover_projects_profit_on_price_drop() {
    if !integration_enabled() {
        return;
    }

    let client = connect();
    let conn = client.conn.clone();
    teardown(&conn, SHORT_STRATEGY_IDENTITY);
    let adapter = adapter(conn.clone(), SHORT_STRATEGY_IDENTITY);
    let mut runtime = runtime(
        4_000.0,
        [
            StrategyDecision::open_short(2.0).with_reason("short entry"),
            StrategyDecision::close_long().with_reason("wrong side"),
            StrategyDecision::close_short().with_reason("cover"),
        ],
    );

    project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_000_000_000, 200.0),
    );
    let ignored_step = project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_086_400_000, 190.0),
    );
    assert!(ignored_step
        .events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::StrategyDecisionIgnored { .. })));
    assert!(get_paper_trades(&conn, SHORT_STRATEGY_IDENTITY, RUNTIME_ASSET).is_empty());

    project_completed_candle(
        &mut runtime,
        &adapter,
        make_candle(1_700_172_800_000, 180.0),
    );
    std::thread::sleep(std::time::Duration::from_millis(200));

    assert!(get_paper_open_position(&conn, SHORT_STRATEGY_IDENTITY, RUNTIME_ASSET).is_none());
    let trades = get_paper_trades(&conn, SHORT_STRATEGY_IDENTITY, RUNTIME_ASSET);
    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].side, "short");
    assert_eq!(trades[0].entry_price, 200.0);
    assert_eq!(trades[0].exit_price, 180.0);
    assert_eq!(trades[0].quantity, 2.0);
    assert_eq!(trades[0].realized_pnl, 40.0);
    assert_eq!(trades[0].exit_kind, PaperExitKind::StrategyExit);

    teardown(&conn, SHORT_STRATEGY_IDENTITY);
}
