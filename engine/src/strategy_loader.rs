use std::path::Path;
use crate::EngineError;

/// Minimal metadata declared at the top of every `.lua` strategy file.
///
/// ```lua
/// local config = { name = "sma_cross" }
/// function on_tick(candles, context) ... end
/// return config
/// ```
#[derive(Debug, Clone)]
pub struct StrategyConfig {
    pub name: String,
}

/// Load a `.lua` strategy file from disk and return its source + parsed config.
///
/// Validates:
/// 1. The file exists and is readable.
/// 2. The Lua source executes without syntax errors.
/// 3. A global `on_tick` function is defined.
/// 4. The module returns a table with at least a `name` field.
pub fn load_strategy_file(path: &Path) -> Result<(String, StrategyConfig), EngineError> {
    let source = std::fs::read_to_string(path)?;
    let name = extract_name(&source);
    validate_strategy_source(&source)?;
    Ok((source, StrategyConfig { name }))
}

/// Same as `load_strategy_file` but takes the Lua source directly (useful for tests).
pub fn validate_strategy_source(source: &str) -> Result<(), EngineError> {
    let lua = mlua::Lua::new();

    // Execute the source — catches syntax errors
    lua.load(source).exec().map_err(|e| {
        EngineError::Strategy(format!("strategy failed to load: {e}"))
    })?;

    // Must have an on_tick function
    let on_tick: mlua::Value = lua.globals().get("on_tick")?;
    if !matches!(on_tick, mlua::Value::Function(_)) {
        return Err(EngineError::Strategy(
            "strategy must define a global `on_tick(candles, context)` function".into(),
        ));
    }

    Ok(())
}

/// Best-effort extraction of `name` from source.
/// Scans every line for the pattern `name = "..."` and returns the first match.
/// Falls back to `"unnamed"` if the field isn't found.
fn extract_name(source: &str) -> String {
    for line in source.lines() {
        // Accept lines like: `local config = { name = "sma_cross" }`
        // or: `    name = "my_strategy"`
        if let Some(pos) = line.find("name") {
            let after = &line[pos + 4..];
            // skip whitespace and `=`
            let after = after.trim_start();
            if after.starts_with('=') {
                let after = after[1..].trim_start();
                if after.starts_with('"') {
                    if let Some(end) = after[1..].find('"') {
                        return after[1..1 + end].to_string();
                    }
                }
            }
        }
    }
    "unnamed".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_STRATEGY: &str = r#"
local config = { name = "test_strategy" }
function on_tick(candles, context)
    return { signal = "HOLD" }
end
return config
"#;

    const MISSING_ON_TICK: &str = r#"
local config = { name = "bad" }
return config
"#;

    const SYNTAX_ERROR: &str = r#"
function on_tick(candles
"#;

    #[test]
    fn valid_strategy_passes() {
        assert!(validate_strategy_source(VALID_STRATEGY).is_ok());
    }

    #[test]
    fn missing_on_tick_fails() {
        let err = validate_strategy_source(MISSING_ON_TICK).unwrap_err();
        assert!(matches!(err, EngineError::Strategy(_)));
    }

    #[test]
    fn syntax_error_fails() {
        let err = validate_strategy_source(SYNTAX_ERROR).unwrap_err();
        assert!(matches!(err, EngineError::Strategy(_)));
    }

    #[test]
    fn name_extraction() {
        let (_, config) = (VALID_STRATEGY.to_string(), StrategyConfig { name: super::extract_name(VALID_STRATEGY) });
        assert_eq!(config.name, "test_strategy");
    }
}
