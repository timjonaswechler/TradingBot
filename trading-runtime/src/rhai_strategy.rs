//! Rhai strategy loading and hook validation for the trading runtime.
//!
//! This module owns compile/load-time Rhai strategy handling inside the
//! `trading-runtime` crate, the typed decision API, grouped Strategy Context /
//! Strategy State, and the Primary-Timeframe Market View used at the strategy
//! tick boundary. Secondary Market View access is implemented in a later slice.

use crate::{
    RuntimePortfolioSnapshot, SecondaryTimeframeConfig, StrategyConfiguration, StrategyDecision,
    StrategyError, StrategyHandler, StrategyState, StrategyStateValue, StrategyTickInput,
    StrategyTickResult,
};
use indicators::trend::sma::sma;
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::{Candle, Position, Timeframe};
use std::{error::Error, fmt, path::Path, sync::Arc};

/// Placeholder typed anchored configuration returned by `anchored_config()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AnchoredConfiguration;

impl AnchoredConfiguration {
    pub fn new() -> Self {
        Self
    }
}

/// Strategy hooks detected during Rhai strategy loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RhaiStrategyHooks {
    pub has_on_tick: bool,
    pub has_strategy_config: bool,
    pub has_anchored_config: bool,
}

/// Runtime-owned compiled Rhai strategy and load-time metadata.
pub struct RhaiStrategy {
    engine: RhaiEngine,
    ast: AST,
    scope: Scope<'static>,
    hooks: RhaiStrategyHooks,
    strategy_config: StrategyConfiguration,
    anchored_config: Option<AnchoredConfiguration>,
}

#[derive(Debug, Clone)]
struct RhaiMarketView {
    current: RhaiCandle,
    primary_history: RhaiCandleHistory,
}

impl RhaiMarketView {
    fn from_runtime(market: &crate::MarketView<'_>) -> Self {
        Self {
            current: RhaiCandle::new(market.primary_candle().clone()),
            primary_history: RhaiCandleHistory::new(market.primary_history().to_vec()),
        }
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
    fn on_tick(&mut self, input: StrategyTickInput<'_>) -> StrategyTickResult {
        let market = RhaiMarketView::from_runtime(&input.market);
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
        validate_optional_zero_arg_hook(&ast, STRATEGY_CONFIG_HOOK)?;
        validate_optional_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK)?;

        let mut scope = Scope::new();
        engine
            .run_ast_with_scope(&mut scope, &ast)
            .map_err(|error| RhaiStrategyLoadError::Init {
                message: error.to_string(),
            })?;

        let strategy_config = if has_zero_arg_hook(&ast, STRATEGY_CONFIG_HOOK) {
            call_typed_strategy_config(&engine, &mut scope, &ast)?
        } else {
            StrategyConfiguration::default()
        };

        let anchored_config = if has_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK) {
            Some(call_typed_anchored_config(&engine, &mut scope, &ast)?)
        } else {
            None
        };

        let hooks = RhaiStrategyHooks {
            has_on_tick: true,
            has_strategy_config: has_zero_arg_hook(&ast, STRATEGY_CONFIG_HOOK),
            has_anchored_config: has_zero_arg_hook(&ast, ANCHORED_CONFIG_HOOK),
        };

        Ok(Self {
            engine,
            ast,
            scope,
            hooks,
            strategy_config,
            anchored_config,
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
            .finish_non_exhaustive()
    }
}

fn new_rhai_engine() -> RhaiEngine {
    let mut engine = RhaiEngine::new();

    engine.register_type_with_name::<Timeframe>("Timeframe");
    engine.register_type_with_name::<StrategyConfiguration>("StrategyConfig");
    engine.register_type_with_name::<SecondaryTimeframeConfig>("SecondaryConfig");
    engine.register_type_with_name::<AnchoredConfiguration>("AnchoredConfig");
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

    engine.register_fn("timeframe", parse_timeframe);
    engine.register_fn("==", |left: Timeframe, right: Timeframe| left == right);
    engine.register_fn(
        "with_minimum_warmup",
        |config: StrategyConfiguration, minimum_warmup: INT| {
            with_minimum_warmup(config, minimum_warmup)
        },
    );
    engine.register_fn("with_secondary", with_secondary);
    engine.register_fn("with_max_missing_candles", with_max_missing_candles);

    engine.register_fn("__runtime_strategy_config_new", StrategyConfiguration::new);
    engine.register_fn("__runtime_anchored_config_new", AnchoredConfiguration::new);

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
    // Rhai 1.24 reserves `new` even in module paths such as
    // `strategy_config::new()`. Keep the strategy-facing API from ADR 0005 and
    // lower only these approved typed constructors to private runtime function
    // names before compilation. This is intentionally lexical enough to avoid
    // rewriting string literals or comments.
    const REPLACEMENTS: [(&str, &str); 2] = [
        ("strategy_config::new(", "__runtime_strategy_config_new("),
        ("anchored_config::new(", "__runtime_anchored_config_new("),
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

    engine.register_fn("get", strategy_state_get_int);
    engine.register_fn("get", strategy_state_get_float);
    engine.register_fn("get", strategy_state_get_bool);
    engine.register_fn("get", strategy_state_get_string);
    engine.register_fn("set", strategy_state_set_int);
    engine.register_fn("set", strategy_state_set_float);
    engine.register_fn("set", strategy_state_set_bool);
    engine.register_fn("set", strategy_state_set_string);
}

fn parse_timeframe(raw: &str) -> Result<Timeframe, Box<EvalAltResult>> {
    raw.parse::<Timeframe>()
        .map_err(|error| format!("invalid timeframe `{raw}`: {error}").into())
}

fn register_market_view_api(engine: &mut RhaiEngine) {
    engine.register_fn("candle", |market: &mut RhaiMarketView| {
        market.current.clone()
    });
    engine.register_fn("candles", |market: &mut RhaiMarketView| {
        market.primary_history.clone()
    });

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

fn register_indicator_api(engine: &mut RhaiEngine) {
    let mut indicators_module = Module::new();
    indicators_module.set_native_fn(
        "sma",
        |history: &mut RhaiCandleHistory, period: INT| -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(option_f64(sma(
                &history.chronological_closes_before_offset(0),
                non_negative_usize(period)?,
            )))
        },
    );
    indicators_module.set_native_fn(
        "sma",
        |history: &mut RhaiCandleHistory,
         period: INT,
         offset: INT|
         -> Result<Dynamic, Box<EvalAltResult>> {
            Ok(option_f64(sma(
                &history.chronological_closes_before_offset(non_negative_usize(offset)?),
                non_negative_usize(period)?,
            )))
        },
    );
    engine.register_static_module("indicators", Arc::new(indicators_module));
}

fn option_f64(value: Option<f64>) -> Dynamic {
    value.map(Dynamic::from).unwrap_or(Dynamic::UNIT)
}

fn non_negative_usize(value: INT) -> Result<usize, Box<EvalAltResult>> {
    usize::try_from(value).map_err(|_| "value must be a non-negative integer".into())
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
            _ => "fn hook()",
        },
    })
}

fn call_typed_strategy_config(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
) -> Result<StrategyConfiguration, RhaiStrategyLoadError> {
    let result = call_load_time_hook(engine, scope, ast, STRATEGY_CONFIG_HOOK)?;
    result
        .try_cast::<StrategyConfiguration>()
        .ok_or(RhaiStrategyLoadError::InvalidHookReturn {
            hook: STRATEGY_CONFIG_HOOK,
            expected: "a typed StrategyConfig from `strategy_config::new()`",
        })
}

fn call_typed_anchored_config(
    engine: &RhaiEngine,
    scope: &mut Scope<'static>,
    ast: &AST,
) -> Result<AnchoredConfiguration, RhaiStrategyLoadError> {
    let result = call_load_time_hook(engine, scope, ast, ANCHORED_CONFIG_HOOK)?;
    result
        .try_cast::<AnchoredConfiguration>()
        .ok_or(RhaiStrategyLoadError::InvalidHookReturn {
            hook: ANCHORED_CONFIG_HOOK,
            expected: "a typed AnchoredConfig from `anchored_config::new()`",
        })
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
        ExecutionAction, IgnoredDecisionReason, MarketInput, PortfolioState, RuntimeConfig,
        RuntimeEvent, SecondaryReadiness, StrategyDecisionIntent, TradingRuntime,
    };
    use shared::{Candle, Timeframe};

    const MINIMAL: &str = r#"
fn on_tick(market, context) {
    decision::hold()
}
"#;

    fn candle(close: f64) -> Candle {
        Candle {
            timestamp: 1,
            symbol: "BTC-USD".into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1_000.0,
            timeframe: Timeframe::minutes(1),
        }
    }

    fn source_returning(expression: &str) -> String {
        format!(
            r#"
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
    fn loads_minimal_strategy_with_only_on_tick() {
        let strategy = RhaiStrategy::load(MINIMAL).expect("strategy should load");

        assert_eq!(
            strategy.hooks(),
            RhaiStrategyHooks {
                has_on_tick: true,
                has_strategy_config: false,
                has_anchored_config: false,
            }
        );
        assert_eq!(
            strategy.strategy_config(),
            &StrategyConfiguration::default()
        );
        assert_eq!(strategy.anchored_config(), None);
    }

    #[test]
    fn loads_strategy_with_top_level_constants_and_typed_strategy_config() {
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_minimum_warmup(200)
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");

        assert!(strategy.hooks().has_strategy_config);
        assert_eq!(strategy.strategy_config().minimum_warmup(), 200);
    }

    #[test]
    fn typed_strategy_config_extracts_required_and_optional_secondaries() {
        let source = r#"
const H1 = timeframe("1h");
const D1 = timeframe("1d");

fn strategy_config() {
    strategy_config::new()
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
    fn runtime_config_merge_adds_strategy_only_secondaries() {
        let source = r#"
const H1 = timeframe("1h");
const D1 = timeframe("1d");

fn strategy_config() {
    strategy_config::new()
        .with_secondary(secondary::required(H1).with_max_missing_candles(1))
        .with_secondary(secondary::optional(D1).with_max_missing_candles(0))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let run_config = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1));

        let resolved = run_config.merge_strategy_config(strategy.strategy_config());

        assert_eq!(resolved.runtime_asset, "BTC-USD");
        assert_eq!(resolved.primary_timeframe, Timeframe::minutes(1));
        assert_eq!(
            resolved.secondary_timeframes,
            vec![
                SecondaryTimeframeConfig::required(Timeframe::hours(1), 1),
                SecondaryTimeframeConfig::optional(Timeframe::days(1), 0),
            ]
        );
    }

    #[test]
    fn runtime_config_merge_preserves_run_config_only_secondaries() {
        let run_config = RuntimeConfig::with_secondary_configs(
            "BTC-USD",
            Timeframe::minutes(1),
            [SecondaryTimeframeConfig::optional(Timeframe::hours(1), 2)],
        );

        let resolved = run_config.merge_strategy_config(&StrategyConfiguration::default());

        assert_eq!(resolved, run_config);
    }

    #[test]
    fn runtime_config_wins_secondary_conflicts() {
        let source = r#"
const H1 = timeframe("1h");

fn strategy_config() {
    strategy_config::new()
        .with_secondary(secondary::required(H1).with_max_missing_candles(1))
}

fn on_tick(market, context) {
    decision::hold()
}
"#;
        let strategy = RhaiStrategy::load(source).expect("strategy should load");
        let run_config = RuntimeConfig::with_secondary_configs(
            "BTC-USD",
            Timeframe::minutes(1),
            [SecondaryTimeframeConfig::optional(Timeframe::hours(1), 0)],
        );

        let resolved = run_config.merge_strategy_config(strategy.strategy_config());

        assert_eq!(resolved.runtime_asset, "BTC-USD");
        assert_eq!(resolved.primary_timeframe, Timeframe::minutes(1));
        assert_eq!(
            resolved.secondary_timeframes,
            vec![SecondaryTimeframeConfig::optional(Timeframe::hours(1), 0)]
        );
        assert_eq!(
            resolved.secondary_timeframes[0].readiness,
            SecondaryReadiness::Optional
        );
    }

    #[test]
    fn strategy_config_cannot_change_runtime_asset_or_primary_timeframe() {
        for forbidden in [
            r#"strategy_config::new().with_runtime_asset("ETH-USD")"#,
            r#"strategy_config::new().with_primary_timeframe(timeframe("1h"))"#,
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
    fn missing_optional_hooks_use_defaults() {
        let strategy = RhaiStrategy::load(MINIMAL).expect("strategy should load");

        assert_eq!(
            strategy.strategy_config(),
            &StrategyConfiguration::default()
        );
        assert!(strategy.anchored_config().is_none());
    }

    #[test]
    fn present_anchored_config_is_called_and_validated() {
        let source = r#"
fn anchored_config() {
    anchored_config::new()
}

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let strategy = RhaiStrategy::load(source).expect("strategy should load");

        assert!(strategy.hooks().has_anchored_config);
        assert_eq!(strategy.anchored_config(), Some(&AnchoredConfiguration));
    }

    #[test]
    fn optional_strategy_config_returning_unit_fails_load() {
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
    fn optional_strategy_config_returning_map_fails_load_without_legacy_mapping() {
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
/* anchored_config::new( */
fn strategy_config() { strategy_config::new() }
fn anchored_config() { anchored_config::new() }
"#;

        let normalized = normalize_reserved_constructor_names(source);

        assert!(normalized.contains("// strategy_config::new("));
        assert!(normalized.contains("\"strategy_config::new(\""));
        assert!(normalized.contains("/* anchored_config::new( */"));
        assert!(normalized.contains("fn strategy_config() { __runtime_strategy_config_new() }"));
        assert!(normalized.contains("fn anchored_config() { __runtime_anchored_config_new() }"));
    }

    #[test]
    fn invalid_top_level_timeframe_fails_initialization() {
        let source = r#"
const BAD = timeframe("15min");

fn on_tick(market, context) {
    decision::hold()
}
"#;

        let error = RhaiStrategy::load(source).unwrap_err();

        assert!(matches!(error, RhaiStrategyLoadError::Init { .. }));
        assert!(error.to_string().contains("invalid timeframe"));
    }
}
