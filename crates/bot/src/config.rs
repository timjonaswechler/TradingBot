use anyhow::{Context, Result};
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
    pub optimizer:     Option<OptimizerConfig>,
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
    /// Inline-Liste (fallback wenn watchlist_file nicht gesetzt)
    #[serde(default)]
    pub watchlist: Vec<String>,
    /// Pfad zu einer Textdatei: ein Ticker pro Zeile, # = Kommentar
    pub watchlist_file: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct OptimizerConfig {
    /// Strategie-Name: "macd_enhanced", "rsi", "bollinger", "sma_crossover"
    pub strategy: String,
    /// Anzahl Bots pro Generation (wird intern in 2 Gruppen geteilt)
    pub population_size: usize,
    /// Maximale Anzahl Generationen
    pub max_generations: u32,
    /// Mutationsstärke: 0.0 = keine Änderung, 1.0 = vollständig zufällig
    pub mutation_magnitude: f64,
    /// Minimale Fenstergröße in Candles für eine Bewertung
    pub min_window_candles: usize,
    /// Fitness-Gewichte
    #[serde(default)]
    pub fitness: FitnessWeights,
}

#[derive(Debug, Deserialize)]
pub struct FitnessWeights {
    /// Sharpe Ratio (risikoadjustierte Rendite, normiert)
    #[serde(default = "fw_sharpe")]
    pub sharpe: f64,
    /// Win-Rate (Anteil gewonnener Trades, 0–1)
    #[serde(default = "fw_win_rate")]
    pub win_rate: f64,
    /// Ø Gewinn pro Gewinn-Trade in % (höher = besser)
    #[serde(default = "fw_avg_win")]
    pub avg_win: f64,
    /// Strafe: Ø Verlust pro Verlust-Trade in % (wird subtrahiert)
    #[serde(default = "fw_avg_loss")]
    pub avg_loss: f64,
    /// Erwartungswert pro Trade in % (win_rate×avg_win − loss_rate×avg_loss)
    #[serde(default = "fw_expectancy")]
    pub expectancy: f64,
    /// Strafe für maximalen Drawdown in % (wird subtrahiert)
    #[serde(default = "fw_drawdown")]
    pub drawdown: f64,
    /// Mindestanzahl Trades — weniger → Fitness = -∞
    #[serde(default = "fw_min_trades")]
    pub min_trades: usize,
}

impl Default for FitnessWeights {
    fn default() -> Self {
        Self {
            sharpe:     fw_sharpe(),
            win_rate:   fw_win_rate(),
            avg_win:    fw_avg_win(),
            avg_loss:   fw_avg_loss(),
            expectancy: fw_expectancy(),
            drawdown:   fw_drawdown(),
            min_trades: fw_min_trades(),
        }
    }
}

fn fw_sharpe()     -> f64   { 1.0 }
fn fw_win_rate()   -> f64   { 0.5 }
fn fw_avg_win()    -> f64   { 0.3 }
fn fw_avg_loss()   -> f64   { 0.3 }
fn fw_expectancy() -> f64   { 1.0 }
fn fw_drawdown()   -> f64   { 0.5 }
fn fw_min_trades() -> usize { 5   }

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let mut cfg: Config = toml::from_str(&content)?;

        if let Some(ref wl_file) = cfg.assets.watchlist_file.clone() {
            let base = path.parent().unwrap_or(Path::new("."));
            let wl_path = base.join(wl_file);
            let text = std::fs::read_to_string(&wl_path)
                .with_context(|| format!("watchlist_file '{}' nicht gefunden", wl_path.display()))?;
            cfg.assets.watchlist = text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split('#').next().unwrap_or(l).trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            log::info!("Watchlist: {} Symbole aus '{}'", cfg.assets.watchlist.len(), wl_file);
        }

        Ok(cfg)
    }
}
