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
    config::AssetConfig,
    protective_shutdown::{ProtectiveShutdownPolicy, ProtectiveShutdownTrigger},
};

const PROTECTIVE_RUNNER_SHUTDOWN_REASON: &str = "protective runner shutdown";

/// Live runtime wrapper for one configured Runtime Asset.
struct LiveRuntimeAsset {
    config: RuntimeConfig,
    runtime: TradingRuntime<RhaiStrategy>,
}

impl LiveRuntimeAsset {
    fn from_strategy_source(asset: AssetConfig, strategy_source: &str) -> Result<Self> {
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
            PortfolioState::new(asset.balance),
            warmup_plan,
            strategy,
        );

        Ok(Self { config, runtime })
    }

    fn from_strategy_file(asset: AssetConfig) -> Result<Self> {
        let strategy_source = std::fs::read_to_string(&asset.strategy)
            .map_err(|e| anyhow!("Cannot read strategy '{}': {e}", asset.strategy))?;
        Self::from_strategy_source(asset, &strategy_source)
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

/// Run a live runtime instance for one configured asset. The strategy file owns
/// the Primary Timeframe and any Secondary Timeframes; daemon config binds that
/// contract to the Runtime Asset and live runner policies.
pub async fn run(
    client: Arc<SpacetimeClient>,
    asset: AssetConfig,
    cancel: CancellationToken,
) -> Result<()> {
    let symbol = asset.symbol.clone();

    info!(symbol, strategy = asset.strategy, "Starting live runtime");

    let mut runtime_asset = LiveRuntimeAsset::from_strategy_file(asset.clone())?;
    let primary_timeframe = runtime_asset.primary_timeframe();
    let configured_timeframes = runtime_asset.configured_timeframes();

    info!(
        symbol,
        primary_timeframe = %primary_timeframe,
        secondary_timeframes = ?runtime_asset.config.secondary_timeframes,
        warmup_requirement = runtime_asset.warmup_requirement(),
        protective_shutdown_enabled = asset.protective_shutdown.enabled,
        protective_shutdown_threshold = asset.protective_shutdown.required_secondary_failure_threshold,
        "Live runtime configured"
    );

    let mut protective_shutdown =
        ProtectiveShutdownPolicy::new(symbol.clone(), primary_timeframe, asset.protective_shutdown);

    let conn: Arc<DbConnection> = client.conn.clone();
    let warmup_requirement = runtime_asset.warmup_requirement();
    let warmup_high_water =
        warmup_runtime_asset(&conn, &mut runtime_asset, &symbol, warmup_requirement)?;

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

                match runtime_asset.on_completed_candle(candle) {
                    Ok(step) => {
                        info!(events = ?step.events, snapshot = ?step.portfolio_snapshot, "Runtime step completed");
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
                                &mut runtime_asset,
                                &step.portfolio_snapshot,
                                mark,
                                &trigger,
                            )? {
                                info!(
                                    events = ?force_close_step.events,
                                    snapshot = ?force_close_step.portfolio_snapshot,
                                    "Protective Runner Shutdown force-close request completed"
                                );
                            }
                            break;
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "Runtime input error");
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
                        let step = runtime_asset.force_close(candle, "shutdown liquidation");
                        info!(events = ?step.events, snapshot = ?step.portfolio_snapshot, "Shutdown force-close request completed");
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

fn complete_protective_shutdown(
    runtime_asset: &mut LiveRuntimeAsset,
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

    Ok(Some(runtime_asset.force_close(
        mark_candle,
        PROTECTIVE_RUNNER_SHUTDOWN_REASON,
    )))
}

fn warmup_runtime_asset(
    conn: &DbConnection,
    runtime_asset: &mut LiveRuntimeAsset,
    symbol: &str,
    warmup_requirement: usize,
) -> Result<std::collections::HashMap<Timeframe, i64>> {
    let mut high_water = std::collections::HashMap::new();

    if warmup_requirement == 0 {
        return Ok(high_water);
    }

    for timeframe in runtime_asset.configured_timeframes() {
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
            let step = runtime_asset.on_warmup_candle(candle)?;
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
    use super::*;
    use crate::config::ProtectiveShutdownConfig;
    use trading_runtime::{ExitKind, RuntimeEvent, StrategyDecisionIntent};

    fn asset() -> AssetConfig {
        AssetConfig {
            symbol: "BTC-USD".into(),
            strategy: "strategy.rhai".into(),
            balance: 10_000.0,
            liquidate_on_shutdown: true,
            protective_shutdown: ProtectiveShutdownConfig::default(),
        }
    }

    fn candle(timeframe: &str, timestamp: i64, close: f64) -> Candle {
        Candle {
            timestamp,
            symbol: "BTC-USD".into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            timeframe: timeframe.parse().expect("valid timeframe"),
        }
    }

    fn trigger() -> ProtectiveShutdownTrigger {
        ProtectiveShutdownTrigger {
            runtime_asset: "BTC-USD".into(),
            primary_timeframe: "1m".parse().expect("valid timeframe"),
            threshold: 1,
            blocked_contexts: Vec::new(),
            counters: Vec::new(),
        }
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
        let mut runner = LiveRuntimeAsset::from_strategy_source(asset(), source)
            .expect("runtime asset should build");

        let secondary_step = runner
            .on_completed_candle(candle("1h", 3_600_000, 105.0))
            .expect("secondary candle should be accepted");

        assert!(secondary_step
            .events
            .iter()
            .all(|event| !matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
        assert!(secondary_step.portfolio_snapshot.open_position.is_none());

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
    }

    #[test]
    fn protective_shutdown_when_flat_stops_without_force_close() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::hold().with_reason("flat")
}
"#;
        let mut runner = LiveRuntimeAsset::from_strategy_source(asset(), source)
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
    }

    #[test]
    fn protective_shutdown_with_open_position_requests_runtime_force_close() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::open_long(2.0).with_reason("entry")
}
"#;
        let mut runner = LiveRuntimeAsset::from_strategy_source(asset(), source)
            .expect("runtime asset should build");
        let step = runner
            .on_completed_candle(candle("1m", 60_000, 100.0))
            .expect("primary candle should open a position");

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
    }

    #[test]
    fn protective_shutdown_with_open_position_and_no_mark_returns_clear_error() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::open_long(1.0).with_reason("entry")
}
"#;
        let mut runner = LiveRuntimeAsset::from_strategy_source(asset(), source)
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
}
