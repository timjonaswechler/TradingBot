/// TOML-based runtime configuration for the trading daemon.
use serde::Deserialize;

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
pub struct AssetConfig {
    /// Ticker symbol, e.g. `"AAPL"` or `"BTC-USD"`.
    pub symbol: String,

    /// List of timeframes to track, e.g. `["1d", "1h"]`.
    pub intervals: Vec<String>,

    /// Path to the Rhai strategy file (only used by `run` subcommand).
    #[serde(default)]
    pub strategy: String,

    /// Starting paper-trading balance in USD (only used by `run` subcommand).
    #[serde(default = "default_balance")]
    pub balance: f64,

    /// On graceful shutdown, close any open position at the last observed
    /// candle close. When `false`, the position is left in `live_positions`
    /// and restored on next startup.
    #[serde(default = "default_liquidate")]
    pub liquidate_on_shutdown: bool,
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
