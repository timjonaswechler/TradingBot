/// Live runner task: subscribes to completed candles via SpacetimeDB on_insert
/// callbacks, feeds one Trading Runtime for a Runtime Asset, and logs runtime
/// events. Portfolio transitions and execution semantics are owned by
/// `trading-runtime`; this module only bridges live IO into runtime input.
use anyhow::{anyhow, bail, Result};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use db_layer::{
    get_candles_before, module_bindings::CandlesTableAccess, DbConnection, SpacetimeClient,
};
use domain::{Candle, Timeframe};
use spacetimedb_sdk::Table;
use trading_runtime::{
    resolve_warmup_plan, MarketInput, PortfolioState, RhaiStrategy, RuntimeConfig,
    RuntimeInputError, RuntimePortfolioSnapshot, RuntimeStep, TradingRuntime,
};

use crate::{
    config::{AssetConfig, LiveExecutionMode},
    paper_trading_persistence::{PaperTradingPersistenceAdapter, PaperTradingPersistenceStore},
    protective_shutdown::{ProtectiveShutdownPolicy, ProtectiveShutdownTrigger},
};

const PROTECTIVE_RUNNER_SHUTDOWN_REASON: &str = "protective runner shutdown";

/// Live runtime wrapper for one configured Runtime Asset.
struct LiveRuntimeAsset {
    config: RuntimeConfig,
    runtime: TradingRuntime<RhaiStrategy>,
}

impl LiveRuntimeAsset {
    fn from_strategy_source(
        asset: &AssetConfig,
        strategy_source: &str,
        initial_portfolio: PortfolioState,
    ) -> Result<Self> {
        let strategy = RhaiStrategy::load(strategy_source)?;
        let config =
            RuntimeConfig::from_strategy_config(asset.symbol.clone(), strategy.strategy_config())?;
        let warmup_plan = resolve_warmup_plan(
            &config,
            strategy.strategy_config(),
            strategy.ast(),
            strategy.scope(),
            0,
        );
        let runtime = TradingRuntime::with_warmup_plan(
            config.clone(),
            initial_portfolio,
            warmup_plan,
            strategy,
        );

        Ok(Self { config, runtime })
    }

    fn primary_timeframe(&self) -> Timeframe {
        self.config.primary_timeframe
    }

    fn configured_timeframes(&self) -> Vec<Timeframe> {
        let mut timeframes = Vec::with_capacity(1 + self.config.secondary_timeframes.len());
        timeframes.push(self.config.primary_timeframe);
        timeframes.extend(
            self.config
                .secondary_timeframes
                .iter()
                .map(|secondary| secondary.timeframe),
        );
        timeframes
    }

    fn warmup_requirement(&self) -> usize {
        self.runtime.warmup_requirement()
    }

    fn on_warmup_candle(&mut self, candle: Candle) -> Result<RuntimeStep> {
        self.runtime
            .on_market_input(MarketInput::WarmupCandle(candle))
            .map_err(runtime_input_error)
    }

    fn on_completed_candle(&mut self, candle: Candle) -> Result<RuntimeStep> {
        self.runtime
            .on_market_input(MarketInput::CompletedCandle(candle))
            .map_err(runtime_input_error)
    }

    fn force_close(&mut self, mark_candle: Candle, reason: impl Into<String>) -> RuntimeStep {
        self.runtime.force_close(mark_candle, reason)
    }
}

/// Paper Trading Live Runner session for one Strategy Identity × Runtime Asset.
///
/// This adapter-owned wrapper keeps the Trading Runtime mode-free: the runtime is
/// built from a restored `PortfolioState`, and every active `RuntimeStep` is
/// synchronously projected by the daemon before another Tradable Candle can be
/// accepted for this session.
struct PaperLiveRuntimeSession<S> {
    runtime_asset: LiveRuntimeAsset,
    persistence: PaperTradingPersistenceAdapter<S>,
    stopped_after_persistence_failure: bool,
}

impl<S: PaperTradingPersistenceStore> PaperLiveRuntimeSession<S> {
    fn from_strategy_source(asset: AssetConfig, strategy_source: &str, store: S) -> Result<Self> {
        ensure_supported_execution_mode(&asset)?;
        let strategy_identity = required_paper_strategy_identity(&asset)?;
        let persistence =
            PaperTradingPersistenceAdapter::new(store, strategy_identity, asset.symbol.clone());
        let initial_portfolio = persistence
            .restore_portfolio_state(asset.balance)
            .map_err(|error| {
                anyhow!(
                    "failed to restore Paper Trading portfolio for strategy_identity '{}' and runtime_asset '{}': {error}",
                    persistence.strategy_identity(),
                    persistence.runtime_asset()
                )
            })?;
        let runtime_asset =
            LiveRuntimeAsset::from_strategy_source(&asset, strategy_source, initial_portfolio)?;

        Ok(Self {
            runtime_asset,
            persistence,
            stopped_after_persistence_failure: false,
        })
    }

    fn from_strategy_file(asset: AssetConfig, store: S) -> Result<Self> {
        ensure_supported_execution_mode(&asset)?;
        let strategy_source = std::fs::read_to_string(&asset.strategy)
            .map_err(|e| anyhow!("Cannot read strategy '{}': {e}", asset.strategy))?;
        Self::from_strategy_source(asset, &strategy_source, store)
    }

    fn strategy_identity(&self) -> &str {
        self.persistence.strategy_identity()
    }

    fn runtime_asset(&self) -> &str {
        self.persistence.runtime_asset()
    }

    fn primary_timeframe(&self) -> Timeframe {
        self.runtime_asset.primary_timeframe()
    }

    fn configured_timeframes(&self) -> Vec<Timeframe> {
        self.runtime_asset.configured_timeframes()
    }

    fn warmup_requirement(&self) -> usize {
        self.runtime_asset.warmup_requirement()
    }

    fn on_warmup_candle(&mut self, candle: Candle) -> Result<RuntimeStep> {
        self.ensure_running()?;
        self.runtime_asset.on_warmup_candle(candle)
    }

    fn on_completed_candle(&mut self, candle: Candle) -> Result<RuntimeStep> {
        self.ensure_running()?;
        let step = self.runtime_asset.on_completed_candle(candle)?;
        self.project_step_or_stop(&step, "completed candle")?;
        Ok(step)
    }

    fn force_close(
        &mut self,
        mark_candle: Candle,
        reason: impl Into<String>,
    ) -> Result<RuntimeStep> {
        self.ensure_running()?;
        let step = self.runtime_asset.force_close(mark_candle, reason);
        self.project_step_or_stop(&step, "force close")?;
        Ok(step)
    }

    fn ensure_running(&self) -> Result<()> {
        if self.stopped_after_persistence_failure {
            bail!(
                "Paper Live Runner for strategy_identity '{}' and runtime_asset '{}' stopped after a Paper Trading persistence failure; no further Tradable Candles will be processed",
                self.strategy_identity(),
                self.runtime_asset()
            );
        }
        Ok(())
    }

    fn project_step_or_stop(&mut self, step: &RuntimeStep, operation: &str) -> Result<()> {
        if let Err(error) = self.persistence.project_step(step) {
            self.stopped_after_persistence_failure = true;
            bail!(
                "Paper Trading persistence projection failed during {operation} for strategy_identity '{}' and runtime_asset '{}': {error}",
                self.strategy_identity(),
                self.runtime_asset()
            );
        }
        Ok(())
    }
}

fn ensure_supported_execution_mode(asset: &AssetConfig) -> Result<()> {
    match asset.execution_mode {
        LiveExecutionMode::PaperTrading => Ok(()),
        LiveExecutionMode::RealMoney => bail!(
            "real-money live execution is not yet supported for asset '{}'; runtime PositionOpened/PositionClosed events are not projected as broker truth. Configure execution_mode = \"paper_trading\" for simulated Paper Trading.",
            asset.symbol
        ),
    }
}

fn required_paper_strategy_identity(asset: &AssetConfig) -> Result<String> {
    asset.strategy_identity
        .as_deref()
        .map(str::trim)
        .filter(|identity| !identity.is_empty())
        .map(str::to_string)
        .ok_or_else(|| {
            anyhow!(
                "persistent Paper Trading for asset '{}' requires an explicit non-empty strategy_identity",
                asset.symbol
            )
        })
}

/// Run a live runtime instance for one configured asset. The strategy file owns
/// the Primary Timeframe and any Secondary Timeframes; daemon config binds that
/// contract to the Runtime Asset and live runner policies.
pub async fn run(
    client: Arc<SpacetimeClient>,
    asset: AssetConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let symbol = asset.symbol.clone();

    info!(
        symbol,
        strategy = asset.strategy,
        execution_mode = %asset.execution_mode,
        "Starting live runtime"
    );

    let conn: Arc<DbConnection> = client.conn.clone();
    let mut runtime_session =
        PaperLiveRuntimeSession::from_strategy_file(asset.clone(), conn.clone())?;
    let primary_timeframe = runtime_session.primary_timeframe();
    let configured_timeframes = runtime_session.configured_timeframes();

    info!(
        symbol,
        strategy_identity = runtime_session.strategy_identity(),
        execution_mode = %asset.execution_mode,
        primary_timeframe = %primary_timeframe,
        secondary_timeframes = ?runtime_session.runtime_asset.config.secondary_timeframes,
        warmup_requirement = runtime_session.warmup_requirement(),
        protective_shutdown_enabled = asset.protective_shutdown.enabled,
        protective_shutdown_threshold = asset.protective_shutdown.required_secondary_failure_threshold,
        "Live runtime configured"
    );

    let mut protective_shutdown =
        ProtectiveShutdownPolicy::new(symbol.clone(), primary_timeframe, asset.protective_shutdown);

    let warmup_requirement = runtime_session.warmup_requirement();
    let warmup_high_water =
        warmup_runtime_asset(&conn, &mut runtime_session, &symbol, warmup_requirement)?;

    // The SDK callback runs in the SDK thread — bridge to Tokio via mpsc.
    let (tx, mut rx) = mpsc::channel::<Candle>(64);

    let sym_filter = symbol.clone();
    let timeframe_filters: HashSet<String> = configured_timeframes
        .iter()
        .map(ToString::to_string)
        .collect();

    conn.db.candles().on_insert(move |_ctx, db_candle| {
        if db_candle.symbol != sym_filter || !timeframe_filters.contains(&db_candle.timeframe) {
            return;
        }

        let timeframe = match db_candle.timeframe.parse::<Timeframe>() {
            Ok(timeframe) => timeframe,
            Err(error) => {
                warn!(timeframe = db_candle.timeframe, error = %error, "Dropped candle with invalid timeframe");
                return;
            }
        };

        // Drop any candle the runtime already saw during warmup.
        if let Some(hw) = warmup_high_water.get(&timeframe) {
            if db_candle.timestamp <= *hw {
                return;
            }
        }

        let candle = Candle {
            timestamp: db_candle.timestamp,
            symbol: db_candle.symbol.clone(),
            open: db_candle.open,
            high: db_candle.high,
            low: db_candle.low,
            close: db_candle.close,
            volume: db_candle.volume,
            timeframe,
        };
        if let Err(e) = tx.try_send(candle) {
            warn!(error = %e, "Dropped candle — runtime channel full");
        }
    });

    info!(
        symbol,
        primary_timeframe = %primary_timeframe,
        "on_insert callback registered — waiting for new candles"
    );

    // Track the most recent Primary candle so optional shutdown liquidation has
    // a runtime mark price without treating Secondary context as executable.
    let mut last_primary_candle: Option<Candle> = None;

    loop {
        tokio::select! {
            Some(candle) = rx.recv() => {
                if candle.timeframe == primary_timeframe {
                    last_primary_candle = Some(candle.clone());
                }
                info!(
                    symbol = candle.symbol,
                    timeframe = %candle.timeframe,
                    ts = candle.timestamp,
                    close = candle.close,
                    "Completed candle"
                );

                match runtime_session.on_completed_candle(candle) {
                    Ok(step) => {
                        info!(events = ?step.events, snapshot = ?step.portfolio_snapshot, "Runtime step completed and Paper Trading projection confirmed");
                        if let Some(trigger) = protective_shutdown.observe_step(&step) {
                            warn!(
                                symbol,
                                primary_timeframe = %primary_timeframe,
                                threshold = trigger.threshold,
                                blocked_contexts = ?trigger.blocked_contexts,
                                counters = ?trigger.counters,
                                "Protective Runner Shutdown triggered"
                            );

                            let mark = latest_primary_mark_candle(
                                &conn,
                                &symbol,
                                primary_timeframe,
                                last_primary_candle.clone(),
                            );
                            if let Some(force_close_step) = complete_protective_shutdown(
                                &mut runtime_session,
                                &step.portfolio_snapshot,
                                mark,
                                &trigger,
                            )? {
                                info!(
                                    events = ?force_close_step.events,
                                    snapshot = ?force_close_step.portfolio_snapshot,
                                    "Protective Runner Shutdown force-close request completed and Paper Trading projection confirmed"
                                );
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Paper live runtime step failed");
                        return Err(e);
                    }
                }
            }

            _ = cancel.cancelled() => {
                info!(symbol, primary_timeframe = %primary_timeframe, "Live runtime shutting down");
                if asset.liquidate_on_shutdown {
                    let mark = latest_primary_mark_candle(
                        &conn,
                        &symbol,
                        primary_timeframe,
                        last_primary_candle.clone(),
                    );

                    if let Some(candle) = mark {
                        let step = runtime_session.force_close(candle, "shutdown liquidation")?;
                        info!(events = ?step.events, snapshot = ?step.portfolio_snapshot, "Shutdown force-close request completed and Paper Trading projection confirmed");
                    } else {
                        warn!(symbol, "No Primary mark price available for shutdown force-close request");
                    }
                }
                break;
            }
        }
    }

    Ok(())
}

fn latest_primary_mark_candle(
    conn: &DbConnection,
    symbol: &str,
    primary_timeframe: Timeframe,
    last_primary_candle: Option<Candle>,
) -> Option<Candle> {
    last_primary_candle.or_else(|| {
        get_candles_before(conn, symbol, &primary_timeframe.to_string(), i64::MAX, 1)
            .into_iter()
            .next_back()
    })
}

fn complete_protective_shutdown<S: PaperTradingPersistenceStore>(
    runtime_session: &mut PaperLiveRuntimeSession<S>,
    snapshot: &RuntimePortfolioSnapshot,
    mark_candle: Option<Candle>,
    trigger: &ProtectiveShutdownTrigger,
) -> Result<Option<RuntimeStep>> {
    if snapshot.open_position.is_none() {
        info!(
            runtime_asset = %trigger.runtime_asset,
            primary_timeframe = %trigger.primary_timeframe,
            "Protective Runner Shutdown stopping flat runtime without force-close"
        );
        return Ok(None);
    }

    let Some(mark_candle) = mark_candle else {
        bail!(
            "Protective Runner Shutdown for asset '{}' on Primary timeframe '{}' found an open position but no completed Primary mark candle was available for runtime force_close",
            trigger.runtime_asset,
            trigger.primary_timeframe,
        );
    };

    Ok(Some(runtime_session.force_close(
        mark_candle,
        PROTECTIVE_RUNNER_SHUTDOWN_REASON,
    )?))
}

fn warmup_runtime_asset<S: PaperTradingPersistenceStore>(
    conn: &DbConnection,
    runtime_session: &mut PaperLiveRuntimeSession<S>,
    symbol: &str,
    warmup_requirement: usize,
) -> Result<std::collections::HashMap<Timeframe, i64>> {
    let mut high_water = std::collections::HashMap::new();

    if warmup_requirement == 0 {
        return Ok(high_water);
    }

    for timeframe in runtime_session.configured_timeframes() {
        let timeframe_string = timeframe.to_string();
        let candles = get_candles_before(
            conn,
            symbol,
            &timeframe_string,
            i64::MAX,
            warmup_requirement as u32,
        );
        let loaded = candles.len();

        if loaded == 0 {
            warn!(
                symbol,
                timeframe = %timeframe,
                warmup_requirement,
                "No historical candles in DB — runtime starts cold for timeframe"
            );
            continue;
        }

        if loaded < warmup_requirement {
            warn!(
                symbol,
                timeframe = %timeframe,
                available = loaded,
                requested = warmup_requirement,
                "Fewer candles than requested — runtime partially warmed for timeframe"
            );
        }

        for candle in candles {
            high_water.insert(timeframe, candle.timestamp);
            let step = runtime_session.on_warmup_candle(candle)?;
            info!(events = ?step.events, "Runtime warmup input accepted");
        }

        info!(
            symbol,
            timeframe = %timeframe,
            loaded,
            high_water_ts = ?high_water.get(&timeframe),
            "Runtime warmed for timeframe"
        );
    }

    Ok(high_water)
}

fn runtime_input_error(error: RuntimeInputError) -> anyhow::Error {
    match error {
        RuntimeInputError::UnknownTimeframe { timeframe } => {
            anyhow!("runtime rejected unknown timeframe '{timeframe}'")
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::{Cell, RefCell};

    use super::*;
    use crate::{
        config::ProtectiveShutdownConfig,
        paper_trading_persistence::{
            open_position_projection_key, paper_open_position_from_runtime,
            paper_trade_from_runtime, PaperTradingPersistenceError,
        },
    };
    use db_layer::{PaperOpenPosition, PaperTrade};
    use domain::{ClosedPosition, EntryRiskParameters, OpenPosition, PositionSide};
    use trading_runtime::{ExitKind, RuntimeEvent, StrategyDecisionIntent};

    const STRATEGY_IDENTITY: &str = "btc-paper";
    const RUNTIME_ASSET: &str = "BTC-USD";

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum PaperWrite {
        Open {
            projection_key: String,
        },
        Close {
            open_projection_key: String,
            trade_projection_key: String,
        },
    }

    #[derive(Default)]
    struct FakePaperStore {
        open_position: RefCell<Option<PaperOpenPosition>>,
        trades: RefCell<Vec<PaperTrade>>,
        writes: RefCell<Vec<PaperWrite>>,
        load_calls: Cell<usize>,
        fail_next_open: RefCell<Option<String>>,
        fail_next_close: RefCell<Option<String>>,
    }

    impl FakePaperStore {
        fn set_open_position(&self, position: PaperOpenPosition) {
            *self.open_position.borrow_mut() = Some(position);
        }

        fn set_trades(&self, trades: Vec<PaperTrade>) {
            *self.trades.borrow_mut() = trades;
        }

        fn fail_next_open(&self, message: impl Into<String>) {
            *self.fail_next_open.borrow_mut() = Some(message.into());
        }
    }

    impl PaperTradingPersistenceStore for FakePaperStore {
        fn load_open_position(
            &self,
            _strategy_identity: &str,
            _runtime_asset: &str,
        ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError> {
            self.load_calls.set(self.load_calls.get() + 1);
            Ok(self.open_position.borrow().clone())
        }

        fn load_trades(
            &self,
            _strategy_identity: &str,
            _runtime_asset: &str,
        ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError> {
            self.load_calls.set(self.load_calls.get() + 1);
            Ok(self.trades.borrow().clone())
        }

        fn open_position(
            &self,
            position: &PaperOpenPosition,
        ) -> Result<(), PaperTradingPersistenceError> {
            self.writes.borrow_mut().push(PaperWrite::Open {
                projection_key: position.projection_key.clone(),
            });

            if let Some(message) = self.fail_next_open.borrow_mut().take() {
                return Err(PaperTradingPersistenceError::Store(message));
            }

            let mut open_position = self.open_position.borrow_mut();
            match open_position.as_ref() {
                Some(existing) if existing == position => Ok(()),
                Some(existing) => Err(PaperTradingPersistenceError::Store(format!(
                    "conflicting open position '{}'",
                    existing.projection_key
                ))),
                None => {
                    *open_position = Some(position.clone());
                    Ok(())
                }
            }
        }

        fn record_position_closed(
            &self,
            open_projection_key: &str,
            trade: &PaperTrade,
        ) -> Result<(), PaperTradingPersistenceError> {
            self.writes.borrow_mut().push(PaperWrite::Close {
                open_projection_key: open_projection_key.to_string(),
                trade_projection_key: trade.projection_key.clone(),
            });

            if let Some(message) = self.fail_next_close.borrow_mut().take() {
                return Err(PaperTradingPersistenceError::Store(message));
            }

            let mut trades = self.trades.borrow_mut();
            if let Some(existing_trade) = trades
                .iter()
                .find(|existing| existing.projection_key == trade.projection_key)
            {
                if existing_trade == trade {
                    remove_matching_open_position(
                        &mut self.open_position.borrow_mut(),
                        open_projection_key,
                    );
                    return Ok(());
                }

                return Err(PaperTradingPersistenceError::Store(format!(
                    "conflicting completed trade '{}'",
                    existing_trade.projection_key
                )));
            }

            let mut open_position = self.open_position.borrow_mut();
            let Some(existing_open) = open_position.as_ref() else {
                return Err(PaperTradingPersistenceError::Store(format!(
                    "no matching open paper position for '{open_projection_key}'"
                )));
            };
            if existing_open.projection_key != open_projection_key {
                return Err(PaperTradingPersistenceError::Store(format!(
                    "open paper position '{}' does not match '{open_projection_key}'",
                    existing_open.projection_key
                )));
            }

            *open_position = None;
            trades.push(trade.clone());
            Ok(())
        }
    }

    fn remove_matching_open_position(
        open_position: &mut Option<PaperOpenPosition>,
        open_projection_key: &str,
    ) {
        if open_position
            .as_ref()
            .is_some_and(|open| open.projection_key == open_projection_key)
        {
            *open_position = None;
        }
    }

    fn asset() -> AssetConfig {
        AssetConfig {
            symbol: RUNTIME_ASSET.into(),
            strategy: "strategy.rhai".into(),
            execution_mode: LiveExecutionMode::PaperTrading,
            strategy_identity: Some(STRATEGY_IDENTITY.into()),
            balance: 10_000.0,
            liquidate_on_shutdown: true,
            protective_shutdown: ProtectiveShutdownConfig::default(),
        }
    }

    fn candle(timeframe: &str, timestamp: i64, close: f64) -> Candle {
        Candle {
            timestamp,
            symbol: RUNTIME_ASSET.into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            timeframe: timeframe.parse().expect("valid timeframe"),
        }
    }

    fn runtime_position(side: PositionSide) -> OpenPosition {
        OpenPosition {
            symbol: RUNTIME_ASSET.into(),
            side,
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: 1_700_000_000_000,
            entry_risk: EntryRiskParameters {
                stop_loss: Some(95.0),
                take_profit: Some(120.0),
            },
        }
    }

    fn closed_position(position: OpenPosition, realized_pnl: f64) -> ClosedPosition {
        ClosedPosition {
            position,
            exit_price: 110.0,
            exit_time: 1_700_000_060_000,
            realized_pnl,
        }
    }

    fn hold_strategy() -> &'static str {
        r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::hold().with_reason("hold")
}
"#
    }

    fn open_long_strategy() -> &'static str {
        r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::open_long(2.0).with_reason("entry")
}
"#
    }

    fn trigger() -> ProtectiveShutdownTrigger {
        ProtectiveShutdownTrigger {
            runtime_asset: RUNTIME_ASSET.into(),
            primary_timeframe: "1m".parse().expect("valid timeframe"),
            threshold: 1,
            blocked_contexts: Vec::new(),
            counters: Vec::new(),
        }
    }

    #[test]
    fn paper_live_session_restores_portfolio_state_before_runtime_ticks() {
        let store = FakePaperStore::default();
        store.set_open_position(paper_open_position_from_runtime(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &runtime_position(PositionSide::Long),
        ));
        let mut trade = paper_trade_from_runtime(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &closed_position(runtime_position(PositionSide::Short), 25.0),
            ExitKind::StrategyExit,
        );
        trade.realized_pnl = 25.0;
        store.set_trades(vec![trade]);
        let mut restored_asset = asset();
        restored_asset.balance = 1_000.0;

        let mut session =
            PaperLiveRuntimeSession::from_strategy_source(restored_asset, hold_strategy(), &store)
                .expect("paper live session should restore");

        let step = session
            .on_completed_candle(candle("1m", 1_700_000_120_000, 110.0))
            .expect("restored runtime should accept primary candle");

        assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_025.0);
        assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
        assert_eq!(step.portfolio_snapshot.current_equity, 1_045.0);
        assert_eq!(
            step.portfolio_snapshot
                .open_position
                .as_ref()
                .map(|p| p.side),
            Some(PositionSide::Long)
        );
    }

    #[test]
    fn missing_paper_strategy_identity_returns_clear_configuration_error() {
        let store = FakePaperStore::default();
        let mut missing_identity = asset();
        missing_identity.strategy_identity = None;

        let error = match PaperLiveRuntimeSession::from_strategy_source(
            missing_identity,
            hold_strategy(),
            &store,
        ) {
            Ok(_) => panic!("missing paper identity should fail before runtime starts"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("requires an explicit non-empty strategy_identity"));
        assert_eq!(store.load_calls.get(), 0);
        assert!(store.writes.borrow().is_empty());
    }

    #[test]
    fn real_money_mode_is_not_projected_as_paper_or_broker_truth() {
        let store = FakePaperStore::default();
        let mut real_money = asset();
        real_money.execution_mode = LiveExecutionMode::RealMoney;

        let error = match PaperLiveRuntimeSession::from_strategy_source(
            real_money,
            open_long_strategy(),
            &store,
        ) {
            Ok(_) => panic!("real-money execution is intentionally unsupported in this slice"),
            Err(error) => error,
        };

        assert!(error
            .to_string()
            .contains("real-money live execution is not yet supported"));
        assert!(error.to_string().contains("not projected as broker truth"));
        assert_eq!(store.load_calls.get(), 0);
        assert!(store.writes.borrow().is_empty());
    }

    #[test]
    fn live_runtime_asset_feeds_secondary_context_and_primary_ticks_into_one_runtime() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::optional(timeframe("1h")))
}

fn on_tick(market, context) {
    if market.candle(timeframe("1h")) == () {
        return decision::hold().with_reason("missing secondary");
    }

    decision::open_long(1.0).with_reason("secondary available")
}
"#;
        let store = FakePaperStore::default();
        let mut runner = PaperLiveRuntimeSession::from_strategy_source(asset(), source, &store)
            .expect("runtime asset should build");

        let secondary_step = runner
            .on_completed_candle(candle("1h", 3_600_000, 105.0))
            .expect("secondary candle should be accepted");

        assert!(secondary_step
            .events
            .iter()
            .all(|event| !matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
        assert!(secondary_step.portfolio_snapshot.open_position.is_none());
        assert!(store.writes.borrow().is_empty());

        let primary_step = runner
            .on_completed_candle(candle("1m", 7_200_000, 100.0))
            .expect("primary candle should be accepted");

        assert!(primary_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyDecisionProduced { decision }
                if decision.intent == StrategyDecisionIntent::OpenLong
        )));
        assert!(primary_step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::PositionOpened { .. })));
        assert!(matches!(
            store.writes.borrow().as_slice(),
            [PaperWrite::Open { .. }]
        ));
    }

    #[test]
    fn paper_projection_is_confirmed_before_accepting_next_completed_candle() {
        let store = FakePaperStore::default();
        let mut session =
            PaperLiveRuntimeSession::from_strategy_source(asset(), open_long_strategy(), &store)
                .expect("paper live session should build");

        let first_step = session
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("first candle should open and project synchronously");
        assert!(first_step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::PositionOpened { .. })));
        assert!(store.open_position.borrow().is_some());
        assert!(matches!(
            store.writes.borrow().as_slice(),
            [PaperWrite::Open { .. }]
        ));

        session
            .on_completed_candle(candle("1m", 120_000, 101.0))
            .expect("next candle is accepted only after prior projection returned");
        assert!(store.open_position.borrow().is_some());
    }

    #[test]
    fn force_close_step_is_projected_before_shutdown_completes() {
        let store = FakePaperStore::default();
        let mut session =
            PaperLiveRuntimeSession::from_strategy_source(asset(), open_long_strategy(), &store)
                .expect("paper live session should build");
        session
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("entry should be projected");
        store.writes.borrow_mut().clear();

        let force_close_step = session
            .force_close(candle("1m", 120_000, 101.0), "shutdown liquidation")
            .expect("force close should project before returning");

        assert!(force_close_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::PositionClosed {
                exit_kind: ExitKind::ForceClose,
                ..
            }
        )));
        assert!(store.open_position.borrow().is_none());
        assert_eq!(store.trades.borrow().len(), 1);
        assert!(matches!(
            store.writes.borrow().as_slice(),
            [PaperWrite::Close { .. }]
        ));
    }

    #[test]
    fn failed_paper_projection_stops_session_before_further_candles() {
        let store = FakePaperStore::default();
        store.fail_next_open("unconfirmed open projection");
        let mut session =
            PaperLiveRuntimeSession::from_strategy_source(asset(), open_long_strategy(), &store)
                .expect("paper live session should build");

        let error = session
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect_err("unconfirmed projection should stop paper live runner");

        assert!(error
            .to_string()
            .contains("Paper Trading persistence projection failed"));
        assert!(error.to_string().contains("unconfirmed open projection"));
        assert_eq!(store.writes.borrow().len(), 1);

        let stopped_error = session
            .on_completed_candle(candle("1m", 120_000, 101.0))
            .expect_err("stopped runner must not feed another candle");

        assert!(stopped_error
            .to_string()
            .contains("no further Tradable Candles will be processed"));
        assert_eq!(store.writes.borrow().len(), 1);
    }

    #[test]
    fn protective_shutdown_when_flat_stops_without_force_close() {
        let store = FakePaperStore::default();
        let mut runner =
            PaperLiveRuntimeSession::from_strategy_source(asset(), hold_strategy(), &store)
                .expect("runtime asset should build");
        let step = runner
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("primary candle should be accepted");

        let force_close_step = complete_protective_shutdown(
            &mut runner,
            &step.portfolio_snapshot,
            Some(candle("1m", 120_000, 101.0)),
            &trigger(),
        )
        .expect("flat shutdown should succeed");

        assert!(force_close_step.is_none());
        assert!(store.writes.borrow().is_empty());
    }

    #[test]
    fn protective_shutdown_with_open_position_requests_runtime_force_close() {
        let store = FakePaperStore::default();
        let mut runner =
            PaperLiveRuntimeSession::from_strategy_source(asset(), open_long_strategy(), &store)
                .expect("runtime asset should build");
        let step = runner
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("primary candle should open a position");
        store.writes.borrow_mut().clear();

        assert!(step.portfolio_snapshot.open_position.is_some());

        let force_close_step = complete_protective_shutdown(
            &mut runner,
            &step.portfolio_snapshot,
            Some(candle("1m", 120_000, 101.0)),
            &trigger(),
        )
        .expect("open-position shutdown with a mark should succeed")
        .expect("force-close step should be returned");

        assert!(force_close_step
            .events
            .contains(&RuntimeEvent::ForceCloseRequested {
                candle: candle("1m", 120_000, 101.0),
                reason: PROTECTIVE_RUNNER_SHUTDOWN_REASON.into(),
            }));
        assert!(force_close_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::PositionClosed {
                exit_kind: ExitKind::ForceClose,
                ..
            }
        )));
        assert!(force_close_step.portfolio_snapshot.open_position.is_none());
        assert!(matches!(
            store.writes.borrow().as_slice(),
            [PaperWrite::Close { .. }]
        ));
    }

    #[test]
    fn protective_shutdown_with_open_position_and_no_mark_returns_clear_error() {
        let store = FakePaperStore::default();
        let mut runner =
            PaperLiveRuntimeSession::from_strategy_source(asset(), open_long_strategy(), &store)
                .expect("runtime asset should build");
        let step = runner
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("primary candle should open a position");

        let error =
            complete_protective_shutdown(&mut runner, &step.portfolio_snapshot, None, &trigger())
                .expect_err("missing mark should stop with an error");

        assert!(error
            .to_string()
            .contains("no completed Primary mark candle"));
    }

    #[test]
    fn open_projection_uses_runtime_asset_and_strategy_identity_boundary() {
        let position = runtime_position(PositionSide::Long);
        let expected_key =
            open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);
        let projected =
            paper_open_position_from_runtime(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);

        assert_eq!(projected.projection_key, expected_key);
        assert_eq!(projected.strategy_identity, STRATEGY_IDENTITY);
        assert_eq!(projected.runtime_asset, RUNTIME_ASSET);
    }
}
