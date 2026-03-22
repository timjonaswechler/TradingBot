use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub paper_trading: PaperTradingConfigFile,
    #[serde(default)]
    pub costs: CostsConfig,
    #[serde(default)]
    pub tax: TaxConfigFile,
    pub assets: AssetsConfig,
    #[serde(default)]
    pub strategy: StrategyConfig,
    pub db: DbConfig,
    #[serde(default)]
    pub data: DataConfig,
    #[serde(default)]
    pub optimizer: OptimizerConfigFile,
}

// ── Strategy ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize, Default)]
pub struct StrategyConfig {
    #[serde(default = "default_primary_interval")]
    pub primary_interval: String,
    #[serde(default = "default_secondary_interval")]
    pub secondary_interval: String,
    #[serde(default = "default_macd_fast")]
    pub macd_fast: usize,
    #[serde(default = "default_macd_slow")]
    pub macd_slow: usize,
    #[serde(default = "default_macd_signal")]
    pub macd_signal: usize,
}

fn default_primary_interval() -> String { "1d".to_string() }
fn default_secondary_interval() -> String { "1h".to_string() }
fn default_macd_fast() -> usize { 12 }
fn default_macd_slow() -> usize { 26 }
fn default_macd_signal() -> usize { 9 }

// ── Fitness Weights ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct FitnessWeightsConfig {
    #[serde(default = "fw_one")]    pub sharpe: f64,
    #[serde(default = "fw_half")]   pub win_rate: f64,
    #[serde(default = "fw_one")]    pub expectancy: f64,
    #[serde(default = "fw_third")]  pub avg_win: f64,
    #[serde(default = "fw_third")]  pub avg_loss: f64,
    #[serde(default = "fw_half")]   pub drawdown: f64,
    #[serde(default = "fw_five")]   pub min_trades: usize,
}

impl Default for FitnessWeightsConfig {
    fn default() -> Self {
        Self {
            sharpe:     fw_one(),
            win_rate:   fw_half(),
            expectancy: fw_one(),
            avg_win:    fw_third(),
            avg_loss:   fw_third(),
            drawdown:   fw_half(),
            min_trades: fw_five(),
        }
    }
}

fn fw_one()   -> f64   { 1.0 }
fn fw_half()  -> f64   { 0.5 }
fn fw_third() -> f64   { 0.3 }
fn fw_five()  -> usize { 5   }

// ── Optimizer ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct OptimizerConfigFile {
    #[serde(default = "opt_pop_25")]       pub population_size: usize,
    #[serde(default = "opt_gen_100")]      pub max_generations: usize,
    #[serde(default = "opt_mut_08")]       pub initial_mutation: f64,
    #[serde(default = "opt_decay_097")]    pub mutation_decay: f64,
    #[serde(default = "opt_win_50")]       pub min_window_candles: usize,
    #[serde(default)]                      pub assets: Vec<String>,
    #[serde(default)]                      pub fitness: FitnessWeightsConfig,
}

impl Default for OptimizerConfigFile {
    fn default() -> Self {
        Self {
            population_size:    opt_pop_25(),
            max_generations:    opt_gen_100(),
            initial_mutation:   opt_mut_08(),
            mutation_decay:     opt_decay_097(),
            min_window_candles: opt_win_50(),
            assets:             Vec::new(),
            fitness:            FitnessWeightsConfig::default(),
        }
    }
}

fn opt_pop_25()    -> usize { 25   }
fn opt_gen_100()   -> usize { 100  }
fn opt_mut_08()    -> f64   { 0.8  }
fn opt_decay_097() -> f64   { 0.97 }
fn opt_win_50()    -> usize { 50   }

// ── Paper Trading ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct PaperTradingConfigFile {
    #[serde(default = "pt_cap_1m")]       pub starting_capital: i64,
    #[serde(default = "pt_comm_flat")]    pub commission_type: String,
    #[serde(default = "pt_comm_100")]     pub commission_amount: i64,
    #[serde(default = "pt_pos_095")]      pub position_size_pct: f64,
    #[serde(default = "pt_short_050")]    pub max_short_size_pct: f64,
}

impl Default for PaperTradingConfigFile {
    fn default() -> Self {
        Self {
            starting_capital:  pt_cap_1m(),
            commission_type:   pt_comm_flat(),
            commission_amount: pt_comm_100(),
            position_size_pct: pt_pos_095(),
            max_short_size_pct: pt_short_050(),
        }
    }
}

fn pt_cap_1m()    -> i64    { 1_000_000   }
fn pt_comm_flat() -> String { "flat".to_string() }
fn pt_comm_100()  -> i64    { 100         }
fn pt_pos_095()   -> f64    { 0.95        }
fn pt_short_050() -> f64    { 0.50        }

// ── Tax ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct TaxConfigFile {
    #[serde(default = "tax_country_de")]   pub country: String,
    #[serde(default = "tax_freistellung")] pub freistellungsauftrag: i64,
    #[serde(default)]                      pub kirchensteuer: bool,
}

impl Default for TaxConfigFile {
    fn default() -> Self {
        Self {
            country:              tax_country_de(),
            freistellungsauftrag: tax_freistellung(),
            kirchensteuer:        false,
        }
    }
}

fn tax_country_de()   -> String { "DE".to_string() }
fn tax_freistellung() -> i64    { 100_100 }

// ── Costs (kept for backward compatibility) ───────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct CostsConfig {
    #[serde(default = "costs_comm_flat")]   pub commission_type: String,
    #[serde(default = "costs_comm_100")]    pub commission_amount: i64,
}

impl Default for CostsConfig {
    fn default() -> Self {
        Self {
            commission_type:   costs_comm_flat(),
            commission_amount: costs_comm_100(),
        }
    }
}

fn costs_comm_flat() -> String { "flat".to_string() }
fn costs_comm_100()  -> i64    { 100 }

// ── Assets ────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
pub struct AssetsConfig {
    #[serde(default)]
    pub watchlist: Vec<String>,
    pub watchlist_file: Option<String>,
}

// ── Data ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DataConfig {
    pub intervals: Vec<String>,
    pub range: String,
}

impl Default for DataConfig {
    fn default() -> Self {
        Self { intervals: vec!["1d".into()], range: "2y".into() }
    }
}

impl DataConfig {
    pub fn primary_interval(&self) -> &str {
        self.intervals.first().map(|s| s.as_str()).unwrap_or("1d")
    }
}

// ── Db ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct DbConfig {
    pub path: String,
}

// ── Config loader ─────────────────────────────────────────────────────────────

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("config file '{}' not found", path.display()))?;
        let mut cfg: Config = toml::from_str(&content)
            .with_context(|| format!("failed to parse config '{}'", path.display()))?;

        if let Some(ref wl_file) = cfg.assets.watchlist_file.clone() {
            let base = path.parent().unwrap_or(Path::new("."));
            let wl_path = base.join(wl_file);
            let text = std::fs::read_to_string(&wl_path)
                .with_context(|| format!("watchlist_file '{}' not found", wl_path.display()))?;
            cfg.assets.watchlist = text
                .lines()
                .map(|l| l.trim())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.split('#').next().unwrap_or(l).trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            log::info!("Watchlist: {} symbols from '{}'", cfg.assets.watchlist.len(), wl_file);
        }

        Ok(cfg)
    }
}
