//! Runtime-backed in-memory backtester.
//!
//! Pure, synchronous, no I/O, no database writes. Historical candles are
//! replayed through `trading-runtime`, and this crate derives backtest reports
//! from runtime snapshots and events.
pub mod plan;

use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Result};
use domain::{Candle, PositionSide, Timeframe};
use trading_runtime::{
    resolve_warmup_plan, ExitKind, MarketInput, PortfolioState, RhaiStrategy, RiskExitKind,
    RuntimeConfig, RuntimeEvent, RuntimeStep, TradingRuntime, WarmupPlan,
};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single completed (closed) trade in the backtest.
#[derive(Debug, Clone)]
pub struct Trade {
    pub side: PositionSide,
    pub entry_price: f64,
    pub exit_price: f64,
    pub size: f64,
    pub pnl: f64,
    pub entry_time: i64,
    pub exit_time: i64,
    pub entry_reason: String,
    pub exit_reason: String,
}

/// Equity measurement at a given candle timestamp.
#[derive(Debug, Clone, Copy)]
pub struct EquityPoint {
    pub timestamp: i64,
    pub equity: f64,
}

/// Aggregated metrics after a backtest run.
#[derive(Debug, Clone, Copy)]
pub struct BacktestMetrics {
    pub trade_count: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
    pub final_equity: f64,
    pub peak_equity: f64,
    pub cagr: f64,
    pub sharpe: f64,
    pub time_in_market_pct: f64,
    pub years: f64,
}

/// Buy-and-hold benchmark on the same candle series.
#[derive(Debug, Clone, Copy)]
pub struct Benchmark {
    pub final_equity: f64,
    pub cagr: f64,
    pub max_drawdown: f64,
    pub max_drawdown_pct: f64,
}

/// Full output of a backtest run.
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub trades: Vec<Trade>,
    pub equity_curve: Vec<EquityPoint>,
    pub metrics: BacktestMetrics,
    pub benchmark: Benchmark,
    pub final_balance: f64,
}

/// Runtime-backed backtest configuration for one runtime asset.
#[derive(Debug, Clone)]
pub struct RuntimeBacktestConfig {
    pub runtime_asset: String,
    pub initial_balance: f64,
    pub runtime_minimum_warmup: usize,
}

impl RuntimeBacktestConfig {
    pub fn new(runtime_asset: impl Into<String>, initial_balance: f64) -> Self {
        Self {
            runtime_asset: runtime_asset.into(),
            initial_balance,
            runtime_minimum_warmup: 0,
        }
    }

    pub fn with_runtime_minimum_warmup(mut self, runtime_minimum_warmup: usize) -> Self {
        self.runtime_minimum_warmup = runtime_minimum_warmup;
        self
    }
}

/// Historical candles for one non-primary configured timeframe.
#[derive(Debug, Clone)]
pub struct HistoricalCandleSeries {
    pub timeframe: Timeframe,
    pub candles: Vec<Candle>,
}

/// Historical market data supplied to the runtime-backed backtester.
#[derive(Debug, Clone)]
pub struct HistoricalMarketData {
    pub primary: Vec<Candle>,
    pub secondary: Vec<HistoricalCandleSeries>,
}

impl HistoricalMarketData {
    pub fn single_timeframe(primary: Vec<Candle>) -> Self {
        Self {
            primary,
            secondary: Vec::new(),
        }
    }

    pub fn with_secondary(
        primary: Vec<Candle>,
        secondary: impl IntoIterator<Item = HistoricalCandleSeries>,
    ) -> Self {
        Self {
            primary,
            secondary: secondary.into_iter().collect(),
        }
    }
}

/// Result of a runtime-backed historical replay.
#[derive(Debug, Clone)]
pub struct RuntimeBacktestResult {
    pub result: BacktestResult,
    pub steps: Vec<RuntimeStep>,
    pub effective_config: RuntimeConfig,
    pub warmup_plan: WarmupPlan,
}

// ── Analytics helpers ────────────────────────────────────────────────────────

const MS_PER_YEAR: f64 = 365.25 * 86_400_000.0;

fn span_years(curve: &[EquityPoint]) -> f64 {
    match (curve.first(), curve.last()) {
        (Some(a), Some(b)) if b.timestamp > a.timestamp => {
            (b.timestamp - a.timestamp) as f64 / MS_PER_YEAR
        }
        _ => 0.0,
    }
}

fn compute_cagr(initial: f64, final_val: f64, years: f64) -> f64 {
    if initial <= 0.0 || final_val <= 0.0 || years <= 0.0 {
        return 0.0;
    }
    (final_val / initial).powf(1.0 / years) - 1.0
}

/// Annualised Sharpe ratio (risk-free = 0) from the equity curve.
fn compute_sharpe(curve: &[EquityPoint], years: f64) -> f64 {
    if curve.len() < 2 || years <= 0.0 {
        return 0.0;
    }
    let rets: Vec<f64> = curve
        .windows(2)
        .filter_map(|w| {
            if w[0].equity > 0.0 {
                Some(w[1].equity / w[0].equity - 1.0)
            } else {
                None
            }
        })
        .collect();
    if rets.len() < 2 {
        return 0.0;
    }
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (rets.len() - 1) as f64;
    let std = var.sqrt();
    if std == 0.0 {
        return 0.0;
    }
    let periods_per_year = rets.len() as f64 / years;
    (mean / std) * periods_per_year.sqrt()
}

/// Compute buy-and-hold benchmark over the tick candles.
/// Starts with `initial_balance`, buys at first close, holds to last close.
fn compute_benchmark(initial_balance: f64, candles: &[Candle]) -> Benchmark {
    if candles.len() < 2 || candles[0].close <= 0.0 {
        return Benchmark {
            final_equity: initial_balance,
            cagr: 0.0,
            max_drawdown: 0.0,
            max_drawdown_pct: 0.0,
        };
    }
    let entry = candles[0].close;
    let size = initial_balance / entry;

    let mut peak = initial_balance;
    let mut max_dd = 0.0;
    for c in candles {
        let eq = size * c.close;
        if eq > peak {
            peak = eq;
        }
        let dd = peak - eq;
        if dd > max_dd {
            max_dd = dd;
        }
    }
    let final_equity = size * candles.last().unwrap().close;
    let years = (candles.last().unwrap().timestamp - candles[0].timestamp) as f64 / MS_PER_YEAR;
    let cagr = compute_cagr(initial_balance, final_equity, years);
    let max_dd_pct = if peak > 0.0 { max_dd / peak } else { 0.0 };

    Benchmark {
        final_equity,
        cagr,
        max_drawdown: max_dd,
        max_drawdown_pct: max_dd_pct,
    }
}

// ── Runtime-backed runner ─────────────────────────────────────────────────────

/// Load typed Runtime strategy handling, derive the runtime timeframe contract
/// from Strategy Configuration, and replay historical candles through one
/// [`TradingRuntime`].
pub fn run_runtime_backtest(
    strategy_src: &str,
    market_data: HistoricalMarketData,
    config: RuntimeBacktestConfig,
) -> Result<RuntimeBacktestResult> {
    let strategy = RhaiStrategy::load(strategy_src)?;
    let effective_config = RuntimeConfig::from_strategy_config(
        config.runtime_asset.clone(),
        strategy.strategy_config(),
    )?;
    let warmup_plan = resolve_warmup_plan(
        &effective_config,
        strategy.strategy_config(),
        strategy.ast(),
        strategy.scope(),
        config.runtime_minimum_warmup,
    );

    run_prepared_runtime_backtest(
        strategy,
        effective_config,
        warmup_plan,
        market_data,
        config.initial_balance,
    )
}

/// Variant for callers (such as the CLI) that load each strategy-configured
/// timeframe after Strategy Configuration has been resolved.
pub fn run_runtime_backtest_with_loader<F>(
    strategy_src: &str,
    config: RuntimeBacktestConfig,
    mut load_candles: F,
) -> Result<RuntimeBacktestResult>
where
    F: FnMut(&str, Timeframe) -> Result<Vec<Candle>>,
{
    let strategy = RhaiStrategy::load(strategy_src)?;
    let effective_config = RuntimeConfig::from_strategy_config(
        config.runtime_asset.clone(),
        strategy.strategy_config(),
    )?;
    let warmup_plan = resolve_warmup_plan(
        &effective_config,
        strategy.strategy_config(),
        strategy.ast(),
        strategy.scope(),
        config.runtime_minimum_warmup,
    );

    let primary = load_candles(
        &effective_config.runtime_asset,
        effective_config.primary_timeframe,
    )?;
    let secondary = effective_config
        .secondary_timeframes
        .iter()
        .map(|secondary| {
            Ok(HistoricalCandleSeries {
                timeframe: secondary.timeframe,
                candles: load_candles(&effective_config.runtime_asset, secondary.timeframe)?,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    run_prepared_runtime_backtest(
        strategy,
        effective_config,
        warmup_plan,
        HistoricalMarketData::with_secondary(primary, secondary),
        config.initial_balance,
    )
}

fn run_prepared_runtime_backtest(
    strategy: RhaiStrategy,
    effective_config: RuntimeConfig,
    warmup_plan: WarmupPlan,
    market_data: HistoricalMarketData,
    initial_balance: f64,
) -> Result<RuntimeBacktestResult> {
    let mut histories = prepare_histories(&effective_config, market_data)?;
    let mut runtime = TradingRuntime::with_warmup_plan(
        effective_config.clone(),
        PortfolioState::new(initial_balance),
        warmup_plan.clone(),
        strategy,
    );
    let mut steps = Vec::new();
    let mut recorder = RuntimeBacktestRecorder::new(initial_balance);

    let mut replay_inputs = Vec::new();
    let mut replay_sequence = 0usize;
    for timeframe in configured_timeframes(&effective_config) {
        let required = warmup_plan.requirement_for(timeframe).unwrap_or(0);
        let history = histories.remove(&timeframe).unwrap_or_default();

        for (index, candle) in history.into_iter().enumerate() {
            replay_inputs.push(HistoricalReplayInput {
                is_warmup: index < required,
                sequence: replay_sequence,
                candle,
            });
            replay_sequence += 1;
        }
    }

    replay_inputs.sort_by(|left, right| {
        left.candle
            .close_time()
            .cmp(&right.candle.close_time())
            .then_with(|| {
                input_order_for_same_close_time(&effective_config, &left.candle).cmp(
                    &input_order_for_same_close_time(&effective_config, &right.candle),
                )
            })
            .then_with(|| {
                left.candle
                    .timeframe
                    .to_string()
                    .cmp(&right.candle.timeframe.to_string())
            })
            .then_with(|| left.sequence.cmp(&right.sequence))
    });

    for replay_input in replay_inputs {
        let is_completed_primary = !replay_input.is_warmup
            && replay_input.candle.timeframe == effective_config.primary_timeframe;
        let recorder_candle = is_completed_primary.then(|| replay_input.candle.clone());
        let market_input = if replay_input.is_warmup {
            MarketInput::WarmupCandle(replay_input.candle)
        } else {
            MarketInput::CompletedCandle(replay_input.candle)
        };
        let step = runtime
            .on_market_input(market_input)
            .map_err(runtime_input_error)?;

        if let Some(candle) = recorder_candle.as_ref() {
            if step_has_tradable_candle(&step) {
                recorder.record_tradable_step(candle, &step);
            }
        }

        steps.push(step);
    }

    let result = recorder.finish();

    Ok(RuntimeBacktestResult {
        result,
        steps,
        effective_config,
        warmup_plan,
    })
}

struct HistoricalReplayInput {
    candle: Candle,
    is_warmup: bool,
    sequence: usize,
}

fn prepare_histories(
    config: &RuntimeConfig,
    market_data: HistoricalMarketData,
) -> Result<HashMap<Timeframe, Vec<Candle>>> {
    let configured: HashSet<Timeframe> = configured_timeframes(config).into_iter().collect();
    let mut histories = HashMap::new();

    insert_history(
        &mut histories,
        config.primary_timeframe,
        market_data.primary,
        &configured,
    )?;

    for series in market_data.secondary {
        insert_history(
            &mut histories,
            series.timeframe,
            series.candles,
            &configured,
        )?;
    }

    for timeframe in configured {
        histories.entry(timeframe).or_insert_with(Vec::new);
    }

    Ok(histories)
}

fn configured_timeframes(config: &RuntimeConfig) -> Vec<Timeframe> {
    let mut timeframes = Vec::with_capacity(1 + config.secondary_timeframes.len());
    timeframes.push(config.primary_timeframe);
    timeframes.extend(
        config
            .secondary_timeframes
            .iter()
            .map(|secondary| secondary.timeframe),
    );
    timeframes
}

fn insert_history(
    histories: &mut HashMap<Timeframe, Vec<Candle>>,
    timeframe: Timeframe,
    mut candles: Vec<Candle>,
    configured: &HashSet<Timeframe>,
) -> Result<()> {
    if !configured.contains(&timeframe) {
        bail!("historical candles supplied for unconfigured timeframe '{timeframe}'");
    }
    if histories.contains_key(&timeframe) {
        bail!("duplicate historical candle history for timeframe '{timeframe}'");
    }
    if candles.iter().any(|candle| candle.timeframe != timeframe) {
        bail!(
            "historical candle history for '{timeframe}' contains a candle with another timeframe"
        );
    }

    candles.sort_by_key(|candle| candle.timestamp);
    histories.insert(timeframe, candles);
    Ok(())
}

fn input_order_for_same_close_time(config: &RuntimeConfig, candle: &Candle) -> u8 {
    if candle.timeframe == config.primary_timeframe {
        1
    } else {
        0
    }
}

fn step_has_tradable_candle(step: &RuntimeStep) -> bool {
    step.events
        .iter()
        .any(|event| matches!(event, RuntimeEvent::TradableCandleAccepted { .. }))
}

fn runtime_input_error(error: trading_runtime::RuntimeInputError) -> anyhow::Error {
    match error {
        trading_runtime::RuntimeInputError::UnknownTimeframe { timeframe } => {
            anyhow!("runtime rejected unknown timeframe '{timeframe}'")
        }
    }
}

struct RuntimeBacktestRecorder {
    initial_balance: f64,
    trades: Vec<Trade>,
    equity_curve: Vec<EquityPoint>,
    peak_equity: f64,
    max_drawdown: f64,
    bars_total: usize,
    bars_in_position: usize,
    final_balance: f64,
    tradable_primary_candles: Vec<Candle>,
}

impl RuntimeBacktestRecorder {
    fn new(initial_balance: f64) -> Self {
        Self {
            initial_balance,
            trades: Vec::new(),
            equity_curve: Vec::new(),
            peak_equity: initial_balance,
            max_drawdown: 0.0,
            bars_total: 0,
            bars_in_position: 0,
            final_balance: initial_balance,
            tradable_primary_candles: Vec::new(),
        }
    }

    fn record_tradable_step(&mut self, candle: &Candle, step: &RuntimeStep) {
        for event in &step.events {
            if let RuntimeEvent::PositionClosed {
                closed_position,
                exit_kind,
            } = event
            {
                self.trades.push(Trade {
                    side: closed_position.position.side,
                    entry_price: closed_position.position.entry_price,
                    exit_price: closed_position.exit_price,
                    size: closed_position.position.size,
                    pnl: closed_position.realized_pnl,
                    entry_time: closed_position.position.entry_time,
                    exit_time: closed_position.exit_time,
                    entry_reason: String::new(),
                    exit_reason: exit_reason(*exit_kind).to_string(),
                });
            }
        }

        let equity = step.portfolio_snapshot.current_equity;
        if equity > self.peak_equity {
            self.peak_equity = equity;
        }
        let drawdown = self.peak_equity - equity;
        if drawdown > self.max_drawdown {
            self.max_drawdown = drawdown;
        }

        self.bars_total += 1;
        if step.portfolio_snapshot.open_position.is_some() {
            self.bars_in_position += 1;
        }
        self.final_balance = step.portfolio_snapshot.realized_cash_balance;
        self.equity_curve.push(EquityPoint {
            timestamp: candle.timestamp,
            equity,
        });
        self.tradable_primary_candles.push(candle.clone());
    }

    fn finish(self) -> BacktestResult {
        let metrics = self.metrics();
        let benchmark = compute_benchmark(self.initial_balance, &self.tradable_primary_candles);
        BacktestResult {
            trades: self.trades,
            equity_curve: self.equity_curve,
            metrics,
            benchmark,
            final_balance: self.final_balance,
        }
    }

    fn metrics(&self) -> BacktestMetrics {
        let trade_count = self.trades.len();
        let wins = self.trades.iter().filter(|trade| trade.pnl > 0.0).count();
        let losses = self.trades.iter().filter(|trade| trade.pnl < 0.0).count();
        let total_pnl = self.trades.iter().map(|trade| trade.pnl).sum::<f64>();
        let final_equity = self
            .equity_curve
            .last()
            .map(|point| point.equity)
            .unwrap_or(self.initial_balance);
        let win_rate = if trade_count == 0 {
            0.0
        } else {
            wins as f64 / trade_count as f64
        };
        let years = span_years(&self.equity_curve);
        let max_drawdown_pct = if self.peak_equity > 0.0 {
            self.max_drawdown / self.peak_equity
        } else {
            0.0
        };
        let time_in_market_pct = if self.bars_total == 0 {
            0.0
        } else {
            self.bars_in_position as f64 / self.bars_total as f64
        };

        BacktestMetrics {
            trade_count,
            wins,
            losses,
            win_rate,
            total_pnl,
            max_drawdown: self.max_drawdown,
            max_drawdown_pct,
            final_equity,
            peak_equity: self.peak_equity,
            cagr: compute_cagr(self.initial_balance, final_equity, years),
            sharpe: compute_sharpe(&self.equity_curve, years),
            time_in_market_pct,
            years,
        }
    }
}

fn exit_reason(exit_kind: ExitKind) -> &'static str {
    match exit_kind {
        ExitKind::StrategyExit => "strategy exit",
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        } => "stop-loss triggered",
        ExitKind::RiskExit {
            selected: RiskExitKind::TakeProfit,
        } => "take-profit triggered",
        ExitKind::ForceClose => "force-close",
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use domain::Timeframe;
    use trading_runtime::{RuntimeEvent, StrategyDecisionIntent};

    fn candle_at(ts: i64, close: f64, timeframe: Timeframe) -> Candle {
        Candle {
            timestamp: ts,
            symbol: "TEST".into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            timeframe,
        }
    }

    #[test]
    fn historical_backtest_feeds_warmup_primary_and_secondary_then_ticks_one_runtime() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(1)
        .with_secondary(secondary::required(H1).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    if market.candle(H1) == () {
        decision::hold().with_reason("missing secondary")
    } else {
        decision::open_long(2.0).with_reason("secondary available")
    }
}
"#;
        let market_data = HistoricalMarketData::with_secondary(
            vec![
                candle_at(60_000, 100.0, primary),
                candle_at(3_600_000, 101.0, primary),
            ],
            [HistoricalCandleSeries {
                timeframe: secondary,
                candles: vec![
                    candle_at(0, 150.0, secondary),
                    candle_at(3_600_000, 200.0, secondary),
                ],
            }],
        );
        let config = RuntimeBacktestConfig::new("TEST", 10_000.0);

        let backtest = run_runtime_backtest(source, market_data, config).unwrap();

        assert_eq!(backtest.warmup_plan.requirement_for(primary), Some(1));
        assert_eq!(backtest.warmup_plan.requirement_for(secondary), Some(1));
        assert!(backtest.steps.iter().any(|step| step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::WarmupCompleted { .. }))));

        let secondary_step = backtest
            .steps
            .iter()
            .find(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::MarketInputAccepted { candle }
                            if candle.timeframe == secondary && candle.timestamp == 3_600_000
                    )
                })
            })
            .expect("completed secondary step should be recorded");
        assert!(secondary_step
            .events
            .iter()
            .all(|event| !matches!(event, RuntimeEvent::StrategyTickStarted { .. })));

        let primary_step = backtest
            .steps
            .iter()
            .find(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::TradableCandleAccepted { candle }
                            if candle.timeframe == primary && candle.timestamp == 3_600_000
                    )
                })
            })
            .expect("completed primary step should be tradable");
        assert!(primary_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyDecisionProduced { decision }
                if decision.intent == StrategyDecisionIntent::OpenLong
        )));
        assert!(primary_step
            .portfolio_snapshot
            .open_position
            .as_ref()
            .is_some_and(|position| position.size == 2.0));
        assert_eq!(backtest.result.equity_curve.len(), 1);
    }

    #[test]
    fn historical_backtest_replays_warmup_chronologically_without_future_secondary_leakage() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(1)
        .with_secondary(secondary::required(H1).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    if market.candle(H1) == () {
        decision::hold().with_reason("missing secondary")
    } else {
        decision::open_long(1.0).with_reason("future secondary leaked")
    }
}
"#;
        let market_data = HistoricalMarketData::with_secondary(
            vec![
                candle_at(0, 100.0, primary),
                candle_at(60_000, 101.0, primary),
            ],
            [HistoricalCandleSeries {
                timeframe: secondary,
                candles: vec![candle_at(3_600_000, 200.0, secondary)],
            }],
        );
        let config = RuntimeBacktestConfig::new("TEST", 10_000.0);

        let backtest = run_runtime_backtest(source, market_data, config).unwrap();

        let early_primary_index = backtest
            .steps
            .iter()
            .position(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::MarketInputAccepted { candle }
                            if candle.timeframe == primary && candle.timestamp == 60_000
                    )
                })
            })
            .expect("early primary completed candle should be replayed");
        let future_secondary_index = backtest
            .steps
            .iter()
            .position(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::WarmupInputAccepted { candle }
                            if candle.timeframe == secondary && candle.timestamp == 3_600_000
                    )
                })
            })
            .expect("future secondary warmup candle should be replayed");
        assert!(early_primary_index < future_secondary_index);

        let early_primary_step = &backtest.steps[early_primary_index];
        assert!(early_primary_step.events.iter().all(|event| !matches!(
            event,
            RuntimeEvent::TradableCandleAccepted { .. }
                | RuntimeEvent::StrategyDecisionProduced { .. }
                | RuntimeEvent::PositionOpened { .. }
        )));
        assert!(early_primary_step
            .portfolio_snapshot
            .open_position
            .is_none());
        assert!(backtest.result.equity_curve.is_empty());
    }

    #[test]
    fn runtime_backtest_replays_secondary_at_same_derived_close_time_before_primary() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let ten_oclock = 10 * Timeframe::hours(1).duration_ms();
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    if market.candle(H1) == () {
        decision::hold().with_reason("missing secondary")
    } else {
        decision::open_long(1.0).with_reason("same-boundary secondary visible")
    }
}
"#;
        let before_secondary_close = ten_oclock + 58 * Timeframe::minutes(1).duration_ms();
        let at_secondary_close = ten_oclock + 59 * Timeframe::minutes(1).duration_ms();
        let market_data = HistoricalMarketData::with_secondary(
            vec![
                candle_at(before_secondary_close, 100.0, primary),
                candle_at(at_secondary_close, 101.0, primary),
            ],
            [HistoricalCandleSeries {
                timeframe: secondary,
                candles: vec![candle_at(ten_oclock, 200.0, secondary)],
            }],
        );

        let backtest = run_runtime_backtest(
            source,
            market_data,
            RuntimeBacktestConfig::new("TEST", 10_000.0),
        )
        .unwrap();

        let early_primary_index = backtest
            .steps
            .iter()
            .position(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::TradableCandleAccepted { candle }
                            if candle.timeframe == primary && candle.timestamp == before_secondary_close
                    )
                })
            })
            .expect("primary before secondary close should be replayed");
        let secondary_index = backtest
            .steps
            .iter()
            .position(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::MarketInputAccepted { candle }
                            if candle.timeframe == secondary && candle.timestamp == ten_oclock
                    )
                })
            })
            .expect("secondary input should be replayed");
        let boundary_primary_index = backtest
            .steps
            .iter()
            .position(|step| {
                step.events.iter().any(|event| {
                    matches!(
                        event,
                        RuntimeEvent::TradableCandleAccepted { candle }
                            if candle.timeframe == primary && candle.timestamp == at_secondary_close
                    )
                })
            })
            .expect("same-boundary primary should be replayed");

        assert!(early_primary_index < secondary_index);
        assert!(secondary_index < boundary_primary_index);
        assert!(backtest.steps[early_primary_index]
            .events
            .iter()
            .any(|event| {
                matches!(event, RuntimeEvent::StrategyTickBlocked { blocked_contexts, .. }
                if blocked_contexts.iter().any(|blocked| blocked.timeframe == secondary))
            }));
        assert!(backtest.steps[boundary_primary_index]
            .events
            .iter()
            .any(|event| {
                matches!(event, RuntimeEvent::StrategyDecisionProduced { decision }
                if decision.intent == StrategyDecisionIntent::OpenLong)
            }));
    }

    #[test]
    fn historical_backtest_preserves_required_secondary_blocking_and_optional_unavailable_events() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let required_source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(timeframe("1h")).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    decision::open_long(1.0)
}
"#;

        let required = run_runtime_backtest(
            required_source,
            HistoricalMarketData::single_timeframe(vec![candle_at(60_000, 100.0, primary)]),
            RuntimeBacktestConfig::new("TEST", 10_000.0),
        )
        .unwrap();
        let required_step = required
            .steps
            .last()
            .expect("primary step should be recorded");
        assert!(required_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyTickBlocked { blocked_contexts, .. }
                if blocked_contexts.iter().any(|blocked| blocked.timeframe == secondary)
        )));
        assert!(!required_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyDecisionProduced { .. } | RuntimeEvent::PositionOpened { .. }
        )));

        let optional_source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::optional(timeframe("1h")).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    decision::open_long(1.0)
}
"#;

        let optional = run_runtime_backtest(
            optional_source,
            HistoricalMarketData::single_timeframe(vec![candle_at(60_000, 100.0, primary)]),
            RuntimeBacktestConfig::new("TEST", 10_000.0),
        )
        .unwrap();
        let optional_step = optional
            .steps
            .last()
            .expect("primary step should be recorded");
        assert!(optional_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::SecondaryContextUnavailable { timeframe, .. } if *timeframe == secondary
        )));
        assert!(optional_step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyDecisionProduced { decision }
                if decision.intent == StrategyDecisionIntent::OpenLong
        )));
        assert!(optional_step.portfolio_snapshot.open_position.is_some());
    }

    #[test]
    fn runtime_backtest_loader_fetches_strategy_declared_secondary_timeframes() {
        let primary = Timeframe::minutes(1);
        let secondary = Timeframe::hours(1);
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let mut loaded = Vec::new();

        let backtest = run_runtime_backtest_with_loader(
            source,
            RuntimeBacktestConfig::new("TEST", 10_000.0),
            |symbol, timeframe| {
                loaded.push((symbol.to_string(), timeframe));
                Ok(match timeframe {
                    timeframe if timeframe == primary => vec![candle_at(3_600_000, 100.0, primary)],
                    timeframe if timeframe == secondary => {
                        vec![candle_at(3_600_000, 200.0, secondary)]
                    }
                    _ => Vec::new(),
                })
            },
        )
        .unwrap();

        assert_eq!(
            loaded,
            vec![
                ("TEST".to_string(), primary),
                ("TEST".to_string(), secondary),
            ]
        );
        assert!(backtest
            .effective_config
            .secondary_timeframes
            .iter()
            .any(|configured| configured.timeframe == secondary));
    }
}
