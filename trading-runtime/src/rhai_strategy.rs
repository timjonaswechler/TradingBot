//! Rhai strategy loading and hook validation for the trading runtime.
//!
//! This module owns compile/load-time Rhai strategy handling inside the
//! `trading-runtime` crate and the typed decision API used at the strategy tick
//! boundary. Full strategy-facing Market View and Context APIs are implemented
//! in later Strategy Handling slices.

use crate::{
    StrategyDecision, StrategyError, StrategyHandler, StrategyTickInput, StrategyTickResult,
};
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, FLOAT, INT};
use shared::Timeframe;
use std::{error::Error, fmt, path::Path, sync::Arc};

/// Typed strategy-declared configuration returned by `strategy_config()`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StrategyConfiguration {
    minimum_warmup: usize,
}

impl StrategyConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn minimum_warmup(&self) -> usize {
        self.minimum_warmup
    }

    fn with_minimum_warmup(mut self, minimum_warmup: usize) -> Self {
        self.minimum_warmup = minimum_warmup;
        self
    }
}

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

impl StrategyHandler for RhaiStrategy {
    fn on_tick(&mut self, _input: StrategyTickInput<'_>) -> StrategyTickResult {
        let result =
            match self
                .engine
                .call_fn::<Dynamic>(&mut self.scope, &self.ast, ON_TICK_HOOK, ((), ()))
            {
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
    engine.register_type_with_name::<AnchoredConfiguration>("AnchoredConfig");
    engine.register_type_with_name::<StrategyDecision>("StrategyDecision");

    engine.register_fn("timeframe", parse_timeframe);
    engine.register_fn(
        "with_minimum_warmup",
        |config: StrategyConfiguration, minimum_warmup: INT| {
            with_minimum_warmup(config, minimum_warmup)
        },
    );

    engine.register_fn("__runtime_strategy_config_new", StrategyConfiguration::new);
    engine.register_fn("__runtime_anchored_config_new", AnchoredConfiguration::new);

    let mut strategy_config_module = Module::new();
    strategy_config_module.set_native_fn("new", || Ok(StrategyConfiguration::new()));
    engine.register_static_module("strategy_config", Arc::new(strategy_config_module));

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

fn parse_timeframe(raw: &str) -> Result<Timeframe, Box<EvalAltResult>> {
    raw.parse::<Timeframe>()
        .map_err(|error| format!("invalid timeframe `{raw}`: {error}").into())
}

fn with_minimum_warmup(
    config: StrategyConfiguration,
    minimum_warmup: INT,
) -> Result<StrategyConfiguration, Box<EvalAltResult>> {
    let minimum_warmup = usize::try_from(minimum_warmup)
        .map_err(|_| "minimum warmup must be a non-negative integer".to_string())?;

    Ok(config.with_minimum_warmup(minimum_warmup))
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
        ExecutionAction, IgnoredDecisionReason, MarketInput, PortfolioState, RuntimeEvent,
        StrategyDecisionIntent, TradingRuntime,
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
