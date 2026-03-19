use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub paper_trading: PaperTradingConfig,
    pub costs:         CostsConfig,
    pub tax:           TaxConfig,
    pub assets:        AssetsConfig,
    pub strategy:      StrategyConfig,
    pub db:            DbConfig,
    #[serde(default)]
    pub data:          DataConfig,
}

#[derive(Debug, Deserialize)]
pub struct PaperTradingConfig {
    pub starting_capital:  i64, // in Cent
    pub position_size_pct: u8,  // % des Cashs der pro Trade investiert wird (1–100)
}

#[derive(Debug, Clone, Deserialize)]
pub struct CostsConfig {
    pub commission_type:   String, // "flat" | "percent"
    pub commission_amount: i64,    // Cent (flat) oder Basispunkte (percent)
}

#[derive(Debug, Clone, Deserialize)]
pub struct TaxConfig {
    pub country:              String,
    pub freistellungsauftrag: i64, // in Cent
    pub kirchensteuer:        bool,
}

#[derive(Debug, Deserialize)]
pub struct AssetsConfig {
    pub watchlist: Vec<String>,
}

/// Konfiguration für die Datenbeschaffung (Yahoo Finance).
#[derive(Debug, Deserialize)]
pub struct DataConfig {
    /// Candle-Intervalle die heruntergeladen werden: ["1d", "1h", "1wk"]
    /// Das erste Intervall wird als Primärquelle für Strategie & Backtest verwendet.
    pub intervals: Vec<String>,
    /// Historischer Zeitraum für den Erstabzug: "1y", "2y", "5y", "max"
    pub range:     String,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self { intervals: vec!["1d".into()], range: "2y".into() }
    }
}

impl DataConfig {
    /// Das primäre Intervall (erstes in der Liste) für Strategie & Backtest.
    pub fn primary_interval(&self) -> &str {
        self.intervals.first().map(|s| s.as_str()).unwrap_or("1d")
    }
}

/// Strategie-Parameter – alle spezifischen Felder sind optional
/// damit ungenutzte Felder nicht in config.toml erscheinen müssen.
#[derive(Debug, Deserialize)]
pub struct StrategyConfig {
    pub name: String,

    // SMA Crossover
    #[serde(default = "default_short_period")]
    pub short_period: usize,
    #[serde(default = "default_long_period")]
    pub long_period:  usize,

    // RSI
    pub rsi_period:     Option<usize>,
    pub rsi_oversold:   Option<f64>,
    pub rsi_overbought: Option<f64>,

    // MACD
    pub macd_fast:   Option<usize>,
    pub macd_slow:   Option<usize>,
    pub macd_signal: Option<usize>,

    // Bollinger Bands
    pub bb_period: Option<usize>,
    pub bb_k:      Option<f64>,
}

fn default_short_period() -> usize { 10 }
fn default_long_period()  -> usize { 50 }

#[derive(Debug, Deserialize)]
pub struct DbConfig {
    pub path: String,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&content)?)
    }
}
