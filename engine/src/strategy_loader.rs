use crate::EngineError;
use std::path::Path;

/// Minimal metadata declared at the top of every `.rhai` strategy file.
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub name: String,
}

/// Load a `.rhai` strategy file from disk and return source + config.
///
/// Validates:
/// 1. File exists and is readable.
/// 2. Rhai source compiles without syntax errors.
/// 3. A function `on_tick` is defined.
pub fn load_strategy_file(path: &Path) -> Result<(String, StrategyConfig), EngineError> {
    let source = std::fs::read_to_string(path)?;
    let name = extract_name(&source);
    validate_strategy_source(&source)?;
    Ok((source, StrategyConfig { name }))
}

/// Validate Rhai source without running it (useful for tests).
pub fn validate_strategy_source(source: &str) -> Result<(), EngineError> {
    let mut engine = rhai::Engine::new();
    crate::candle_wrapper::register_types(&mut engine);
    crate::bindings::register_all(&mut engine);

    let ast = engine
        .compile(source)
        .map_err(|e| EngineError::Strategy(format!("compile error: {e}")))?;

    let has_on_tick = ast.iter_functions().any(|f| f.name == "on_tick");
    if !has_on_tick {
        return Err(EngineError::Strategy(
            "strategy must define `fn on_tick(candles, context)`".into(),
        ));
    }

    Ok(())
}

/// Best-effort: scan for a `// name: "..."` comment or `const NAME = "..."`.
/// Falls back to `"unnamed"`.
fn extract_name(source: &str) -> String {
    for line in source.lines() {
        // Support: // name: "sma_cross"  or  const NAME = "sma_cross";
        let trimmed = line.trim();
        if let Some(pos) = trimmed.find("name") {
            let after = trimmed[pos + 4..].trim_start();
            let after = after
                .trim_start_matches(':')
                .trim_start_matches('=')
                .trim_start();
            if after.starts_with('"') {
                if let Some(end) = after[1..].find('"') {
                    return after[1..1 + end].to_string();
                }
            }
        }
    }
    "unnamed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
// name: "test_strategy"
fn on_tick(candles, context) {
    #{ signal: "HOLD" }
}
"#;

    const NO_ON_TICK: &str = r#"let x = 1;"#;

    const SYNTAX_ERROR: &str = r#"fn on_tick(candles "#;

    #[test]
    fn valid_strategy_passes() {
        assert!(validate_strategy_source(VALID).is_ok());
    }

    #[test]
    fn missing_on_tick_fails() {
        assert!(matches!(
            validate_strategy_source(NO_ON_TICK).unwrap_err(),
            EngineError::Strategy(_)
        ));
    }

    #[test]
    fn syntax_error_fails() {
        assert!(matches!(
            validate_strategy_source(SYNTAX_ERROR).unwrap_err(),
            EngineError::Strategy(_)
        ));
    }

    #[test]
    fn name_extraction() {
        assert_eq!(extract_name(VALID), "test_strategy");
    }

    #[test]
    fn name_extraction_fallback() {
        assert_eq!(extract_name("fn on_tick(c, x) {}"), "unnamed");
    }
}
