//! Rhai strategy loading and hook validation for the trading runtime.
//!
//! This module owns compile/load-time Rhai strategy handling inside the
//! `trading-runtime` crate. Tick execution and the full strategy-facing typed
//! APIs are implemented in later Strategy Handling slices.

use crate::StrategyDecision;
use rhai::{Dynamic, Engine as RhaiEngine, EvalAltResult, Module, Scope, AST, INT};
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

    // Minimal placeholder for loader smoke tests. Full typed decision execution
    // is intentionally left to the Strategy Decision Rhai API issue.
    let mut decision_module = Module::new();
    decision_module.set_native_fn("hold", || Ok(StrategyDecision::hold()));
    engine.register_static_module("decision", Arc::new(decision_module));

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

    const MINIMAL: &str = r#"
fn on_tick(market, context) {
    decision::hold()
}
"#;

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
