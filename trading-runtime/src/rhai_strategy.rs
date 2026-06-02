//! Rhai strategy loading and hook validation for the trading runtime.
//!
//! This module owns compile/load-time Rhai strategy handling inside the
//! `trading-runtime` crate, the typed decision API, grouped Strategy Context /
//! Strategy State, and the typed Primary/Secondary-Timeframe Market View used
//! at the strategy tick boundary. Optional Secondary-Timeframe `candle(tf)` and
//! `candles(tf)` access returns Rhai unit `()` when that context is missing or
//! stale for the current Strategy Tick.

use crate::{
    AnchoredConfiguration, AnchoredEvaluatorConfiguration, AnchoredOutput, AnchoredOutputs,
    AnchoredRuntime, MarketState, PivotDetectorConfiguration, PivotEvent, PivotSide,
    RuntimePortfolioSnapshot, SecondaryTimeframeConfig, StrategyConfiguration, StrategyDecision,
    StrategyError, StrategyHandler, StrategyState, StrategyStateValue, StrategyTickInput,
    StrategyTickResult, StructureConfiguration, StructureObjectConfiguration,
    StructureObjectRegistry, StructurePointRegistry, StructurePointSource,
};
use indicators::{
    anchored::evaluators::TrendLine,
    momentum::{cci::cci, roc::roc, rsi::rsi, williams_r::williams_r},
    slope::slope,
    trend::{dema::dema, ema::ema, sma::sma, tema::tema},
    volatility::atr::atr,
    volume::{mfi::mfi, obv::obv},
};
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::{Candle, Position, PositionSide, Timeframe};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    fmt,
    path::Path,
    sync::Arc,
};

/// Strategy hooks detected during Rhai strategy loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhaiStrategyHooks {
    pub has_on_tick: bool,
    pub has_strategy_config: bool,
    pub has_anchored_config: bool,
    pub has_structure_config: bool,
}

/// Runtime-owned compiled Rhai strategy and load-time metadata.
pub struct RhaiStrategy {
    engine: RhaiEngine,
    ast: AST,
    scope: Scope<'static>,
    hooks: RhaiStrategyHooks,
    strategy_config: StrategyConfiguration,
    anchored_config: Option<AnchoredConfiguration>,
    structure_config: Option<StructureConfiguration>,
    structure_object_ids: Option<HashSet<String>>,
    anchored_runtime: Option<AnchoredRuntime>,
}

#[derive(Debug, Clone)]
struct RhaiMarketView {
    current: RhaiCandle,
    primary_history: RhaiCandleHistory,
    histories_by_timeframe: HashMap<Timeframe, Option<RhaiCandleHistory>>,
    anchored_outputs: Option<AnchoredOutputs>,
    structure_object_ids: Option<HashSet<String>>,
}

impl RhaiMarketView {
    fn from_runtime(
        market: &crate::MarketView<'_>,
        anchored_outputs: Option<AnchoredOutputs>,
        structure_object_ids: Option<HashSet<String>>,
    ) -> Self {
        let primary_history = RhaiCandleHistory::new(market.primary_history().to_vec());
        let mut view = Self {
            current: RhaiCandle::new(market.primary_candle().clone()),
            primary_history: primary_history.clone(),
            histories_by_timeframe: HashMap::from([(
                market.primary_timeframe(),
                Some(primary_history),
            )]),
            anchored_outputs,
            structure_object_ids,
        };

        for timeframe in market.configured_timeframes() {
            if timeframe == market.primary_timeframe() {
                continue;
            }

            let history = market
                .visible_history(timeframe)
                .expect("configured timeframe should be visible or unavailable");
            view.insert_visible_history(timeframe, history);
        }

        view
    }

    fn insert_visible_history(&mut self, timeframe: Timeframe, history: Option<&[Candle]>) {
        self.histories_by_timeframe.insert(
            timeframe,
            history.map(|candles| RhaiCandleHistory::new(candles.to_vec())),
        );
    }

    fn visible_history(
        &self,
        timeframe: Timeframe,
    ) -> Result<Option<RhaiCandleHistory>, Box<EvalAltResult>> {
        match self.histories_by_timeframe.get(&timeframe) {
            Some(history) => Ok(history.clone()),
            None => Err(format!("unconfigured timeframe `{timeframe}`").into()),
        }
    }
}

#[derive(Debug, Clone)]
struct RhaiMarketStructureView {
    outputs: Option<AnchoredOutputs>,
    object_ids: Option<HashSet<String>>,
}

impl RhaiMarketStructureView {
    fn active(&self, object_id: &str) -> Result<Dynamic, Box<EvalAltResult>> {
        let Some(object_ids) = &self.object_ids else {
            return Err("no `structure_config()` declared Market Structure objects".into());
        };
        if !object_ids.contains(object_id) {
            return Err(format!("unknown Structure Object id `{object_id}`").into());
        }

        let lines = self
            .outputs
            .as_ref()
            .and_then(|outputs| outputs.values.get(object_id))
            .map(trendlines_to_rhai_array)
            .unwrap_or_default();
        Ok(Dynamic::from(lines))
    }
}

#[derive(Debug, Clone, PartialEq)]
struct RhaiCandle {
    candle: Candle,
}

impl RhaiCandle {
    fn new(candle: Candle) -> Self {
        Self { candle }
    }
}

#[derive(Debug, Clone)]
struct RhaiCandleHistory {
    candles: Arc<Vec<Candle>>,
}

impl RhaiCandleHistory {
    fn new(candles: Vec<Candle>) -> Self {
        Self {
            candles: Arc::new(candles),
        }
    }

    fn latest_candle(&self) -> Option<RhaiCandle> {
        self.candles.last().cloned().map(RhaiCandle::new)
    }

    fn chronological_candles_before_offset(&self, offset: usize) -> &[Candle] {
        let end = self.candles.len().saturating_sub(offset);
        &self.candles[..end]
    }

    fn chronological_closes_before_offset(&self, offset: usize) -> Vec<f64> {
        self.chronological_candles_before_offset(offset)
            .iter()
            .map(|candle| candle.close)
            .collect()
    }
}

#[derive(Debug, Clone, Copy)]
struct RhaiTrendLine {
    line: TrendLine,
}

impl RhaiTrendLine {
    fn new(line: TrendLine) -> Self {
        Self { line }
    }
}

#[derive(Debug, Clone, Copy)]
struct RhaiPivotEvent {
    event: PivotEvent,
}

impl RhaiPivotEvent {
    fn new(event: PivotEvent) -> Self {
        Self { event }
    }
}

#[derive(Debug, Clone)]
struct RhaiStrategyContext {
    portfolio: RhaiPortfolioSnapshot,
    state: StrategyState,
}

impl RhaiStrategyContext {
    fn from_runtime(portfolio: &RuntimePortfolioSnapshot, state: &StrategyState) -> Self {
        Self {
            portfolio: RhaiPortfolioSnapshot::from_runtime(portfolio),
            state: state.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct RhaiPortfolioSnapshot {
    realized_cash_balance: f64,
    equity: f64,
    completed_trades: INT,
    position: Option<RhaiPosition>,
}

impl RhaiPortfolioSnapshot {
    fn from_runtime(snapshot: &RuntimePortfolioSnapshot) -> Self {
        Self {
            realized_cash_balance: snapshot.realized_cash_balance,
            equity: snapshot.current_equity,
            completed_trades: snapshot.completed_trade_count as INT,
            position: snapshot.open_position.clone().map(RhaiPosition::new),
        }
    }
}

#[derive(Debug, Clone)]
struct RhaiPosition {
    position: Position,
}

impl RhaiPosition {
    fn new(position: Position) -> Self {
        Self { position }
    }
}

impl StrategyHandler for RhaiStrategy {
    fn on_market_input_accepted(
        &mut self,
        market_state: &MarketState,
        candle: &Candle,
        primary_timeframe: Timeframe,
    ) {
        let Some(runtime) = &mut self.anchored_runtime else {
            return;
        };
        if candle.timeframe != primary_timeframe {
            return;
        }
        let Some(history) = market_state.history(primary_timeframe) else {
            return;
        };

        runtime.on_market_input_accepted(candle, history);
    }

    fn on_tick(&mut self, input: StrategyTickInput<'_>) -> StrategyTickResult {
        let market = RhaiMarketView::from_runtime(
            &input.market,
            self.anchored_runtime
                .as_ref()
                .map(|runtime| runtime.outputs().clone()),
            self.structure_object_ids.clone(),
        );
        let context =
            RhaiStrategyContext::from_runtime(input.context.portfolio, input.context.state);

        let result = match self.engine.call_fn::<Dynamic>(
            &mut self.scope,
            &self.ast,
            ON_TICK_HOOK,
            (market, context),
        ) {
            Ok(result) => result,
            Err(error) => {
                return StrategyTickResult::Error(StrategyError::new(format!(
                    "strategy hook `on_tick` failed: {error}"
                )))
            }
        };

        let actual_type = result.type_name().to_string();
        match result.try_cast::<StrategyDecision>() {
            Some(decision) => StrategyTickResult::Decision(decision),
            None => StrategyTickResult::Error(StrategyError::new(format!(
                "strategy hook `on_tick` must return typed StrategyDecision from `decision::*`; got {actual_type}"
            ))),
        }
    }
}

impl RhaiStrategy {
    /// Compile and initialize a Rhai strategy from source.
    pub fn load(source: &str) -> Result<Self, RhaiStrategyLoadError> {
        let engine = new_rhai_engine();

        let normalized_source = normalize_reserved_constructor_names(source);
        let ast =
            engine
                .compile(&normalized_source)
                .map_err(|error| RhaiStrategyLoadError::Compile {
                    message: error.to_string(),
                })?;

        validate_required_on_tick(&ast)?;
        validate_required_zero_arg_hook(&ast, STRATEGY_CONFIG_HOOK)?;
        validate_optional_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK)?;
        validate_optional_zero_arg_hook(&ast, STRUCTURE_CONFIG_HOOK)?;
        validate_no_conflicting_structure_hooks(&ast)?;

        let mut scope = Scope::new();
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|error| RhaiStrategyLoadError::Init {
                message: error.to_string(),
            })?;

        let strategy_config = call_typed_strategy_config(&engine, &mut scope, &ast)?;

        let anchored_config = if has_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK) {
            Some(call_typed_anchored_config(&engine, &mut scope, &ast)?)
        } else {
            None
        };
        let structure_config = if has_zero_arg_hook(&ast, STRUCTURE_CONFIG_HOOK) {
            Some(call_typed_structure_config(&engine, &mut scope, &ast)?)
        } else {
            None
        };
        let runtime_config = match (&anchored_config, &structure_config) {
            (Some(config), None) => Some((ANCHORED_CONFIG_HOOK, config.clone())),
            (None, Some(config)) => {
                Some((STRUCTURE_CONFIG_HOOK, config.to_anchored_configuration()))
            }
            (None, None) => None,
            (Some(_), Some(_)) => unreachable!("conflicting hooks are validated before loading"),
        };
        let anchored_runtime = runtime_config
            .as_ref()
            .filter(|(_, config)| !config.is_empty())
            .map(|(hook, config)| {
                AnchoredRuntime::from_config(config).map_err(|error| {
                    RhaiStrategyLoadError::HookEvaluation {
                        hook: *hook,
                        message: error.to_string(),
                    }
                })
            })
            .transpose()?;
        let structure_object_ids = structure_config
            .as_ref()
            .map(StructureConfiguration::object_ids);

        let hooks = RhaiStrategyHooks {
            has_on_tick: true,
            has_strategy_config: has_zero_arg_hook(&ast, STRATEGY_CONFIG_HOOK),
            has_anchored_config: has_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK),
            has_structure_config: has_zero_arg_hook(&ast, STRUCTURE_CONFIG_HOOK),
        };

        Ok(Self {
            engine,
            ast,
            scope,
            hooks,
            strategy_config,
            anchored_config,
            structure_config,
            structure_object_ids,
            anchored_runtime,
        })
    }

    /// Load a Rhai strategy file from disk.
    pub fn load_file(path: impl AsRef<Path>) -> Result<Self, RhaiStrategyLoadError> {
        let source = std::fs::read_to_string(path).map_err(|error| RhaiStrategyLoadError::Io {
            message: error.to_string(),
        })?;
        Self::load(&source)
    }

    pub fn hooks(&self) -> RhaiStrategyHooks {
        self.hooks
    }

    pub fn strategy_config(&self) -> &StrategyConfiguration {
        &self.strategy_config
    }

    pub fn anchored_config(&self) -> Option<&AnchoredConfiguration> {
        self.anchored_config.as_ref()
    }

    pub fn structure_config(&self) -> Option<&StructureConfiguration> {
        self.structure_config.as_ref()
    }

    /// Rhai engine with runtime-owned strategy-facing registrations.
    pub fn engine(&self) -> &RhaiEngine {
        &self.engine
    }

    /// Compiled Rhai AST, kept available for later warmup/indicator detection.
    pub fn ast(&self) -> &AST {
        &self.ast
    }

    /// Persistent Rhai scope populated by top-level declarations.
    pub fn scope(&self) -> &Scope<'static> {
        &self.scope
    }
}

const ON_TICK_HOOK: &str = "on_tick";
const STRATEGY_CONFIG_HOOK: &str = "strategy_config";
const ANCHORED_CONFIG_HOOK: &str = "anchored_config";
const STRUCTURE_CONFIG_HOOK: &str = "structure_config";

/// Load-time errors from Rhai strategy handling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RhaiStrategyLoadError {
    Io {
        message: String,
    },
    Compile {
        message: String,
    },
    Init {
        message: String,
    },
    MissingRequiredHook {
        expected: &'static str,
    },
    InvalidHookSignature {
        hook: &'static str,
        expected: &'static str,
    },
    HookEvaluation {
        hook: &'static str,
        message: String,
    },
    InvalidHookReturn {
        hook: &'static str,
        expected: &'static str,
    },
    InvalidStrategyConfiguration {
        message: String,
    },
    ConflictingHooks {
        first: &'static str,
        second: &'static str,
    },
}

impl fmt::Display for RhaiStrategyLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { message } => write!(formatter, "strategy file read error: {message}"),
            Self::Compile { message } => write!(formatter, "strategy compile error: {message}"),
            Self::Init { message } => write!(formatter, "strategy init error: {message}"),
            Self::MissingRequiredHook { expected } => {
                write!(formatter, "strategy must define required hook `{expected}`")
            }
            Self::InvalidHookSignature { hook, expected } => {
                write!(formatter, "strategy hook `{hook}` must be `{expected}`")
            }
            Self::HookEvaluation { hook, message } => {
                write!(
                    formatter,
                    "strategy hook `{hook}` failed at load time: {message}"
                )
            }
            Self::InvalidHookReturn { hook, expected } => {
                write!(formatter, "strategy hook `{hook}` must return {expected}")
            }
            Self::InvalidStrategyConfiguration { message } => {
                write!(formatter, "strategy configuration error: {message}")
            }
            Self::ConflictingHooks { first, second } => write!(
                formatter,
                "strategy hooks `{first}` and `{second}` are mutually exclusive"
            ),
        }
    }
}

impl Error for RhaiStrategyLoadError {}

impl fmt::Debug for RhaiStrategy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RhaiStrategy")
            .field("hooks", &self.hooks)
            .field("strategy_config", &self.strategy_config)
            .field("anchored_config", &self.anchored_config)
            .field("structure_config", &self.structure_config)
            .finish_non_exhaustive()
    }
}

fn new_rhai_engine() -> RhaiEngine {
    let mut engine = RhaiEngine::new();

    engine.register_type_with_name::<Timeframe>("Timeframe");
    engine.register_type_with_name::<StrategyConfiguration>("StrategyConfig");
    engine.register_type_with_name::<SecondaryTimeframeConfig>("SecondaryConfig");
    engine.register_type_with_name::<AnchoredConfiguration>("AnchoredConfig");
    engine.register_type_with_name::<PivotDetectorConfiguration>("PivotDetectorConfig");
    engine.register_type_with_name::<AnchoredEvaluatorConfiguration>("AnchoredEvaluatorConfig");
    engine.register_type_with_name::<StructureConfiguration>("StructureConfig");
    engine.register_type_with_name::<StructurePointRegistry>("StructurePointRegistry");
    engine.register_type_with_name::<StructureObjectRegistry>("StructureObjectRegistry");
    engine.register_type_with_name::<StructurePointSource>("StructurePointSource");
    engine.register_type_with_name::<StructureObjectConfiguration>("StructureObjectConfig");
    engine.register_type_with_name::<PivotSide>("PivotSide");
    engine.register_type_with_name::<RhaiMarketStructureView>("MarketStructureView");
    engine.register_type_with_name::<RhaiTrendLine>("TrendLine");
    engine.register_type_with_name::<RhaiPivotEvent>("PivotEvent");
    engine.register_type_with_name::<StrategyDecision>("StrategyDecision");
    engine.register_type_with_name::<RhaiMarketView>("MarketView");
    engine.register_type_with_name::<RhaiCandle>("Candle");
    engine.register_type_with_name::<RhaiCandleHistory>("CandleHistory");
    engine.register_type_with_name::<RhaiStrategyContext>("StrategyContext");
    engine.register_type_with_name::<RhaiPortfolioSnapshot>("PortfolioSnapshot");
    engine.register_type_with_name::<RhaiPosition>("Position");
    engine.register_type_with_name::<StrategyState>("StrategyState");

    register_market_view_api(&mut engine);
    register_strategy_context_api(&mut engine);
    register_indicator_api(&mut engine);
    register_anchored_api(&mut engine);

    engine.register_fn("timeframe", parse_timeframe);
    engine.register_fn("==", |left: Timeframe, right: Timeframe| left == right);
    engine.register_fn("with_primary", with_primary);
    engine.register_fn(
        "with_minimum_warmup",
        |config: StrategyConfiguration, minimum_warmup: INT| {
            with_minimum_warmup(config, minimum_warmup)
        },
    );
    engine.register_fn("with_secondary", with_secondary);
    engine.register_fn("with_max_missing_candles", with_max_missing_candles);
    engine.register_fn("with_detector", anchored_config_with_detector);
    engine.register_fn("with_evaluator", anchored_config_with_evaluator);
    engine.register_fn("with_left_bars", pivot_detector_with_left_bars);
    engine.register_fn("with_right_bars", pivot_detector_with_right_bars);
    engine.register_fn("with_side", anchored_evaluator_with_side);
    engine.register_fn("with_pivot_buffer", anchored_evaluator_with_pivot_buffer);
    engine.register_fn("with_tolerance", anchored_evaluator_with_tolerance);
    engine.register_fn("with_min_touches", anchored_evaluator_with_min_touches);
    engine.register_fn("with_max_lines", anchored_evaluator_with_max_lines);
    engine.register_fn("pivots", structure_points_pivots);
    engine.register_fn("trendline", structure_objects_trendline);
    engine.register_fn("with_side", structure_object_with_side);
    engine.register_fn("with_pivot_buffer", structure_object_with_pivot_buffer);
    engine.register_fn("with_tolerance", structure_object_with_tolerance);
    engine.register_fn("with_min_touches", structure_object_with_min_touches);
    engine.register_fn("with_max_active", structure_object_with_max_active);
    engine.register_get("points", |config: &mut StructureConfiguration| {
        config.points()
    });
    engine.register_get("objects", |config: &mut StructureConfiguration| {
        config.objects()
    });

    engine.register_fn("__runtime_strategy_config_new", StrategyConfiguration::new);
    engine.register_fn("__runtime_anchored_config_new", AnchoredConfiguration::new);
    engine.register_fn(
        "__runtime_structure_config_new",
        StructureConfiguration::new,
    );
    engine.register_fn("__runtime_pivot_detector_new", pivot_detector_new);

    let mut strategy_config_module = Module::new();
    strategy_config_module.set_native_fn("new", || Ok(StrategyConfiguration::new()));
    engine.register_static_module("strategy_config", Arc::new(strategy_config_module));

    let mut secondary_module = Module::new();
    secondary_module.set_native_fn("required", |timeframe: Timeframe| {
        Ok(SecondaryTimeframeConfig::required(timeframe, 0))
    });
    secondary_module.set_native_fn("optional", |timeframe: Timeframe| {
        Ok(SecondaryTimeframeConfig::optional(timeframe, 0))
    });
    engine.register_static_module("secondary", Arc::new(secondary_module));

    let mut anchored_config_module = Module::new();
    anchored_config_module.set_native_fn("new", || Ok(AnchoredConfiguration::new()));
    engine.register_static_module("anchored_config", Arc::new(anchored_config_module));

    let mut structure_config_module = Module::new();
    structure_config_module.set_native_fn("new", || Ok(StructureConfiguration::new()));
    engine.register_static_module("structure_config", Arc::new(structure_config_module));

    let mut pivot_detector_module = Module::new();
    pivot_detector_module.set_native_fn("new", |id: &str| Ok(PivotDetectorConfiguration::new(id)));
    engine.register_static_module("pivot_detector", Arc::new(pivot_detector_module));

    let mut pivot_side_module = Module::new();
    pivot_side_module.set_native_fn("high", || Ok(PivotSide::high()));
    pivot_side_module.set_native_fn("low", || Ok(PivotSide::low()));
    engine.register_static_module("pivot_side", Arc::new(pivot_side_module));

    let mut structure_side_module = Module::new();
    structure_side_module.set_native_fn("high", || Ok(PivotSide::high()));
    structure_side_module.set_native_fn("low", || Ok(PivotSide::low()));
    engine.register_static_module("structure_side", Arc::new(structure_side_module));

    let mut anchored_module = Module::new();
    anchored_module.set_native_fn("trendline", |expose_as: &str, pivot_source: &str| {
        Ok(AnchoredEvaluatorConfiguration::trendline(
            expose_as,
            pivot_source,
        ))
    });
    engine.register_static_module("anchored", Arc::new(anchored_module));

    let mut decision_module = Module::new();
    decision_module.set_native_fn("hold", || Ok(StrategyDecision::hold()));
    decision_module.set_native_fn("open_long", |quantity: FLOAT| {
        Ok(StrategyDecision::open_long(quantity))
    });
    decision_module.set_native_fn("open_long", |quantity: INT| {
        Ok(StrategyDecision::open_long(quantity as f64))
    });
    decision_module.set_native_fn("close_long", || Ok(StrategyDecision::close_long()));
    decision_module.set_native_fn("open_short", |quantity: FLOAT| {
        Ok(StrategyDecision::open_short(quantity))
    });
    decision_module.set_native_fn("open_short", |quantity: INT| {
        Ok(StrategyDecision::open_short(quantity as f64))
    });
    decision_module.set_native_fn("close_short", || Ok(StrategyDecision::close_short()));
    engine.register_static_module("decision", Arc::new(decision_module));

    engine.register_fn("with_stop_loss", with_stop_loss);
    engine.register_fn("with_take_profit", with_take_profit);
    engine.register_fn("with_reason", with_reason);

    engine
}

fn normalize_reserved_constructor_names(source: &str) -> String {
    // Rhai still reserves `new` in module paths such as `strategy_config::new()`
    // as of the locked 1.25.x dependency. Keep the strategy-facing API from ADR
    // 0005 and lower only these approved typed constructors to private runtime
    // function names before compilation. This is intentionally lexical enough to
    // avoid rewriting string literals or comments.
    const REPLACEMENTS: [(&str, &str); 4] = [
        ("strategy_config::new(", "__runtime_strategy_config_new("),
        ("anchored_config::new(", "__runtime_anchored_config_new("),
        ("structure_config::new(", "__runtime_structure_config_new("),
        ("pivot_detector::new(", "__runtime_pivot_detector_new("),
    ];

    let mut output = String::with_capacity(source.len());
    let mut index = 0;

    while index < source.len() {
        let remaining = &source[index..];

        if let Some((from, to)) = REPLACEMENTS
            .iter()
            .find(|(from, _)| remaining.starts_with(from))
        {
            output.push_str(to);
            index += from.len();
            continue;
        }

        if remaining.starts_with("//") {
            let next = copy_until_line_end(source, index, &mut output);
            index = next;
            continue;
        }

        if remaining.starts_with("/*") {
            let next = copy_until_block_comment_end(source, index, &mut output);
            index = next;
            continue;
        }

        if remaining.starts_with('"') {
            let next = copy_until_string_end(source, index, &mut output);
            index = next;
            continue;
        }

        let character = remaining
            .chars()
            .next()
            .expect("remaining source should contain a character");
        output.push(character);
        index += character.len_utf8();
    }

    output
}

fn copy_until_line_end(source: &str, start: usize, output: &mut String) -> usize {
    let end = source[start..]
        .find('\n')
        .map(|offset| start + offset + 1)
        .unwrap_or(source.len());
    output.push_str(&source[start..end]);
    end
}

fn copy_until_block_comment_end(source: &str, start: usize, output: &mut String) -> usize {
    let end = source[start + 2..]
        .find("*/")
        .map(|offset| start + 2 + offset + 2)
        .unwrap_or(source.len());
    output.push_str(&source[start..end]);
    end
}

fn copy_until_string_end(source: &str, start: usize, output: &mut String) -> usize {
    let mut escaped = false;
    let mut end = source.len();

    for (offset, character) in source[start..].char_indices().skip(1) {
        if escaped {
            escaped = false;
            continue;
        }

        match character {
            '\\' => escaped = true,
            '"' => {
                end = start + offset + character.len_utf8();
                break;
            }
            _ => {}
        }
    }

    output.push_str(&source[start..end]);
    end
}

fn register_strategy_context_api(engine: &mut RhaiEngine) {
    engine.register_get("portfolio", |context: &mut RhaiStrategyContext| {
        context.portfolio.clone()
    });
    engine.register_get("state", |context: &mut RhaiStrategyContext| {
        context.state.clone()
    });

    engine.register_get(
        "realized_cash_balance",
        |portfolio: &mut RhaiPortfolioSnapshot| portfolio.realized_cash_balance,
    );
    engine.register_get("equity", |portfolio: &mut RhaiPortfolioSnapshot| {
        portfolio.equity
    });
    engine.register_get(
        "completed_trades",
        |portfolio: &mut RhaiPortfolioSnapshot| portfolio.completed_trades,
    );
    engine.register_get(
        "position",
        |portfolio: &mut RhaiPortfolioSnapshot| -> Dynamic {
            portfolio
                .position
                .clone()
                .map(Dynamic::from)
                .unwrap_or(Dynamic::UNIT)
        },
    );
    engine.register_fn("is_flat", |portfolio: &mut RhaiPortfolioSnapshot| {
        portfolio.position.is_none()
    });
    engine.register_fn("has_position", |portfolio: &mut RhaiPortfolioSnapshot| {
        portfolio.position.is_some()
    });
    engine.register_fn("is_long", |portfolio: &mut RhaiPortfolioSnapshot| {
        portfolio
            .position
            .as_ref()
            .map(|position| position.position.side == PositionSide::Long)
            .unwrap_or(false)
    });
    engine.register_fn("is_short", |portfolio: &mut RhaiPortfolioSnapshot| {
        portfolio
            .position
            .as_ref()
            .map(|position| position.position.side == PositionSide::Short)
            .unwrap_or(false)
    });

    engine.register_get("side", |position: &mut RhaiPosition| {
        position.position.side.to_string()
    });
    engine.register_get("entry_price", |position: &mut RhaiPosition| {
        position.position.entry_price
    });
    engine.register_get("size", |position: &mut RhaiPosition| position.position.size);
    engine.register_get("entry_time", |position: &mut RhaiPosition| {
        position.position.entry_time
    });
    engine.register_get("stop_loss", |position: &mut RhaiPosition| -> Dynamic {
        position
            .position
            .stop_loss
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });
    engine.register_get("take_profit", |position: &mut RhaiPosition| -> Dynamic {
        position
            .position
            .take_profit
            .map(Dynamic::from)
            .unwrap_or(Dynamic::UNIT)
    });
    engine.register_fn("is_long", |position: &mut RhaiPosition| {
        position.position.side == PositionSide::Long
    });
    engine.register_fn("is_short", |position: &mut RhaiPosition| {
        position.position.side == PositionSide::Short
    });
    engine.register_fn("has_stop_loss", |position: &mut RhaiPosition| {
        position.position.stop_loss.is_some()
    });
    engine.register_fn("has_take_profit", |position: &mut RhaiPosition| {
        position.position.take_profit.is_some()
    });

    engine.register_fn("get", strategy_state_get_int);
    engine.register_fn("get", strategy_state_get_float);
    engine.register_fn("get", strategy_state_get_bool);
    engine.register_fn("get", strategy_state_get_string);
    engine.register_fn("int", strategy_state_get_int);
    engine.register_fn("float", strategy_state_get_float);
    engine.register_fn("bool", strategy_state_get_bool);
    engine.register_fn("string", strategy_state_get_string);
    engine.register_fn("set", strategy_state_set_int);
    engine.register_fn("set", strategy_state_set_float);
    engine.register_fn("set", strategy_state_set_bool);
    engine.register_fn("set", strategy_state_set_string);
    engine.register_fn("set_int", strategy_state_set_int);
    engine.register_fn("set_float", strategy_state_set_float);
    engine.register_fn("set_bool", strategy_state_set_bool);
    engine.register_fn("set_string", strategy_state_set_string);
}

fn parse_timeframe(raw: &str) -> Result<Timeframe, Box<EvalAltResult>> {
    raw.parse::<Timeframe>()
        .map_err(|error| format!("invalid timeframe `{raw}`: {error}").into())
}

fn register_market_view_api(engine: &mut RhaiEngine) {
    engine.register_get("structure", |market: &mut RhaiMarketView| {
        RhaiMarketStructureView {
            outputs: market.anchored_outputs.clone(),
            object_ids: market.structure_object_ids.clone(),
        }
    });
    engine.register_fn(
        "active",
        |structure: &mut RhaiMarketStructureView, object_id: &str| structure.active(object_id),
    );

    engine.register_fn("candle", |market: &mut RhaiMarketView| {
        market.current.clone()
    });
    engine.register_fn(
        "candle",
        |market: &mut RhaiMarketView,
         timeframe: Timeframe|
         -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(market
                .visible_history(timeframe)?
                .and_then(|history| history.latest_candle())
                .map(Dynamic::from)
                .unwrap_or(Dynamic::UNIT))
        },
    );
    engine.register_fn("candles", |market: &mut RhaiMarketView| {
        market.primary_history.clone()
    });
    engine.register_fn(
        "candles",
        |market: &mut RhaiMarketView,
         timeframe: Timeframe|
         -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(market
                .visible_history(timeframe)?
                .map(Dynamic::from)
                .unwrap_or(Dynamic::UNIT))
        },
    );

    engine.register_get("open", |candle: &mut RhaiCandle| candle.candle.open);
    engine.register_get("high", |candle: &mut RhaiCandle| candle.candle.high);
    engine.register_get("low", |candle: &mut RhaiCandle| candle.candle.low);
    engine.register_get("close", |candle: &mut RhaiCandle| candle.candle.close);
    engine.register_get("volume", |candle: &mut RhaiCandle| candle.candle.volume);
    engine.register_get("timestamp", |candle: &mut RhaiCandle| {
        candle.candle.timestamp as INT
    });
    engine.register_get("symbol", |candle: &mut RhaiCandle| {
        candle.candle.symbol.clone()
    });
    engine.register_get("timeframe", |candle: &mut RhaiCandle| {
        candle.candle.timeframe
    });
    engine.register_fn("body", |candle: &mut RhaiCandle| candle.candle.body());
    engine.register_fn("range", |candle: &mut RhaiCandle| candle.candle.range());

    engine.register_indexer_get(|history: &mut RhaiCandleHistory, index: INT| -> Dynamic {
        if index < 1 {
            return Dynamic::UNIT;
        }

        let len = history.candles.len();
        match len.checked_sub(index as usize) {
            Some(position) => Dynamic::from(RhaiCandle::new(history.candles[position].clone())),
            None => Dynamic::UNIT,
        }
    });
    engine.register_fn("len", |history: &mut RhaiCandleHistory| -> INT {
        history.candles.len() as INT
    });
}

fn trendlines_to_rhai_array(output: &AnchoredOutput) -> rhai::Array {
    match output {
        AnchoredOutput::Trendlines(lines) => lines
            .iter()
            .copied()
            .map(|line| Dynamic::from(RhaiTrendLine::new(line)))
            .collect(),
    }
}

fn register_anchored_api(engine: &mut RhaiEngine) {
    engine.register_fn(
        "anchored",
        |market: &mut RhaiMarketView, name: &str| -> Dynamic {
            let Some(outputs) = &market.anchored_outputs else {
                return Dynamic::UNIT;
            };

            match outputs.values.get(name) {
                Some(output) => Dynamic::from(trendlines_to_rhai_array(output)),
                None => Dynamic::UNIT,
            }
        },
    );
    engine.register_fn(
        "last_pivot",
        |market: &mut RhaiMarketView, detector_id: &str, side: PivotSide| -> Dynamic {
            let Some(outputs) = &market.anchored_outputs else {
                return Dynamic::UNIT;
            };

            let event = match side {
                PivotSide::High => outputs.last_pivot_high.get(detector_id),
                PivotSide::Low => outputs.last_pivot_low.get(detector_id),
            };

            event
                .copied()
                .map(|event| Dynamic::from(RhaiPivotEvent::new(event)))
                .unwrap_or(Dynamic::UNIT)
        },
    );

    engine.register_get("slope", |line: &mut RhaiTrendLine| line.line.slope);
    engine.register_get("intercept", |line: &mut RhaiTrendLine| line.line.intercept);
    engine.register_get("touches", |line: &mut RhaiTrendLine| {
        line.line.touches as INT
    });
    engine.register_get("anchor_start_bar", |line: &mut RhaiTrendLine| {
        line.line.anchor_start_bar as INT
    });
    engine.register_get("anchor_end_bar", |line: &mut RhaiTrendLine| {
        line.line.anchor_end_bar as INT
    });
    engine.register_get("side", |line: &mut RhaiTrendLine| match line.line.side {
        indicators::anchored::evaluators::TrendlineSide::Resistance => "resistance".to_string(),
        indicators::anchored::evaluators::TrendlineSide::Support => "support".to_string(),
    });
    engine.register_fn("y_at", |line: &mut RhaiTrendLine, bar: INT| {
        line.line.y_at(bar.max(0) as u64)
    });

    engine.register_get("bar", |pivot: &mut RhaiPivotEvent| pivot.event.bar as INT);
    engine.register_get("price", |pivot: &mut RhaiPivotEvent| pivot.event.price);
    engine.register_get("volume", |pivot: &mut RhaiPivotEvent| pivot.event.volume);
    engine.register_get("side", |pivot: &mut RhaiPivotEvent| {
        match pivot.event.side {
            PivotSide::High => "high".to_string(),
            PivotSide::Low => "low".to_string(),
        }
    });
}

fn register_indicator_api(engine: &mut RhaiEngine) {
    engine.register_static_module("ta", Arc::new(indicator_module(true)));
    engine.register_static_module("indicators", Arc::new(indicator_module(false)));
}

fn indicator_module(include_cross_helpers: bool) -> Module {
    let mut module = Module::new();

    register_period_close_indicator(&mut module, "sma", sma);
    register_period_close_indicator(&mut module, "ema", ema);
    register_period_close_indicator(&mut module, "dema", dema);
    register_period_close_indicator(&mut module, "tema", tema);
    register_period_close_indicator(&mut module, "slope", slope);
    register_period_close_indicator(&mut module, "rsi", rsi);
    register_period_close_indicator(&mut module, "roc", roc);

    register_period_candle_indicator(&mut module, "cci", cci);
    register_period_candle_indicator(&mut module, "williams_r", williams_r);
    register_period_candle_indicator(&mut module, "atr", atr);
    register_period_candle_indicator(&mut module, "mfi", mfi);

    if include_cross_helpers {
        register_cross_helpers(&mut module);
    }

    module.set_native_fn(
        "obv",
        |history: &mut RhaiCandleHistory| -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(option_f64(obv(
                history.chronological_candles_before_offset(0)
            )))
        },
    );
    module.set_native_fn(
        "obv",
        |history: &mut RhaiCandleHistory, offset: INT| -> Result<Dynamic, Box<EvalAltResult>> {
            let Some(offset) = non_negative_usize(offset) else {
                return Ok(Dynamic::UNIT);
            };
            Ok(option_f64(obv(
                history.chronological_candles_before_offset(offset)
            )))
        },
    );

    module
}

fn register_cross_helpers(module: &mut Module) {
    module.set_native_fn(
        "cross_over",
        |previous_a: FLOAT,
         previous_b: FLOAT,
         current_a: FLOAT,
         current_b: FLOAT|
         -> Result<bool, Box<EvalAltResult>> {
            Ok(previous_a <= previous_b && current_a > current_b)
        },
    );
    module.set_native_fn(
        "cross_under",
        |previous_a: FLOAT,
         previous_b: FLOAT,
         current_a: FLOAT,
         current_b: FLOAT|
         -> Result<bool, Box<EvalAltResult>> {
            Ok(previous_a >= previous_b && current_a < current_b)
        },
    );
}

fn register_period_close_indicator(
    module: &mut Module,
    name: &'static str,
    indicator: fn(&[f64], usize) -> Option<f64>,
) {
    module.set_native_fn(
        name,
        move |history: &mut RhaiCandleHistory,
              period: INT|
              -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(period_close_indicator(history, period, 0, indicator))
        },
    );
    module.set_native_fn(
        name,
        move |history: &mut RhaiCandleHistory,
              period: INT,
              offset: INT|
              -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(period_close_indicator(history, period, offset, indicator))
        },
    );
}

fn register_period_candle_indicator(
    module: &mut Module,
    name: &'static str,
    indicator: fn(&[Candle], usize) -> Option<f64>,
) {
    module.set_native_fn(
        name,
        move |history: &mut RhaiCandleHistory,
              period: INT|
              -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(period_candle_indicator(history, period, 0, indicator))
        },
    );
    module.set_native_fn(
        name,
        move |history: &mut RhaiCandleHistory,
              period: INT,
              offset: INT|
              -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(period_candle_indicator(history, period, offset, indicator))
        },
    );
}

fn period_close_indicator(
    history: &RhaiCandleHistory,
    period: INT,
    offset: INT,
    indicator: fn(&[f64], usize) -> Option<f64>,
) -> Dynamic {
    let (Some(period), Some(offset)) = (non_negative_usize(period), non_negative_usize(offset))
    else {
        return Dynamic::UNIT;
    };
    let closes = history.chronological_closes_before_offset(offset);
    option_f64(indicator(&closes, period))
}

fn period_candle_indicator(
    history: &RhaiCandleHistory,
    period: INT,
    offset: INT,
    indicator: fn(&[Candle], usize) -> Option<f64>,
) -> Dynamic {
    let (Some(period), Some(offset)) = (non_negative_usize(period), non_negative_usize(offset))
    else {
        return Dynamic::UNIT;
    };
    option_f64(indicator(
        history.chronological_candles_before_offset(offset),
        period,
    ))
}

fn option_f64(value: Option<f64>) -> Dynamic {
    value.map(Dynamic::from).unwrap_or(Dynamic::UNIT)
}

fn non_negative_usize(value: INT) -> Option<usize> {
    usize::try_from(value).ok()
}

fn strategy_state_get_int(
    state: StrategyState,
    key: &str,
    default: INT,
) -> Result<INT, Box<EvalAltResult>> {
    match state.get(key) {
        Some(StrategyStateValue::Int(value)) => Ok(value),
        Some(value) => Err(strategy_state_type_error(key, "int", value)),
        None => Ok(default),
    }
}

fn strategy_state_get_float(
    state: StrategyState,
    key: &str,
    default: FLOAT,
) -> Result<FLOAT, Box<EvalAltResult>> {
    match state.get(key) {
        Some(StrategyStateValue::Float(value)) => Ok(value),
        Some(value) => Err(strategy_state_type_error(key, "float", value)),
        None => Ok(default),
    }
}

fn strategy_state_get_bool(
    state: StrategyState,
    key: &str,
    default: bool,
) -> Result<bool, Box<EvalAltResult>> {
    match state.get(key) {
        Some(StrategyStateValue::Bool(value)) => Ok(value),
        Some(value) => Err(strategy_state_type_error(key, "bool", value)),
        None => Ok(default),
    }
}

fn strategy_state_get_string(
    state: StrategyState,
    key: &str,
    default: &str,
) -> Result<String, Box<EvalAltResult>> {
    match state.get(key) {
        Some(StrategyStateValue::String(value)) => Ok(value),
        Some(value) => Err(strategy_state_type_error(key, "string", value)),
        None => Ok(default.to_string()),
    }
}

fn strategy_state_set_int(state: StrategyState, key: &str, value: INT) {
    state.set(key, StrategyStateValue::Int(value));
}

fn strategy_state_set_float(state: StrategyState, key: &str, value: FLOAT) {
    state.set(key, StrategyStateValue::Float(value));
}

fn strategy_state_set_bool(state: StrategyState, key: &str, value: bool) {
    state.set(key, StrategyStateValue::Bool(value));
}

fn strategy_state_set_string(state: StrategyState, key: &str, value: &str) {
    state.set(key, StrategyStateValue::String(value.to_string()));
}

fn strategy_state_type_error(
    key: &str,
    expected: &'static str,
    actual: StrategyStateValue,
) -> Box<EvalAltResult> {
    let actual = match actual {
        StrategyStateValue::Int(_) => "int",
        StrategyStateValue::Float(_) => "float",
        StrategyStateValue::Bool(_) => "bool",
        StrategyStateValue::String(_) => "string",
    };
    format!("strategy state key `{key}` contains {actual}, not requested {expected}").into()
}

fn with_primary(
    config: StrategyConfiguration,
    primary_timeframe: Timeframe,
) -> StrategyConfiguration {
    config.with_primary(primary_timeframe)
}

fn with_minimum_warmup(
    config: StrategyConfiguration,
    minimum_warmup: INT,
) -> Result<StrategyConfiguration, Box<EvalAltResult>> {
    let minimum_warmup = usize::try_from(minimum_warmup)
        .map_err(|_| "minimum warmup must be a non-negative integer".to_string())?;

    Ok(config.with_minimum_warmup(minimum_warmup))
}

fn with_secondary(
    config: StrategyConfiguration,
    secondary: SecondaryTimeframeConfig,
) -> StrategyConfiguration {
    config.with_secondary(secondary)
}

fn with_max_missing_candles(
    mut secondary: SecondaryTimeframeConfig,
    max_missing_candles: INT,
) -> Result<SecondaryTimeframeConfig, Box<EvalAltResult>> {
    secondary.max_missing_candles = u32::try_from(max_missing_candles).map_err(|_| {
        "max missing candles must be a non-negative integer fitting u32".to_string()
    })?;

    Ok(secondary)
}

fn pivot_detector_new(id: &str) -> PivotDetectorConfiguration {
    PivotDetectorConfiguration::new(id)
}

fn structure_points_pivots(
    points: StructurePointRegistry,
    id: &str,
    left_bars: INT,
    right_bars: INT,
) -> Result<StructurePointSource, Box<EvalAltResult>> {
    let left_bars = positive_usize(left_bars, "left_bars")?;
    let right_bars = positive_usize(right_bars, "right_bars")?;
    points
        .pivots(id, left_bars, right_bars)
        .map_err(|error| error.to_string().into())
}

fn structure_objects_trendline(
    objects: StructureObjectRegistry,
    object_id: &str,
    point_source: StructurePointSource,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    objects
        .trendline(object_id, point_source)
        .map_err(|error| error.to_string().into())
}

fn structure_object_with_side(
    object: StructureObjectConfiguration,
    side: PivotSide,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    object
        .with_side(side)
        .map_err(|error| error.to_string().into())
}

fn structure_object_with_pivot_buffer(
    object: StructureObjectConfiguration,
    pivot_buffer: INT,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    let pivot_buffer = positive_usize(pivot_buffer, "pivot_buffer")?;
    if pivot_buffer < 3 {
        return Err("pivot_buffer must be >= 3".into());
    }
    object
        .with_pivot_buffer(pivot_buffer)
        .map_err(|error| error.to_string().into())
}

fn structure_object_with_tolerance(
    object: StructureObjectConfiguration,
    tolerance: FLOAT,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    if !(tolerance.is_finite() && tolerance > 0.0 && tolerance < 0.5) {
        return Err("tolerance must be finite and in (0.0, 0.5)".into());
    }
    object
        .with_tolerance(tolerance)
        .map_err(|error| error.to_string().into())
}

fn structure_object_with_min_touches(
    object: StructureObjectConfiguration,
    min_touches: INT,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    let min_touches = u32::try_from(min_touches)
        .map_err(|_| "min_touches must be a non-negative integer fitting u32".to_string())?;
    if min_touches < 3 {
        return Err("min_touches must be >= 3".into());
    }
    object
        .with_min_touches(min_touches)
        .map_err(|error| error.to_string().into())
}

fn structure_object_with_max_active(
    object: StructureObjectConfiguration,
    max_active: INT,
) -> Result<StructureObjectConfiguration, Box<EvalAltResult>> {
    let max_active = positive_usize(max_active, "max_active")?;
    object
        .with_max_active(max_active)
        .map_err(|error| error.to_string().into())
}

fn anchored_config_with_detector(
    config: AnchoredConfiguration,
    detector: PivotDetectorConfiguration,
) -> Result<AnchoredConfiguration, Box<EvalAltResult>> {
    ensure_non_empty_label(detector_id(&detector), "pivot detector id")?;
    Ok(config.with_detector(detector))
}

fn anchored_config_with_evaluator(
    config: AnchoredConfiguration,
    evaluator: AnchoredEvaluatorConfiguration,
) -> Result<AnchoredConfiguration, Box<EvalAltResult>> {
    ensure_non_empty_label(evaluator_name(&evaluator), "anchored evaluator name")?;
    ensure_non_empty_label(
        evaluator_source(&evaluator),
        "anchored evaluator pivot source",
    )?;
    Ok(config.with_evaluator(evaluator))
}

fn pivot_detector_with_left_bars(
    detector: PivotDetectorConfiguration,
    left_bars: INT,
) -> Result<PivotDetectorConfiguration, Box<EvalAltResult>> {
    let left_bars = positive_usize(left_bars, "left_bars")?;
    Ok(detector.with_left_bars(left_bars))
}

fn pivot_detector_with_right_bars(
    detector: PivotDetectorConfiguration,
    right_bars: INT,
) -> Result<PivotDetectorConfiguration, Box<EvalAltResult>> {
    let right_bars = positive_usize(right_bars, "right_bars")?;
    Ok(detector.with_right_bars(right_bars))
}

fn anchored_evaluator_with_side(
    evaluator: AnchoredEvaluatorConfiguration,
    side: PivotSide,
) -> AnchoredEvaluatorConfiguration {
    evaluator.with_side(side)
}

fn anchored_evaluator_with_pivot_buffer(
    evaluator: AnchoredEvaluatorConfiguration,
    pivot_buffer: INT,
) -> Result<AnchoredEvaluatorConfiguration, Box<EvalAltResult>> {
    let pivot_buffer = positive_usize(pivot_buffer, "pivot_buffer")?;
    if pivot_buffer < 3 {
        return Err("pivot_buffer must be >= 3".into());
    }
    Ok(evaluator.with_pivot_buffer(pivot_buffer))
}

fn anchored_evaluator_with_tolerance(
    evaluator: AnchoredEvaluatorConfiguration,
    tolerance: FLOAT,
) -> Result<AnchoredEvaluatorConfiguration, Box<EvalAltResult>> {
    if !(tolerance.is_finite() && tolerance > 0.0 && tolerance < 0.5) {
        return Err("tolerance must be finite and in (0.0, 0.5)".into());
    }
    Ok(evaluator.with_tolerance(tolerance))
}

fn anchored_evaluator_with_min_touches(
    evaluator: AnchoredEvaluatorConfiguration,
    min_touches: INT,
) -> Result<AnchoredEvaluatorConfiguration, Box<EvalAltResult>> {
    let min_touches = u32::try_from(min_touches)
        .map_err(|_| "min_touches must be a non-negative integer fitting u32".to_string())?;
    if min_touches < 3 {
        return Err("min_touches must be >= 3".into());
    }
    Ok(evaluator.with_min_touches(min_touches))
}

fn anchored_evaluator_with_max_lines(
    evaluator: AnchoredEvaluatorConfiguration,
    max_lines: INT,
) -> Result<AnchoredEvaluatorConfiguration, Box<EvalAltResult>> {
    let max_lines = positive_usize(max_lines, "max_lines")?;
    Ok(evaluator.with_max_lines(max_lines))
}

fn positive_usize(value: INT, name: &str) -> Result<usize, Box<EvalAltResult>> {
    let value = usize::try_from(value).map_err(|_| format!("{name} must be a positive integer"))?;
    if value == 0 {
        return Err(format!("{name} must be a positive integer").into());
    }
    Ok(value)
}

fn ensure_non_empty_label(value: &str, name: &str) -> Result<(), Box<EvalAltResult>> {
    if value.is_empty() {
        Err(format!("{name} must not be empty").into())
    } else {
        Ok(())
    }
}

fn detector_id(detector: &PivotDetectorConfiguration) -> &str {
    detector.id()
}

fn evaluator_name(evaluator: &AnchoredEvaluatorConfiguration) -> &str {
    evaluator.expose_as()
}

fn evaluator_source(evaluator: &AnchoredEvaluatorConfiguration) -> &str {
    evaluator.pivot_source()
}

fn with_stop_loss(
    decision: StrategyDecision,
    stop_loss: FLOAT,
) -> Result<StrategyDecision, Box<EvalAltResult>> {
    ensure_opening_decision(&decision, "with_stop_loss")?;
    let take_profit = decision.take_profit;

    Ok(decision.with_entry_risk(Some(stop_loss), take_profit))
}

fn with_take_profit(
    decision: StrategyDecision,
    take_profit: FLOAT,
) -> Result<StrategyDecision, Box<EvalAltResult>> {
    ensure_opening_decision(&decision, "with_take_profit")?;
    let stop_loss = decision.stop_loss;

    Ok(decision.with_entry_risk(stop_loss, Some(take_profit)))
}

fn with_reason(decision: StrategyDecision, reason: &str) -> StrategyDecision {
    decision.with_reason(reason)
}

fn ensure_opening_decision(
    decision: &StrategyDecision,
    method: &'static str,
) -> Result<(), Box<EvalAltResult>> {
    if decision.intent.opens_position() {
        Ok(())
    } else {
        Err(format!(
            "`{method}` is only valid on opening decisions; got {:?}",
            decision.intent
        )
        .into())
    }
}

fn validate_required_on_tick(ast: &AST) -> Result<(), RhaiStrategyLoadError> {
    if has_hook_with_arity(ast, ON_TICK_HOOK, 2) {
        return Ok(());
    }

    if has_any_hook(ast, ON_TICK_HOOK) {
        return Err(RhaiStrategyLoadError::InvalidHookSignature {
            hook: ON_TICK_HOOK,
            expected: "fn on_tick(market, context)",
        });
    }

    Err(RhaiStrategyLoadError::MissingRequiredHook {
        expected: "fn on_tick(market, context)",
    })
}

fn validate_required_zero_arg_hook(
    ast: &AST,
    hook: &'static str,
) -> Result<(), RhaiStrategyLoadError> {
    if has_zero_arg_hook(ast, hook) {
        return Ok(());
    }

    if has_any_hook(ast, hook) {
        return Err(RhaiStrategyLoadError::InvalidHookSignature {
            hook,
            expected: match hook {
                STRATEGY_CONFIG_HOOK => "fn strategy_config()",
                ANCHORED_CONFIG_HOOK => "fn anchored_config()",
                STRUCTURE_CONFIG_HOOK => "fn structure_config()",
                _ => "fn hook()",
            },
        });
    }

    Err(RhaiStrategyLoadError::MissingRequiredHook {
        expected: match hook {
            STRATEGY_CONFIG_HOOK => "fn strategy_config()",
            ANCHORED_CONFIG_HOOK => "fn anchored_config()",
            STRUCTURE_CONFIG_HOOK => "fn structure_config()",
            _ => "fn hook()",
        },
    })
}

fn validate_optional_zero_arg_hook(
    ast: &AST,
    hook: &'static str,
) -> Result<(), RhaiStrategyLoadError> {
    if !has_any_hook(ast, hook) || has_zero_arg_hook(ast, hook) {
        return Ok(());
    }

    Err(RhaiStrategyLoadError::InvalidHookSignature {
        hook,
        expected: match hook {
            STRATEGY_CONFIG_HOOK => "fn strategy_config()",
            ANCHORED_CONFIG_HOOK => "fn anchored_config()",
            STRUCTURE_CONFIG_HOOK => "fn structure_config()",
            _ => "fn hook()",
        },
    })
}

fn validate_no_conflicting_structure_hooks(ast: &AST) -> Result<(), RhaiStrategyLoadError> {
    if has_zero_arg_hook(ast, ANCHORED_CONFIG_HOOK) && has_zero_arg_hook(ast, STRUCTURE_CONFIG_HOOK)
    {
        Err(RhaiStrategyLoadError::ConflictingHooks {
            first: ANCHORED_CONFIG_HOOK,
            second: STRUCTURE_CONFIG_HOOK,
        })
    } else {
        Ok(())
    }
}

fn call_typed_strategy_config(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
) -> Result<StrategyConfiguration, RhaiStrategyLoadError> {
    let result = call_load_time_hook(engine, scope, ast, STRATEGY_CONFIG_HOOK)?;
    let config = result.try_cast::<StrategyConfiguration>().ok_or(
        RhaiStrategyLoadError::InvalidHookReturn {
            hook: STRATEGY_CONFIG_HOOK,
            expected: "a typed StrategyConfig from `strategy_config::new()`",
        },
    )?;
    config.validate_timeframe_contract().map_err(|error| {
        RhaiStrategyLoadError::InvalidStrategyConfiguration {
            message: error.to_string(),
        }
    })?;
    Ok(config)
}

fn call_typed_anchored_config(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
) -> Result<AnchoredConfiguration, RhaiStrategyLoadError> {
    let result = call_load_time_hook(engine, scope, ast, ANCHORED_CONFIG_HOOK)?;
    let config = result.try_cast::<AnchoredConfiguration>().ok_or(
        RhaiStrategyLoadError::InvalidHookReturn {
            hook: ANCHORED_CONFIG_HOOK,
            expected: "a typed AnchoredConfig from `anchored_config::new()`",
        },
    )?;
    config
        .validate()
        .map_err(|error| RhaiStrategyLoadError::HookEvaluation {
            hook: ANCHORED_CONFIG_HOOK,
            message: error.to_string(),
        })?;
    Ok(config)
}

fn call_typed_structure_config(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
) -> Result<StructureConfiguration, RhaiStrategyLoadError> {
    let result = call_load_time_hook(engine, scope, ast, STRUCTURE_CONFIG_HOOK)?;
    let config = result.try_cast::<StructureConfiguration>().ok_or(
        RhaiStrategyLoadError::InvalidHookReturn {
            hook: STRUCTURE_CONFIG_HOOK,
            expected: "a typed StructureConfig from `structure_config::new()`",
        },
    )?;
    config
        .validate()
        .map_err(|error| RhaiStrategyLoadError::HookEvaluation {
            hook: STRUCTURE_CONFIG_HOOK,
            message: error.to_string(),
        })?;
    config.seal();
    Ok(config)
}

fn call_load_time_hook(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
    hook: &'static str,
) -> Result<Dynamic, RhaiStrategyLoadError> {
    engine
        .call_fn(scope, ast, hook, ())
        .map_err(|error| RhaiStrategyLoadError::HookEvaluation {
            hook,
            message: error.to_string(),
        })
}

fn has_zero_arg_hook(ast: &AST, name: &str) -> bool {
    has_hook_with_arity(ast, name, 0)
}

fn has_any_hook(ast: &AST, name: &str) -> bool {
    ast.iter_functions().any(|function| function.name == name)
}

fn has_hook_with_arity(ast: &AST, name: &str, arity: usize) -> bool {
    ast.iter_functions()
        .any(|function| function.name == name && function.params.len() == arity)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AnchoredEvaluatorSpec, ExecutionAction, IgnoredDecisionReason, MarketInput, PortfolioState,
        RuntimeConfig, RuntimeEvent, StrategyDecisionIntent, TradingRuntime,
    };
    use shared::{Candle, Timeframe};

    const MINIMAL: &str = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

    fn candle(close: f64) -> Candle {
        candle_at(close, 1, Timeframe::minutes(1))
    }

    fn candle_at(close: f64, timestamp: i64, timeframe: Timeframe) -> Candle {
        candle_ohlc_at(close, close, close, close, timestamp, timeframe)
    }

    fn candle_ohlc_at(
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        timestamp: i64,
        timeframe: Timeframe,
    ) -> Candle {
        Candle {
            timestamp,
            symbol: "BTC-USD".into(),
            open,
            high,
            low,
            close,
            volume: 1_000.0,
            timeframe,
        }
    }

    fn source_returning(expression: &str) -> String {
        format!(
            r#"
fn strategy_config() {{
    strategy_config::new().with_primary(timeframe("1m"))
}}

fn on_tick(market, context) {{
    {expression}
}}
"#
        )
    }

    fn run_completed_tick(source: &str) -> crate::RuntimeStep {
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("completed primary candle should be accepted")
    }

    fn produced_decision(step: &crate::RuntimeStep) -> StrategyDecision {
        step.events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::StrategyDecisionProduced { decision } => Some(decision.clone()),
                _ => None,
            })
            .expect("step should include a produced strategy decision")
    }

    fn strategy_error_message(step: &crate::RuntimeStep) -> String {
        step.events
            .iter()
            .find_map(|event| match event {
                RuntimeEvent::StrategyError { error, .. } => Some(error.message.clone()),
                _ => None,
            })
            .expect("step should include a strategy error")
    }

    #[test]
    fn grouped_context_exposes_flat_portfolio_snapshot_without_flat_context_aliases() {
        let source = source_returning(
            r#"
if context.portfolio.equity == 1000.0
        && context.portfolio.realized_cash_balance == 1000.0
        && context.portfolio.completed_trades == 0
        && context.portfolio.position == () {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn grouped_context_exposes_open_position_details() {
        let source = source_returning(
            r#"
let position = context.portfolio.position;
if position != ()
        && position.side == "long"
        && position.entry_price == 100.0
        && position.size == 2.0
        && position.entry_time == 1
        && position.stop_loss == 90.0
        && position.take_profit == 120.0 {
    decision::close_long()
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut portfolio = PortfolioState::new(1_000.0);
        portfolio
            .open_long_from_flat(&candle(100.0), 2.0, Some(90.0), Some(120.0))
            .expect("position should open");
        let mut runtime = TradingRuntime::new(portfolio, 0, strategy);

        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(105.0)))
            .expect("completed primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::close_long());
    }

    #[test]
    fn strategy_state_get_set_persists_primitives_between_strategy_ticks() {
        let source = source_returning(
            r#"
let seen = context.state.get("seen", 0);
context.state.set("seen", seen + 1);

if seen == 0 {
    decision::hold()
} else {
    decision::open_long(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_typed_int_helpers_return_default_then_stored_value() {
        let source = source_returning(
            r#"
let seen = context.state.int("seen", 0);
context.state.set_int("seen", seen + 1);

if seen == 0 {
    decision::hold()
} else if seen == 1 {
    decision::open_long(1.0)
} else {
    decision::open_short(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_typed_float_helpers_return_default_then_stored_value() {
        let source = source_returning(
            r#"
let threshold = context.state.float("threshold", 1.5);
context.state.set_float("threshold", threshold + 1.0);

if threshold == 1.5 {
    decision::hold()
} else if threshold == 2.5 {
    decision::open_long(1.0)
} else {
    decision::open_short(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_typed_bool_helpers_return_default_then_stored_value() {
        let source = source_returning(
            r#"
let enabled = context.state.bool("enabled", false);
context.state.set_bool("enabled", true);

if enabled {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_typed_string_helpers_return_default_then_stored_value() {
        let source = source_returning(
            r#"
let phase = context.state.string("phase", "new");
context.state.set_string("phase", "active");

if phase == "new" {
    decision::hold()
} else if phase == "active" {
    decision::open_long(1.0)
} else {
    decision::open_short(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_typed_getters_report_the_existing_type_mismatch_contract() {
        let source = source_returning(
            r#"
context.state.set_int("seen", 1);
let enabled = context.state.bool("seen", false);

if enabled {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);
        let message = strategy_error_message(&step);

        assert!(
            message.contains("strategy state key `seen` contains int, not requested bool"),
            "{message}"
        );
        assert!(!step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    }

    #[test]
    fn strategy_state_starts_empty_for_each_runtime_session() {
        let source = source_returning(
            r#"
let seen = context.state.get("seen", 0);
context.state.set("seen", seen + 1);

if seen == 0 {
    decision::hold()
} else {
    decision::open_long(1.0)
}
"#,
        );

        let first_step_in_first_runtime = run_completed_tick(&source);
        let first_step_in_second_runtime = run_completed_tick(&source);

        assert_eq!(
            produced_decision(&first_step_in_first_runtime),
            StrategyDecision::hold()
        );
        assert_eq!(
            produced_decision(&first_step_in_second_runtime),
            StrategyDecision::hold()
        );
    }

    #[test]
    fn warmup_input_does_not_mutate_strategy_state() {
        let source = source_returning(
            r#"
let seen = context.state.get("seen", 0);
context.state.set("seen", seen + 1);

if seen == 0 {
    decision::hold()
} else {
    decision::open_long(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 1, strategy);

        runtime
            .on_market_input(MarketInput::WarmupCandle(candle(99.0)))
            .expect("warmup candle should be accepted");
        let first_strategy_tick = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("completed primary candle should be accepted");

        assert_eq!(
            produced_decision(&first_strategy_tick),
            StrategyDecision::hold()
        );
    }

    #[test]
    fn market_view_exposes_current_primary_candle_fields() {
        let source = source_returning(
            r#"
let c = market.candle();
if c.open == 100.0
        && c.high == 100.0
        && c.low == 100.0
        && c.close == 100.0
        && c.volume == 1000.0
        && c.timestamp == 1
        && c.symbol == "BTC-USD"
        && c.timeframe == timeframe("1m") {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn market_view_primary_history_is_newest_first_one_indexed_and_unit_out_of_range() {
        let source = source_returning(
            r#"
let history = market.candles();
let current = market.candle();

if history[0] == () || history[3] == () {
    if history[1] != ()
            && history[2] != ()
            && history[1].close == current.close
            && history[1].close == 101.0
            && history[2].close == 100.0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
} else {
    decision::open_short(1.0)
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn market_view_history_accepts_sma_indicator_binding() {
        let source = source_returning(
            r#"
let average = indicators::sma(market.candles(), 3);
if average != () && average == 101.0 {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");
        let third = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(102.0)))
            .expect("third completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::hold());
        assert_eq!(produced_decision(&third), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn ta_namespace_indicator_runs_after_explicit_strategy_warmup() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(2)
}

fn on_tick(market, context) {
    let average = ta::sma(market.candles(), 3);
    if average != () && average == 101.0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let runtime_config =
            RuntimeConfig::from_strategy_config("BTC-USD", strategy.strategy_config())
                .expect("strategy config should resolve");
        let warmup_requirement = strategy.strategy_config().minimum_warmup();
        let mut runtime = TradingRuntime::with_config(
            runtime_config,
            PortfolioState::new(1_000.0),
            warmup_requirement,
            strategy,
        );

        runtime
            .on_market_input(MarketInput::WarmupCandle(candle_at(
                100.0,
                60_000,
                Timeframe::minutes(1),
            )))
            .expect("first warmup candle should be accepted");
        runtime
            .on_market_input(MarketInput::WarmupCandle(candle_at(
                101.0,
                120_000,
                Timeframe::minutes(1),
            )))
            .expect("second warmup candle should be accepted");
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                102.0,
                180_000,
                Timeframe::minutes(1),
            )))
            .expect("completed primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn ta_cross_over_returns_expected_booleans_for_crossing_and_non_crossing_inputs() {
        let source = source_returning(
            r#"
let crossed = ta::cross_over(1.0, 1.0, 2.0, 1.5);
let did_not_cross_when_already_above = ta::cross_over(2.0, 1.0, 3.0, 2.0);
let did_not_cross_when_current_equal = ta::cross_over(1.0, 1.0, 1.0, 1.0);
let did_not_cross_when_current_below = ta::cross_over(1.0, 2.0, 1.5, 2.0);

if crossed
        && !did_not_cross_when_already_above
        && !did_not_cross_when_current_equal
        && !did_not_cross_when_current_below {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn ta_cross_under_returns_expected_booleans_for_crossing_and_non_crossing_inputs() {
        let source = source_returning(
            r#"
let crossed = ta::cross_under(2.0, 2.0, 1.0, 1.5);
let did_not_cross_when_already_below = ta::cross_under(1.0, 2.0, 1.0, 2.5);
let did_not_cross_when_current_equal = ta::cross_under(2.0, 2.0, 2.0, 2.0);
let did_not_cross_when_current_above = ta::cross_under(2.0, 1.0, 2.5, 1.0);

if crossed
        && !did_not_cross_when_already_below
        && !did_not_cross_when_current_equal
        && !did_not_cross_when_current_above {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn ta_cross_helpers_accept_indicator_inputs_after_explicit_unit_guards() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_minimum_warmup(2)
}

fn bullish_cross(market) {
    let candles = market.candles();
    let fast = ta::sma(candles, 1);
    let slow = ta::sma(candles, 2);
    let fast_prev = ta::sma(candles, 1, 1);
    let slow_prev = ta::sma(candles, 2, 1);

    if fast == () || slow == () || fast_prev == () || slow_prev == () {
        return false;
    }

    ta::cross_over(fast_prev, slow_prev, fast, slow)
}

fn on_tick(market, context) {
    if bullish_cross(market) {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let runtime_config =
            RuntimeConfig::from_strategy_config("BTC-USD", strategy.strategy_config())
                .expect("strategy config should resolve");
        let warmup_requirement = strategy.strategy_config().minimum_warmup();
        let mut runtime = TradingRuntime::with_config(
            runtime_config,
            PortfolioState::new(1_000.0),
            warmup_requirement,
            strategy,
        );

        runtime
            .on_market_input(MarketInput::WarmupCandle(candle_at(
                100.0,
                60_000,
                Timeframe::minutes(1),
            )))
            .expect("first warmup candle should be accepted");
        runtime
            .on_market_input(MarketInput::WarmupCandle(candle_at(
                100.0,
                120_000,
                Timeframe::minutes(1),
            )))
            .expect("second warmup candle should be accepted");
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                102.0,
                180_000,
                Timeframe::minutes(1),
            )))
            .expect("completed primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn primary_market_view_history_accepts_v1_scalar_indicator_pack() {
        for namespace in ["ta", "indicators"] {
            let body = r#"
let candles = market.candles();
let ema = NS::ema(candles, 5);
let dema = NS::dema(candles, 5);
let tema = NS::tema(candles, 5);
let slope = NS::slope(candles, 5);
let rsi = NS::rsi(candles, 5);
let roc = NS::roc(candles, 5);
let cci = NS::cci(candles, 5);
let williams = NS::williams_r(candles, 5);
let atr = NS::atr(candles, 5);
let mfi = NS::mfi(candles, 5);
let obv = NS::obv(candles);

let all_available = true;
if ema == () { all_available = false; }
if dema == () { all_available = false; }
if tema == () { all_available = false; }
if slope == () { all_available = false; }
if rsi == () { all_available = false; }
if roc == () { all_available = false; }
if cci == () { all_available = false; }
if williams == () { all_available = false; }
if atr == () { all_available = false; }
if mfi == () { all_available = false; }
if obv == () { all_available = false; }

if all_available {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#
            .replace("NS::", &format!("{namespace}::"));
            let source = source_returning(&body);
            let strategy = RhaiStrategy::load(&source).expect("strategy should load");
            let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);
            let mut latest_step = None;

            for index in 1..=30 {
                let close = index as f64;
                latest_step = Some(
                    runtime
                        .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                            close,
                            close + 0.5,
                            close - 0.5,
                            close,
                            index * 60_000,
                            Timeframe::minutes(1),
                        )))
                        .expect("primary completed candle should be accepted"),
                );
            }

            assert_eq!(
                produced_decision(&latest_step.expect("at least one step should run")),
                StrategyDecision::open_long(1.0),
                "{namespace} namespace should expose the scalar indicator pack"
            );
        }
    }

    #[test]
    fn v1_scalar_indicator_pack_supports_offset_overloads() {
        for namespace in ["ta", "indicators"] {
            let body = r#"
let candles = market.candles();
let sma_offset = NS::sma(candles, 5, 1);
let ema = NS::ema(candles, 5, 1);
let dema = NS::dema(candles, 5, 1);
let tema = NS::tema(candles, 5, 1);
let slope = NS::slope(candles, 5, 1);
let rsi = NS::rsi(candles, 5, 1);
let roc = NS::roc(candles, 5, 1);
let cci = NS::cci(candles, 5, 1);
let williams = NS::williams_r(candles, 5, 1);
let atr = NS::atr(candles, 5, 1);
let mfi = NS::mfi(candles, 5, 1);
let obv_offset = NS::obv(candles, 1);

let all_available = true;
if sma_offset == () { all_available = false; }
if ema == () { all_available = false; }
if dema == () { all_available = false; }
if tema == () { all_available = false; }
if slope == () { all_available = false; }
if rsi == () { all_available = false; }
if roc == () { all_available = false; }
if cci == () { all_available = false; }
if williams == () { all_available = false; }
if atr == () { all_available = false; }
if mfi == () { all_available = false; }
if obv_offset == () { all_available = false; }

if all_available && sma_offset == 27.0 && obv_offset == 28000.0 {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#
            .replace("NS::", &format!("{namespace}::"));
            let source = source_returning(&body);
            let strategy = RhaiStrategy::load(&source).expect("strategy should load");
            let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);
            let mut latest_step = None;

            for index in 1..=30 {
                let close = index as f64;
                latest_step = Some(
                    runtime
                        .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                            close,
                            close + 0.5,
                            close - 0.5,
                            close,
                            index * 60_000,
                            Timeframe::minutes(1),
                        )))
                        .expect("primary completed candle should be accepted"),
                );
            }

            assert_eq!(
                produced_decision(&latest_step.expect("at least one step should run")),
                StrategyDecision::open_long(1.0),
                "{namespace} namespace should expose offset overloads"
            );
        }
    }

    #[test]
    fn scalar_indicator_bindings_return_unit_for_insufficient_history_and_invalid_periods() {
        for namespace in ["ta", "indicators"] {
            let body = r#"
let candles = market.candles();
let all_unit = true;

if NS::sma(candles, 2) != () { all_unit = false; }
if NS::sma(candles, 0) != () { all_unit = false; }
if NS::ema(candles, -5) != () { all_unit = false; }
if NS::ema(candles, 5, -1) != () { all_unit = false; }
if NS::dema(candles, -5) != () { all_unit = false; }
if NS::tema(candles, -5) != () { all_unit = false; }
if NS::slope(candles, -5) != () { all_unit = false; }
if NS::rsi(candles, -5) != () { all_unit = false; }
if NS::roc(candles, -5) != () { all_unit = false; }
if NS::cci(candles, -5) != () { all_unit = false; }
if NS::williams_r(candles, -5) != () { all_unit = false; }
if NS::atr(candles, -5) != () { all_unit = false; }
if NS::mfi(candles, -5) != () { all_unit = false; }
if NS::obv(candles) != () { all_unit = false; }
if NS::obv(candles, -1) != () { all_unit = false; }

if all_unit {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#
            .replace("NS::", &format!("{namespace}::"));
            let source = source_returning(&body);
            let step = run_completed_tick(&source);

            assert_eq!(
                produced_decision(&step),
                StrategyDecision::open_long(1.0),
                "{namespace} namespace should preserve unit-return behavior"
            );
        }
    }

    #[test]
    fn secondary_market_view_history_accepts_v1_scalar_indicator_pack() {
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1))
}

fn scalar_pack_available(candles) {
    let ema = indicators::ema(candles, 5);
    let dema = indicators::dema(candles, 5);
    let tema = indicators::tema(candles, 5);
    let slope = indicators::slope(candles, 5);
    let rsi = indicators::rsi(candles, 5);
    let roc = indicators::roc(candles, 5);
    let cci = indicators::cci(candles, 5);
    let williams = indicators::williams_r(candles, 5);
    let atr = indicators::atr(candles, 5);
    let mfi = indicators::mfi(candles, 5);
    let obv = indicators::obv(candles);

    let all_available = true;
    if ema == () { all_available = false; }
    if dema == () { all_available = false; }
    if tema == () { all_available = false; }
    if slope == () { all_available = false; }
    if rsi == () { all_available = false; }
    if roc == () { all_available = false; }
    if cci == () { all_available = false; }
    if williams == () { all_available = false; }
    if atr == () { all_available = false; }
    if mfi == () { all_available = false; }
    if obv == () { all_available = false; }
    all_available
}

fn on_tick(market, context) {
    if scalar_pack_available(market.candles(H1)) {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::required(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        for index in 1..=30 {
            let close = index as f64;
            runtime
                .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                    close,
                    close + 0.5,
                    close - 0.5,
                    close,
                    index as i64 * 3_600_000,
                    Timeframe::hours(1),
                )))
                .expect("secondary completed candle should be accepted");
        }
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                100.0,
                30 * 3_600_000,
                Timeframe::minutes(1),
            )))
            .expect("primary completed candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn warmup_input_populates_market_state_without_calling_rhai_on_tick() {
        let source = source_returning(
            r#"
let previous = market.candles()[2];
if previous != () && previous.close == 99.0 {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 1, strategy);

        let warmup = runtime
            .on_market_input(MarketInput::WarmupCandle(candle(99.0)))
            .expect("warmup candle should be accepted");
        let first_strategy_tick = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("completed primary candle should be accepted");

        assert!(!warmup
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::StrategyTickStarted { .. })));
        assert_eq!(
            produced_decision(&first_strategy_tick),
            StrategyDecision::open_long(1.0)
        );
    }

    #[test]
    fn scalar_indicators_read_warmup_rebuilt_market_state_not_strategy_buffers() {
        let source = source_returning(
            r#"
let seen = context.state.get("seen", 0);
context.state.set("seen", seen + 1);

let candles = market.candles();
let tema = indicators::tema(candles, 5);
let obv = indicators::obv(candles);

if seen == 0 && tema != () && obv == 29000.0 {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 29, strategy);

        for index in 1..=29 {
            let close = index as f64;
            let warmup = runtime
                .on_market_input(MarketInput::WarmupCandle(candle_ohlc_at(
                    close,
                    close + 0.5,
                    close - 0.5,
                    close,
                    index * 60_000,
                    Timeframe::minutes(1),
                )))
                .expect("warmup candle should be accepted");
            assert!(!warmup
                .events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::StrategyTickStarted { .. })));
        }

        let first_strategy_tick = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                30.0,
                30.5,
                29.5,
                30.0,
                30 * 60_000,
                Timeframe::minutes(1),
            )))
            .expect("completed primary candle should be accepted");

        assert_eq!(
            produced_decision(&first_strategy_tick),
            StrategyDecision::open_long(1.0)
        );
    }

    #[test]
    fn market_view_reads_available_secondary_candle_and_history_by_typed_timeframe() {
        let source = r#"
const M1 = timeframe("1m");
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(M1)
        .with_secondary(secondary::required(H1))
}

fn on_tick(market, context) {
    let h1 = market.candle(H1);
    let h1_history = market.candles(H1);

    if h1 != ()
            && h1_history[1] != ()
            && h1_history[2] != ()
            && h1.close == 200.0
            && h1_history[1].close == 200.0
            && h1_history[2].close == 150.0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::required(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                150.0,
                0,
                Timeframe::hours(1),
            )))
            .expect("first secondary candle should be accepted");
        runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                200.0,
                3_600_000,
                Timeframe::hours(1),
            )))
            .expect("second secondary candle should be accepted");
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                100.0,
                3_600_000,
                Timeframe::minutes(1),
            )))
            .expect("primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn optional_secondary_missing_returns_unit_for_candle_and_candles() {
        let source = source_returning(
            r#"
let H1 = timeframe("1h");
if market.candle(H1) == () && market.candles(H1) == () {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::optional(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                100.0,
                60_000,
                Timeframe::minutes(1),
            )))
            .expect("primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn optional_secondary_stale_returns_unit_for_candle_and_candles() {
        let source = source_returning(
            r#"
let H1 = timeframe("1h");
if market.candle(H1) == () && market.candles(H1) == () {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::optional(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                150.0,
                0,
                Timeframe::hours(1),
            )))
            .expect("secondary candle should be accepted");
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                100.0,
                3_600_001,
                Timeframe::minutes(1),
            )))
            .expect("primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn required_secondary_missing_blocks_before_rhai_on_tick() {
        let source = source_returning("decision::open_long(1.0)");
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::required(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        let primary = candle_at(100.0, 60_000, Timeframe::minutes(1));
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(primary.clone()))
            .expect("primary candle should be accepted");

        assert!(step.events.contains(&RuntimeEvent::StrategyTickBlocked {
            candle: primary,
            blocked_contexts: vec![crate::BlockedSecondaryContext {
                timeframe: Timeframe::hours(1),
                reason: crate::SecondaryContextUnavailableReason::Missing,
            }],
        }));
        assert!(!step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyTickStarted { .. }
                | RuntimeEvent::StrategyDecisionProduced { .. }
                | RuntimeEvent::PositionOpened { .. }
        )));
    }

    #[test]
    fn unconfigured_typed_timeframe_access_is_strategy_error_without_portfolio_transition() {
        for access in ["market.candle(H4)", "market.candles(H4)"] {
            let source = source_returning(&format!(
                r#"
let H4 = timeframe("4h");
let ignored = {access};
decision::open_long(1.0)
"#
            ));
            let step = run_completed_tick(&source);
            let message = strategy_error_message(&step);

            assert!(message.contains("unconfigured timeframe `4h`"), "{message}");
            assert!(!step.events.iter().any(|event| matches!(
                event,
                RuntimeEvent::StrategyDecisionProduced { .. }
                    | RuntimeEvent::ExecutionActionPlanned { .. }
                    | RuntimeEvent::PositionOpened { .. }
            )));
        }
    }

    #[test]
    fn indicators_work_over_secondary_history() {
        let source = source_returning(
            r#"
let H1 = timeframe("1h");
let average = indicators::sma(market.candles(H1), 2);
if average != () && average == 175.0 {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::with_config(
            RuntimeConfig::with_secondary_configs(
                "BTC-USD",
                Timeframe::minutes(1),
                [SecondaryTimeframeConfig::required(Timeframe::hours(1), 0)],
            ),
            PortfolioState::new(1_000.0),
            0,
            strategy,
        );

        runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                150.0,
                0,
                Timeframe::hours(1),
            )))
            .expect("first secondary candle should be accepted");
        runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                200.0,
                3_600_000,
                Timeframe::hours(1),
            )))
            .expect("second secondary candle should be accepted");
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_at(
                100.0,
                3_600_000,
                Timeframe::minutes(1),
            )))
            .expect("primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn strategy_state_supports_float_bool_and_string_values() {
        let source = source_returning(
            r#"
let price = context.state.get("price", 1.5);
let enabled = context.state.get("enabled", false);
let label = context.state.get("label", "cold");

context.state.set("price", price + 1.0);
context.state.set("enabled", true);
context.state.set("label", "warm");

if price == 2.5 && enabled && label == "warm" {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );
        let strategy = RhaiStrategy::load(&source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);

        let first = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(100.0)))
            .expect("first completed primary candle should be accepted");
        let second = runtime
            .on_market_input(MarketInput::CompletedCandle(candle(101.0)))
            .expect("second completed primary candle should be accepted");

        assert_eq!(produced_decision(&first), StrategyDecision::hold());
        assert_eq!(produced_decision(&second), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn unsupported_strategy_state_values_fail_at_tick_time() {
        for expression in [
            r#"context.state.set("items", [1, 2, 3]);"#,
            r#"context.state.set("shape", #{ seen: 1 });"#,
            r#"context.state.set_int("items", [1, 2, 3]);"#,
            r#"context.state.set_string("shape", #{ seen: 1 });"#,
        ] {
            let source = source_returning(&format!(
                r#"
{expression}
decision::hold()
"#
            ));
            let step = run_completed_tick(&source);
            let message = strategy_error_message(&step);

            assert!(message.contains("set"), "{message}");
            assert!(!step
                .events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
        }
    }

    #[test]
    fn context_does_not_expose_market_data_or_old_flat_portfolio_aliases() {
        for expression in ["context.balance", "context.candle", "context.close"] {
            let source = source_returning(&format!(
                r#"
let forbidden = {expression};
decision::hold()
"#
            ));
            let step = run_completed_tick(&source);
            let message = strategy_error_message(&step);

            assert!(
                message.contains("Property") || message.contains("property"),
                "{expression}: {message}"
            );
            assert!(!step
                .events
                .iter()
                .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
        }
    }

    #[test]
    fn loads_minimal_strategy_with_required_strategy_config_primary() {
        let strategy = RhaiStrategy::load(MINIMAL).expect("strategy should load");

        assert_eq!(
            strategy.hooks(),
            RhaiStrategyHooks {
                has_on_tick: true,
                has_strategy_config: true,
                has_anchored_config: false,
                has_structure_config: false,
            }
        );
        assert_eq!(
            strategy.strategy_config().primary_timeframe(),
            Some(Timeframe::minutes(1))
        );
        assert_eq!(strategy.anchored_config(), None);
    }

    #[test]
    fn loads_strategy_with_top_level_constants_and_typed_strategy_config() {
        let source = r#"
const M30 = timeframe("30m");

fn strategy_config() {
    strategy_config::new()
        .with_primary(M30)
        .with_minimum_warmup(200)
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");

        assert!(strategy.hooks().has_strategy_config);
        assert_eq!(
            strategy.strategy_config().primary_timeframe(),
            Some(Timeframe::minutes(30))
        );
        assert_eq!(strategy.strategy_config().minimum_warmup(), 200);
    }

    #[test]
    fn typed_strategy_config_extracts_primary_and_secondaries() {
        let source = r#"
const M1 = timeframe("1m");
const H1 = timeframe("1h");
const D1 = timeframe("1d");

fn strategy_config() {
    strategy_config::new()
        .with_primary(M1)
        .with_minimum_warmup(200)
        .with_secondary(
            secondary::required(H1)
                .with_max_missing_candles(1)
        )
        .with_secondary(
            secondary::optional(D1)
                .with_max_missing_candles(0)
        )
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");

        assert_eq!(
            strategy.strategy_config().primary_timeframe(),
            Some(Timeframe::minutes(1))
        );
        assert_eq!(strategy.strategy_config().minimum_warmup(), 200);
        assert_eq!(
            strategy.strategy_config().secondary_timeframes(),
            &[
                SecondaryTimeframeConfig::required(Timeframe::hours(1), 1),
                SecondaryTimeframeConfig::optional(Timeframe::days(1), 0),
            ]
        );
    }

    #[test]
    fn runtime_config_is_derived_from_runtime_asset_and_strategy_timeframe_contract() {
        let source = r#"
const M30 = timeframe("30m");
const D1 = timeframe("1d");

fn strategy_config() {
    strategy_config::new()
        .with_primary(M30)
        .with_secondary(secondary::required(D1).with_max_missing_candles(1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");

        let resolved = RuntimeConfig::from_strategy_config("BTC-USD", strategy.strategy_config())
            .expect("validated strategy config should resolve");

        assert_eq!(resolved.runtime_asset, "BTC-USD");
        assert_eq!(resolved.primary_timeframe, Timeframe::minutes(30));
        assert_eq!(
            resolved.secondary_timeframes,
            vec![SecondaryTimeframeConfig::required(Timeframe::days(1), 1)]
        );
    }

    #[test]
    fn strategy_config_rejects_legacy_run_config_methods() {
        for forbidden in [
            r#"strategy_config::new().with_primary(timeframe("1m")).with_runtime_asset("ETH-USD")"#,
            r#"strategy_config::new().with_primary(timeframe("1m")).with_primary_timeframe(timeframe("1h"))"#,
        ] {
            let source = format!(
                r#"
fn strategy_config() {{
    {forbidden}
}}

fn on_tick(market, context) {{
    decision::hold()
}}
"#
            );

            let error = RhaiStrategy::load(&source).unwrap_err();

            assert!(
                matches!(error, RhaiStrategyLoadError::HookEvaluation { .. }),
                "{forbidden}: {error}"
            );
        }
    }

    #[test]
    fn invalid_timeframe_in_strategy_config_fails_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(timeframe("15min")))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::HookEvaluation { .. }
        ));
        assert!(error.to_string().contains("invalid timeframe"));
    }

    #[test]
    fn invalid_secondary_missing_candle_tolerance_fails_load() {
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(H1).with_max_missing_candles(-1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::HookEvaluation { .. }
        ));
        assert!(error.to_string().contains("max missing candles"));
    }

    #[test]
    fn strategy_config_missing_primary_fails_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_minimum_warmup(10)
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidStrategyConfiguration { .. }
        ));
        assert!(error.to_string().contains("with_primary"));
    }

    #[test]
    fn strategy_config_duplicate_primary_fails_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_primary(timeframe("5m"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidStrategyConfiguration { .. }
        ));
        assert!(error.to_string().contains("exactly one Primary"));
    }

    #[test]
    fn strategy_config_secondary_equal_to_primary_fails_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(timeframe("1m")))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidStrategyConfiguration { .. }
        ));
        assert!(error.to_string().contains("must not equal the Primary"));
    }

    #[test]
    fn strategy_config_duplicate_secondaries_fail_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new()
        .with_primary(timeframe("1m"))
        .with_secondary(secondary::required(timeframe("1h")))
        .with_secondary(secondary::optional(timeframe("1h")))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidStrategyConfiguration { .. }
        ));
        assert!(error.to_string().contains("duplicate Secondary"));
    }

    #[test]
    fn typed_decision_constructors_return_runtime_decisions() {
        let cases = [
            ("decision::hold()", StrategyDecision::hold()),
            ("decision::open_long(2.0)", StrategyDecision::open_long(2.0)),
            ("decision::close_long()", StrategyDecision::close_long()),
            (
                "decision::open_short(3.0)",
                StrategyDecision::open_short(3.0),
            ),
            ("decision::close_short()", StrategyDecision::close_short()),
        ];

        for (expression, expected_decision) in cases {
            let source = source_returning(expression);
            let step = run_completed_tick(&source);

            assert_eq!(produced_decision(&step), expected_decision, "{expression}");
        }
    }

    #[test]
    fn fluent_open_long_risk_and_reason_map_to_runtime_decision_fields() {
        let source = source_returning(
            r#"decision::open_long(2.0)
                .with_stop_loss(95.0)
                .with_take_profit(120.0)
                .with_reason("breakout")"#,
        );

        let step = run_completed_tick(&source);
        let decision = produced_decision(&step);

        assert_eq!(decision.intent, StrategyDecisionIntent::OpenLong);
        assert_eq!(decision.quantity, Some(2.0));
        assert_eq!(decision.stop_loss, Some(95.0));
        assert_eq!(decision.take_profit, Some(120.0));
        assert_eq!(decision.reason.as_deref(), Some("breakout"));
        assert!(step.events.iter().any(|event| match event {
            RuntimeEvent::ExecutionActionPlanned { action } => {
                action
                    == &(ExecutionAction::OpenLong {
                        quantity: 2.0,
                        stop_loss: Some(95.0),
                        take_profit: Some(120.0),
                    })
            }
            _ => false,
        }));
    }

    #[test]
    fn fluent_open_short_risk_maps_to_runtime_decision_fields() {
        let source = source_returning(
            r#"decision::open_short(4.0)
                .with_stop_loss(105.0)
                .with_take_profit(80.0)"#,
        );

        let step = run_completed_tick(&source);
        let decision = produced_decision(&step);

        assert_eq!(decision.intent, StrategyDecisionIntent::OpenShort);
        assert_eq!(decision.quantity, Some(4.0));
        assert_eq!(decision.stop_loss, Some(105.0));
        assert_eq!(decision.take_profit, Some(80.0));
        assert!(step.events.iter().any(|event| match event {
            RuntimeEvent::ExecutionActionPlanned { action } => {
                action
                    == &(ExecutionAction::OpenShort {
                        quantity: 4.0,
                        stop_loss: Some(105.0),
                        take_profit: Some(80.0),
                    })
            }
            _ => false,
        }));
    }

    #[test]
    fn reason_is_diagnostic_only_and_allowed_on_non_opening_decisions() {
        let cases = [
            (
                "decision::hold().with_reason(\"waiting\")",
                StrategyDecisionIntent::Hold,
            ),
            (
                "decision::close_long().with_reason(\"target reached\")",
                StrategyDecisionIntent::CloseLong,
            ),
            (
                "decision::close_short().with_reason(\"covered\")",
                StrategyDecisionIntent::CloseShort,
            ),
        ];

        for (expression, expected_intent) in cases {
            let source = source_returning(expression);
            let step = run_completed_tick(&source);
            let decision = produced_decision(&step);

            assert_eq!(decision.intent, expected_intent, "{expression}");
            assert!(decision.reason.is_some(), "{expression}");
        }
    }

    #[test]
    fn risk_methods_on_non_opening_decisions_are_strategy_errors_without_execution_planning() {
        for expression in [
            "decision::hold().with_stop_loss(95.0)",
            "decision::close_long().with_take_profit(120.0)",
        ] {
            let source = source_returning(expression);
            let step = run_completed_tick(&source);

            assert!(step.events.iter().any(|event| matches!(
                event,
                RuntimeEvent::StrategyError { error, .. }
                    if error.message.contains("only valid on opening decisions")
            )));
            assert!(!step.events.iter().any(|event| matches!(
                event,
                RuntimeEvent::StrategyDecisionProduced { .. }
                    | RuntimeEvent::ExecutionActionPlanned { .. }
            )));
        }
    }

    #[test]
    fn wrong_on_tick_return_types_are_strategy_errors_without_legacy_mapping_or_planning() {
        for expression in [
            r#"#{ signal: "BUY", size: 0.5 }"#,
            r#"#{ intent: "OPEN_LONG", quantity: 2.0 }"#,
            r#""HOLD""#,
            "()",
            "42.0",
        ] {
            let source = source_returning(expression);
            let step = run_completed_tick(&source);

            assert!(
                step.events.iter().any(|event| matches!(
                    event,
                    RuntimeEvent::StrategyError { error, .. }
                        if error.message.contains("must return typed StrategyDecision")
                )),
                "{expression}"
            );
            assert!(
                !step.events.iter().any(|event| matches!(
                    event,
                    RuntimeEvent::StrategyDecisionProduced { .. }
                        | RuntimeEvent::ExecutionActionPlanned { .. }
                )),
                "{expression}"
            );
        }
    }

    #[test]
    fn invalid_opening_quantity_uses_existing_runtime_ignored_decision_semantics() {
        let source = source_returning("decision::open_long(0.0)");

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(0.0));
        assert!(step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::ExecutionActionPlanned {
                action: ExecutionAction::Noop,
            }
        )));
        assert!(step.events.iter().any(|event| matches!(
            event,
            RuntimeEvent::StrategyDecisionIgnored {
                reason: IgnoredDecisionReason::InvalidQuantity,
                ..
            }
        )));
    }

    #[test]
    fn load_fails_when_required_on_tick_is_missing() {
        let error = RhaiStrategy::load("let x = 1;").unwrap_err();

        assert_eq!(
            error,
            RhaiStrategyLoadError::MissingRequiredHook {
                expected: "fn on_tick(market, context)",
            }
        );
        assert!(error.to_string().contains("fn on_tick(market, context)"));
    }

    #[test]
    fn load_fails_when_on_tick_has_wrong_arity() {
        let error = RhaiStrategy::load("fn on_tick() {}").unwrap_err();

        assert_eq!(
            error,
            RhaiStrategyLoadError::InvalidHookSignature {
                hook: ON_TICK_HOOK,
                expected: "fn on_tick(market, context)",
            }
        );
    }

    #[test]
    fn load_fails_for_syntax_errors() {
        let error = RhaiStrategy::load("fn on_tick(market, context) {").unwrap_err();

        assert!(matches!(error, RhaiStrategyLoadError::Compile { .. }));
        assert!(error.to_string().contains("strategy compile error"));
    }

    #[test]
    fn load_fails_when_strategy_config_is_missing() {
        let error = RhaiStrategy::load(
            r#"
fn on_tick(market, context) {
    decision::hold()
}
"#,
        )
        .unwrap_err();

        assert_eq!(
            error,
            RhaiStrategyLoadError::MissingRequiredHook {
                expected: "fn strategy_config()",
            }
        );
        assert!(error.to_string().contains("fn strategy_config()"));
    }

    #[test]
    fn missing_optional_anchored_config_uses_default() {
        let strategy = RhaiStrategy::load(MINIMAL).expect("strategy should load");

        assert!(strategy.anchored_config().is_none());
    }

    #[test]
    fn present_anchored_config_is_called_and_validated() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(
            pivot_detector::new("swing")
                .with_left_bars(1)
                .with_right_bars(1)
        )
        .with_evaluator(
            anchored::trendline("trend", "swing")
                .with_side(pivot_side::high())
        )
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let config = strategy
            .anchored_config()
            .expect("anchored config should load");

        assert!(strategy.hooks().has_anchored_config);
        assert_eq!(config.detectors().len(), 1);
        assert_eq!(config.evaluators().len(), 1);
    }

    #[test]
    fn present_structure_config_is_called_and_preserves_fluent_object_settings() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 2);
    s.objects.trendline("support", swing)
        .with_side(structure_side::low())
        .with_pivot_buffer(5)
        .with_tolerance(0.02)
        .with_min_touches(4)
        .with_max_active(2);
    s
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let config = strategy
            .structure_config()
            .expect("structure config should load");
        let lowered = config.to_anchored_configuration();

        assert!(strategy.hooks().has_structure_config);
        assert_eq!(lowered.detectors().len(), 1);
        assert_eq!(lowered.evaluators().len(), 1);
        assert_eq!(lowered.detectors()[0].id(), "swing");
        match &lowered.evaluators()[0] {
            AnchoredEvaluatorSpec::Trendline {
                expose_as,
                pivot_source,
                side,
                pivot_buffer,
                tolerance,
                min_touches,
                max_lines,
            } => {
                assert_eq!(expose_as, "support");
                assert_eq!(pivot_source, "swing");
                assert_eq!(
                    *side,
                    indicators::anchored::evaluators::TrendlineSide::Support
                );
                assert_eq!(*pivot_buffer, 5);
                assert_eq!(*tolerance, 0.02);
                assert_eq!(*min_touches, 4);
                assert_eq!(*max_lines, 2);
            }
        }
    }

    #[test]
    fn invalid_structure_config_fails_load() {
        let cases = [
            (
                r#"
fn structure_config() {
    let s = structure_config::new();
    s.points.pivots("swing", 1, 1);
    s.points.pivots("swing", 2, 2);
    s
}
"#,
                "duplicate point id",
            ),
            (
                r#"
fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing);
    s.objects.trendline("trend", swing);
    s
}
"#,
                "duplicate object id",
            ),
            (
                r#"
fn structure_config() {
    let source = structure_config::new().points.pivots("other", 1, 1);
    let s = structure_config::new();
    s.objects.trendline("trend", source);
    s
}
"#,
                "unknown point source",
            ),
            (
                r#"
fn structure_config() {
    let s = structure_config::new();
    s.points.pivots("swing", 0, 1);
    s
}
"#,
                "left_bars",
            ),
            (
                r#"
fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing).with_min_touches(2);
    s
}
"#,
                "min_touches",
            ),
            (
                r#"
fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing).with_max_active(0);
    s
}
"#,
                "max_active",
            ),
        ];

        for (structure_hook, expected_message) in cases {
            let source = format!(
                r#"
fn strategy_config() {{
    strategy_config::new().with_primary(timeframe("1m"))
}}

{structure_hook}

fn on_tick(market, context) {{
    decision::hold()
}}
"#
            );

            let error = RhaiStrategy::load(&source).unwrap_err();

            assert!(matches!(
                error,
                RhaiStrategyLoadError::HookEvaluation {
                    hook: STRUCTURE_CONFIG_HOOK,
                    ..
                }
            ));
            assert!(
                error.to_string().contains(expected_message),
                "{expected_message}: {error}"
            );
        }
    }

    #[test]
    fn multiple_structure_registries_are_not_magically_merged() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn structure_config() {
    let ignored = structure_config::new();
    ignored.points.pivots("ignored", 1, 1);

    structure_config::new()
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let lowered = strategy
            .structure_config()
            .expect("structure config should be present")
            .to_anchored_configuration();

        assert!(lowered.detectors().is_empty());
        assert!(lowered.evaluators().is_empty());
    }

    #[test]
    fn anchored_and_structure_config_are_mutually_exclusive() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new().with_detector(pivot_detector::new("swing"))
}

fn structure_config() {
    let s = structure_config::new();
    s.points.pivots("swing", 1, 1);
    s
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert_eq!(
            error,
            RhaiStrategyLoadError::ConflictingHooks {
                first: ANCHORED_CONFIG_HOOK,
                second: STRUCTURE_CONFIG_HOOK,
            }
        );
        assert!(error.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn missing_anchored_config_loads_and_market_outputs_are_unit() {
        let source = source_returning(
            r#"
let lines = market.anchored("trend");
let pivot = market.last_pivot("swing", pivot_side::high());
if lines == () && pivot == () {
    decision::open_long(1.0)
} else {
    decision::hold()
}
"#,
        );

        let step = run_completed_tick(&source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn invalid_typed_anchored_config_fails_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(pivot_detector::new("swing"))
        .with_detector(pivot_detector::new("swing"))
        .with_evaluator(anchored::trendline("trend", "swing"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::HookEvaluation {
                hook: ANCHORED_CONFIG_HOOK,
                ..
            }
        ));
        assert!(error.to_string().contains("duplicate detector id"));
    }

    #[test]
    fn invalid_pivot_detector_bars_fail_load() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(pivot_detector::new("swing").with_left_bars(0))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::HookEvaluation {
                hook: ANCHORED_CONFIG_HOOK,
                ..
            }
        ));
        assert!(error.to_string().contains("left_bars"));
    }

    #[test]
    fn anchored_pivot_outputs_update_from_warmup_market_state_and_are_visible_on_market_view() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(
            pivot_detector::new("swing")
                .with_left_bars(1)
                .with_right_bars(1)
        )
}

fn on_tick(market, context) {
    let pivot = market.last_pivot("swing", pivot_side::high());
    if pivot != () && pivot.bar == 1 && pivot.price == 3.0 && pivot.side == "high" {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 3, strategy);

        for (index, high) in [1.0, 3.0, 1.0].into_iter().enumerate() {
            runtime
                .on_market_input(MarketInput::WarmupCandle(candle_ohlc_at(
                    high,
                    high,
                    high,
                    high,
                    index as i64,
                    Timeframe::minutes(1),
                )))
                .expect("warmup candle should be accepted");
        }
        let step = runtime
            .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                1.0,
                1.0,
                1.0,
                1.0,
                3,
                Timeframe::minutes(1),
            )))
            .expect("completed primary candle should be accepted");

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn market_anchored_returns_trendline_output_from_runtime_market_state() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(
            pivot_detector::new("swing")
                .with_left_bars(1)
                .with_right_bars(1)
        )
        .with_evaluator(
            anchored::trendline("trend", "swing")
                .with_side(pivot_side::high())
                .with_pivot_buffer(3)
                .with_min_touches(3)
                .with_max_lines(1)
        )
}

fn on_tick(market, context) {
    let lines = market.anchored("trend");
    if lines != () && lines.len() == 1 && lines[0].touches == 3 && lines[0].side == "resistance" {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);
        let highs = [90.0, 100.0, 90.0, 100.0, 90.0, 100.0, 90.0];
        let mut last_step = None;

        for (index, high) in highs.into_iter().enumerate() {
            last_step = Some(
                runtime
                    .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                        90.0,
                        high,
                        80.0,
                        90.0,
                        index as i64,
                        Timeframe::minutes(1),
                    )))
                    .expect("completed primary candle should be accepted"),
            );
        }

        let final_step = last_step.expect("test should feed candles");
        assert_eq!(
            produced_decision(&final_step),
            StrategyDecision::open_long(1.0)
        );
    }

    #[test]
    fn market_structure_active_returns_empty_array_for_declared_inactive_object() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing)
        .with_side(structure_side::high())
        .with_min_touches(3);
    s
}

fn on_tick(market, context) {
    let lines = market.structure.active("trend");
    if lines.len() == 0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;

        let step = run_completed_tick(source);

        assert_eq!(produced_decision(&step), StrategyDecision::open_long(1.0));
    }

    #[test]
    fn market_structure_active_unknown_object_is_strategy_error() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing);
    s
}

fn on_tick(market, context) {
    let lines = market.structure.active("missing");
    decision::hold()
}
"#;

        let step = run_completed_tick(source);
        let message = strategy_error_message(&step);

        assert!(
            message.contains("unknown Structure Object id `missing`"),
            "{message}"
        );
        assert!(!step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    }

    #[test]
    fn market_structure_active_returns_trendline_output_from_runtime_market_state() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn structure_config() {
    let s = structure_config::new();
    let swing = s.points.pivots("swing", 1, 1);
    s.objects.trendline("trend", swing)
        .with_side(structure_side::high())
        .with_pivot_buffer(3)
        .with_min_touches(3)
        .with_max_active(1);
    s
}

fn on_tick(market, context) {
    let lines = market.structure.active("trend");
    if lines.len() != 1 {
        return decision::hold();
    }

    let line = lines[0];
    let fields_visible = line.touches == 3
        && line.side == "resistance"
        && line.slope == line.slope
        && line.intercept == line.intercept
        && line.anchor_start_bar >= 0
        && line.anchor_end_bar >= 0;

    if fields_visible && line.y_at(market.candles().len() - 1) > 0.0 {
        decision::open_long(1.0)
    } else {
        decision::hold()
    }
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let mut runtime = TradingRuntime::new(PortfolioState::new(1_000.0), 0, strategy);
        let highs = [90.0, 100.0, 90.0, 100.0, 90.0, 100.0, 90.0];
        let mut last_step = None;

        for (index, high) in highs.into_iter().enumerate() {
            last_step = Some(
                runtime
                    .on_market_input(MarketInput::CompletedCandle(candle_ohlc_at(
                        90.0,
                        high,
                        80.0,
                        90.0,
                        index as i64,
                        Timeframe::minutes(1),
                    )))
                    .expect("completed primary candle should be accepted"),
            );
        }

        let final_step = last_step.expect("test should feed candles");
        assert_eq!(
            produced_decision(&final_step),
            StrategyDecision::open_long(1.0)
        );
    }

    #[test]
    fn context_does_not_expose_anchored_compatibility_alias() {
        let source = source_returning(
            r#"
let forbidden = context.anchored("trend");
decision::hold()
"#,
        );

        let step = run_completed_tick(&source);
        let message = strategy_error_message(&step);

        assert!(
            message.contains("Function not found") || message.contains("function"),
            "{message}"
        );
        assert!(!step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::StrategyDecisionProduced { .. })));
    }

    #[test]
    fn strategy_config_returning_unit_fails_load() {
        let source = r#"
fn strategy_config() {
    ()
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert_eq!(
            error,
            RhaiStrategyLoadError::InvalidHookReturn {
                hook: STRATEGY_CONFIG_HOOK,
                expected: "a typed StrategyConfig from `strategy_config::new()`",
            }
        );
    }

    #[test]
    fn strategy_config_returning_map_fails_load_without_legacy_mapping() {
        let source = r#"
fn strategy_config() {
    #{ minimum_warmup: 200 }
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidHookReturn {
                hook: STRATEGY_CONFIG_HOOK,
                ..
            }
        ));
    }

    #[test]
    fn optional_anchored_config_returning_map_fails_load_without_legacy_mapping() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    #{}
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(
            error,
            RhaiStrategyLoadError::InvalidHookReturn {
                hook: ANCHORED_CONFIG_HOOK,
                ..
            }
        ));
    }

    #[test]
    fn reserved_constructor_normalization_preserves_strings_and_comments() {
        let source = r#"
// strategy_config::new(
const LABEL = "strategy_config::new(";
const PIVOT_LABEL = "pivot_detector::new(";
const STRUCTURE_LABEL = "structure_config::new(";
/* anchored_config::new( */
fn strategy_config() { strategy_config::new() }
fn anchored_config() { anchored_config::new() }
fn structure_config() { structure_config::new() }
fn detector() { pivot_detector::new("swing") }
"#;

        let normalized = normalize_reserved_constructor_names(source);

        assert!(normalized.contains("// strategy_config::new("));
        assert!(normalized.contains("\"strategy_config::new(\""));
        assert!(normalized.contains("\"pivot_detector::new(\""));
        assert!(normalized.contains("\"structure_config::new(\""));
        assert!(normalized.contains("/* anchored_config::new( */"));
        assert!(normalized.contains("fn strategy_config() { __runtime_strategy_config_new() }"));
        assert!(normalized.contains("fn anchored_config() { __runtime_anchored_config_new() }"));
        assert!(normalized.contains("fn structure_config() { __runtime_structure_config_new() }"));
        assert!(normalized.contains("fn detector() { __runtime_pivot_detector_new(\"swing\") }"));
    }

    #[test]
    fn locked_rhai_still_reserves_public_module_new_without_normalization() {
        let engine = new_rhai_engine();
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = engine
            .compile(source)
            .expect_err("locked Rhai should still require constructor normalization");
        let message = error.to_string().to_lowercase();

        assert!(message.contains("reserved"), "{message}");
        assert!(message.contains("new"), "{message}");
    }

    #[test]
    fn approved_constructor_syntax_loads_through_normalization() {
        let source = r#"
fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn anchored_config() {
    anchored_config::new()
        .with_detector(pivot_detector::new("swing"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source)
            .expect("normalization should preserve approved public constructor syntax");

        assert_eq!(
            strategy.strategy_config().primary_timeframe(),
            Some(Timeframe::minutes(1))
        );
        assert_eq!(strategy.anchored_config().unwrap().detectors().len(), 1);
    }

    #[test]
    fn invalid_top_level_timeframe_fails_initialization() {
        let source = r#"
const BAD = timeframe("15min");

fn strategy_config() {
    strategy_config::new().with_primary(timeframe("1m"))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(error, RhaiStrategyLoadError::Init { .. }));
        assert!(error.to_string().contains("invalid timeframe"));
    }
}
