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
use shared::{Candle, Timeframe};
use spacetimedb_sdk::Table;
use trading_runtime::{
    resolve_warmup_plan, MarketInput, PortfolioState, RhaiStrategy, RuntimeConfig,
    RuntimeInputError, RuntimeStep, SecondaryTimeframeConfig, TradingRuntime,
};

use crate::config::AssetConfig;

/// Live runtime wrapper for one configured Runtime Asset.
struct LiveRuntimeAsset {
    config: RuntimeConfig,
    runtime: TradingRuntime<RhaiStrategy>,
}

impl LiveRuntimeAsset {
    fn from_strategy_source(asset: AssetConfig, strategy_source: &str) -> Result<Self> {
        let run_config = runtime_config_from_asset(&asset)?;
        let strategy = RhaiStrategy::load(strategy_source)?;
        let config = run_config.merge_strategy_config(strategy.strategy_config());
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

/// Run a live runtime instance for one configured asset. The first configured
/// interval is the Primary Timeframe; later intervals are Secondary Timeframes
/// that feed the same runtime as market context.
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
        "Live runtime configured"
    );

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
                    }
                    Err(e) => {
                        error!(error = %e, "Runtime input error");
                    }
                }
            }

            _ = cancel.cancelled() => {
                info!(symbol, primary_timeframe = %primary_timeframe, "Live runtime shutting down");
                if asset.liquidate_on_shutdown {
                    let mark = last_primary_candle.clone().or_else(|| {
                        get_candles_before(
                            &conn,
                            &symbol,
                            &primary_timeframe.to_string(),
                            i64::MAX,
                            1,
                        )
                        .into_iter()
                        .next_back()
                    });

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

fn runtime_config_from_asset(asset: &AssetConfig) -> Result<RuntimeConfig> {
    let Some(primary_raw) = asset.intervals.first() else {
        bail!(
            "Asset '{}' must configure at least one interval",
            asset.symbol
        );
    };

    let primary_timeframe = parse_timeframe(primary_raw)?;
    let mut seen = HashSet::from([primary_timeframe]);
    let mut secondary_timeframes = Vec::new();

    for raw in asset.intervals.iter().skip(1) {
        let timeframe = parse_timeframe(raw)?;
        if !seen.insert(timeframe) {
            bail!(
                "Asset '{}' configures duplicate timeframe '{}'",
                asset.symbol,
                timeframe
            );
        }
        secondary_timeframes.push(SecondaryTimeframeConfig::optional(timeframe, 0));
    }

    Ok(RuntimeConfig::with_secondary_configs(
        asset.symbol.clone(),
        primary_timeframe,
        secondary_timeframes,
    ))
}

fn parse_timeframe(raw: &str) -> Result<Timeframe> {
    raw.parse()
        .map_err(|e| anyhow!("Invalid configured interval '{}': {e}", raw))
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
    use trading_runtime::{RuntimeEvent, StrategyDecisionIntent};

    fn asset() -> AssetConfig {
        AssetConfig {
            symbol: "BTC-USD".into(),
            intervals: vec!["1m".into(), "1h".into()],
            strategy: "strategy.rhai".into(),
            balance: 10_000.0,
            liquidate_on_shutdown: true,
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

    #[test]
    fn live_runtime_asset_feeds_secondary_context_and_primary_ticks_into_one_runtime() {
        let source = r#"
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
            .on_completed_candle(candle("1m", 3_660_000, 100.0))
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
}
