/// TOML-based runtime configuration for the trading daemon.
use serde::{de, Deserialize, Deserializer};
use std::fmt;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub database: DatabaseConfig,

    #[serde(default)]
    pub seed: SeedConfig,

    #[serde(default)]
    pub assets: Vec<AssetConfig>,
}

#[derive(Debug, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_url")]
    pub url: String,

    #[serde(default = "default_db_module")]
    pub module: String,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: default_db_url(),
            module: default_db_module(),
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct SeedConfig {
    /// Default start date for seeding historical data (ISO 8601: "2020-01-01").
    #[serde(default = "default_seed_from")]
    pub from: String,
}

impl Default for SeedConfig {
    fn default() -> Self {
        Self {
            from: default_seed_from(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetConfig {
    /// Ticker symbol, e.g. `"AAPL"` or `"BTC-USD"`.
    pub symbol: String,

    /// Path to the Rhai strategy file. The strategy's `strategy_config()`
    /// declares the Primary and Secondary Timeframes used by run/seed.
    #[serde(default)]
    pub strategy: String,

    /// Live execution mode for this Live Runner session.
    pub execution_mode: LiveExecutionMode,

    /// Operator-owned Strategy Identity used by persistent Paper Trading.
    #[serde(default)]
    pub strategy_identity: Option<String>,

    /// Starting paper-trading balance in USD (only used by `run` subcommand).
    #[serde(default = "default_balance")]
    pub balance: f64,

    /// On graceful shutdown, close any open position at the last observed
    /// candle close. When `false`, a Paper Trading position is left in
    /// `paper_open_positions` and restored on next startup.
    #[serde(default = "default_liquidate")]
    pub liquidate_on_shutdown: bool,

    /// Live-runner safety policy for repeated required Secondary context loss.
    #[serde(default)]
    pub protective_shutdown: ProtectiveShutdownConfig,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveExecutionMode {
    PaperTrading,
    RealMoney,
}

impl fmt::Display for LiveExecutionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LiveExecutionMode::PaperTrading => write!(f, "paper_trading"),
            LiveExecutionMode::RealMoney => write!(f, "real_money"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectiveShutdownConfig {
    /// Enable Protective Runner Shutdown for repeated required Secondary blocks.
    pub enabled: bool,

    /// Number of consecutive Primary candles blocked by the same required
    /// Secondary Timeframe before the live runner stops that runtime.
    pub required_secondary_failure_threshold: u32,
}

impl Default for ProtectiveShutdownConfig {
    fn default() -> Self {
        Self {
            enabled: default_protective_shutdown_enabled(),
            required_secondary_failure_threshold: default_required_secondary_failure_threshold(),
        }
    }
}

impl<'de> Deserialize<'de> for ProtectiveShutdownConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct RawProtectiveShutdownConfig {
            #[serde(default = "default_protective_shutdown_enabled")]
            enabled: bool,
            #[serde(default = "default_required_secondary_failure_threshold")]
            required_secondary_failure_threshold: u32,
        }

        let raw = RawProtectiveShutdownConfig::deserialize(deserializer)?;
        if raw.required_secondary_failure_threshold == 0 {
            return Err(de::Error::custom(
                "protective_shutdown.required_secondary_failure_threshold must be greater than 0; set protective_shutdown.enabled = false to disable the policy",
            ));
        }

        Ok(Self {
            enabled: raw.enabled,
            required_secondary_failure_threshold: raw.required_secondary_failure_threshold,
        })
    }
}

// ── Defaults ─────────────────────────────────────────────────────────────────

fn default_db_url() -> String {
    "http://127.0.0.1:3000".into()
}
fn default_db_module() -> String {
    "trading-bot".into()
}
fn default_seed_from() -> String {
    "2020-01-01".into()
}
fn default_balance() -> f64 {
    10_000.0
}
fn default_liquidate() -> bool {
    true
}
fn default_protective_shutdown_enabled() -> bool {
    true
}
fn default_required_secondary_failure_threshold() -> u32 {
    3
}

// ── Loader ───────────────────────────────────────────────────────────────────

impl Config {
    pub fn load(path: &str) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| anyhow::anyhow!("Cannot read config file '{}': {}", path, e))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Invalid TOML in '{}': {}", path, e))?;
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_asset(toml: &str) -> Result<Config, toml::de::Error> {
        toml::from_str(toml)
    }

    #[test]
    fn protective_shutdown_defaults_to_enabled_with_threshold_three() {
        let config = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
strategy = "strategy.rhai"
execution_mode = "paper_trading"
strategy_identity = "btc-paper"
"#,
        )
        .expect("config should parse");

        assert_eq!(
            config.assets[0].protective_shutdown,
            ProtectiveShutdownConfig::default()
        );
        assert!(config.assets[0].protective_shutdown.enabled);
        assert_eq!(
            config.assets[0]
                .protective_shutdown
                .required_secondary_failure_threshold,
            3
        );
    }

    #[test]
    fn protective_shutdown_nested_config_overrides_defaults() {
        let config = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
strategy = "strategy.rhai"
execution_mode = "paper_trading"
strategy_identity = "btc-paper"

[assets.protective_shutdown]
enabled = false
required_secondary_failure_threshold = 5
"#,
        )
        .expect("config should parse");

        assert_eq!(
            config.assets[0].protective_shutdown,
            ProtectiveShutdownConfig {
                enabled: false,
                required_secondary_failure_threshold: 5,
            }
        );
    }

    #[test]
    fn live_execution_mode_parses_paper_trading_and_real_money_modes() {
        let config = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
strategy = "paper.rhai"
execution_mode = "paper_trading"
strategy_identity = "btc-paper"

[[assets]]
symbol = "ETH-USD"
strategy = "real.rhai"
execution_mode = "real_money"
"#,
        )
        .expect("execution modes should parse");

        assert_eq!(
            config.assets[0].execution_mode,
            LiveExecutionMode::PaperTrading
        );
        assert_eq!(
            config.assets[1].execution_mode,
            LiveExecutionMode::RealMoney
        );
        assert_eq!(
            config.assets[0].strategy_identity.as_deref(),
            Some("btc-paper")
        );
    }

    #[test]
    fn asset_execution_mode_is_required_for_live_runner_mode_selection() {
        let error = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
strategy = "strategy.rhai"
"#,
        )
        .expect_err("execution mode should be explicit");

        assert!(error.to_string().contains("missing field `execution_mode`"));
    }

    #[test]
    fn asset_intervals_are_rejected_because_strategy_configuration_owns_timeframes() {
        let error = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
intervals = ["1m"]
strategy = "strategy.rhai"
execution_mode = "paper_trading"
strategy_identity = "btc-paper"
"#,
        )
        .expect_err("intervals should no longer be accepted");

        assert!(error.to_string().contains("unknown field `intervals`"));
    }

    #[test]
    fn protective_shutdown_rejects_zero_threshold() {
        let error = parse_asset(
            r#"
[[assets]]
symbol = "BTC-USD"
strategy = "strategy.rhai"
execution_mode = "paper_trading"
strategy_identity = "btc-paper"

[assets.protective_shutdown]
required_secondary_failure_threshold = 0
"#,
        )
        .expect_err("zero threshold should be rejected");

        assert!(error.to_string().contains(
            "protective_shutdown.required_secondary_failure_threshold must be greater than 0"
        ));
    }
}
