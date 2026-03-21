pub mod collector;
pub mod config;
pub mod db;
pub mod market_data;

// Stub: strategy module (Unit 4) — will be replaced by actual implementation
pub mod strategy {
    use crate::market_data::Candle;

    #[derive(Debug, Clone, PartialEq)]
    pub enum Signal { Buy, Sell, Hold, Short }

    pub trait Strategy: Send + Sync {
        fn name(&self) -> &str;
        fn required_history(&self) -> usize;
        fn signal(&self, primary: &[Candle], secondary: &[Candle]) -> Signal;
    }

    pub mod dual_macd {
        use super::Signal;
        use crate::market_data::Candle;

        #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
        pub struct DualMacdParams {
            pub fast: usize,
            pub slow: usize,
            pub signal: usize,
            pub primary_crossover_weight: f64,
            pub primary_histogram_weight: f64,
            pub primary_slope_weight: f64,
            pub primary_slope_lookback: usize,
            pub secondary_drop_threshold: f64,
            pub secondary_drop_weight: f64,
            pub secondary_slope_lookback: usize,
            pub month_start_boost: f64,
            pub month_start_days: usize,
            pub month_end_caution: f64,
            pub month_end_days: usize,
            pub quarter_end_caution: f64,
            pub year_end_boost: f64,
            pub crash_atr_multiplier: f64,
            pub bull_trend_threshold: f64,
            pub bear_trend_threshold: f64,
            pub long_ema_period: usize,
            pub atr_period: usize,
            pub atr_median_period: usize,
            pub buy_threshold: f64,
            pub sell_threshold: f64,
            pub short_threshold: f64,
        }

        pub struct DualMacdStrategy {
            pub params: DualMacdParams,
        }

        impl super::Strategy for DualMacdStrategy {
            fn name(&self) -> &str { "dual_macd" }
            fn required_history(&self) -> usize { 300 }
            fn signal(&self, _primary: &[Candle], _secondary: &[Candle]) -> Signal { Signal::Hold }
        }
    }
}

// Stub: paper_trading (Unit 5) — will be replaced by actual implementation
pub mod paper_trading {
    use chrono::{DateTime, Utc};
    use crate::market_data::Candle;

    #[derive(Debug, Clone)]
    pub enum TradeSide { Buy, Sell, Short, Cover }

    #[derive(Debug, Clone)]
    pub struct Trade {
        pub side: TradeSide,
        pub quantity: i64,
        pub price_cents: i64,
        pub timestamp: i64,
        pub pnl_cents: i64,
        pub commission_cents: i64,
        pub asset: String,
        pub price: i64,
        pub fee: i64,
        pub strategy: String,
        pub gain_loss: Option<i64>,
        pub gain_loss_pct: Option<f64>,
        pub tax: Option<i64>,
    }

    /// Position held in paper trading.
    #[derive(Debug, Clone)]
    pub struct Position {
        pub asset: String,
        pub quantity: i64,
        pub avg_buy_price: i64,
    }

    #[derive(Debug, Clone, Default)]
    pub struct TradingConfig {
        pub starting_capital_cents: i64,
    }

    pub struct PaperTradingEngine {
        pub trades: Vec<Trade>,
        pub equity_curve: Vec<(DateTime<Utc>, i64)>,
        pub positions: Vec<Position>,
        pub cash: i64,
        pub exemption_remaining: i64,
        starting_capital: i64,
    }

    impl PaperTradingEngine {
        pub fn new(cfg: TradingConfig) -> Self {
            let cap = cfg.starting_capital_cents;
            Self {
                trades: vec![],
                equity_curve: vec![],
                positions: vec![],
                cash: cap,
                exemption_remaining: 0,
                starting_capital: cap,
            }
        }

        pub fn execute(&mut self, _signal: &crate::strategy::Signal, _asset: &str, _candle: &Candle) {}

        pub fn snapshot_equity(&mut self, _asset: &str, price: i64, ts: DateTime<Utc>) {
            self.equity_curve.push((ts, price));
        }

        pub fn total_equity_cents(&self) -> i64 { self.starting_capital }

        pub fn total_value(&self, _prices: &std::collections::HashMap<String, i64>) -> i64 {
            self.starting_capital
        }
    }

}

// Stub: metrics (Unit 6) — will be replaced by actual implementation
pub mod metrics {
    use chrono::{DateTime, Utc};

    #[derive(Debug, Clone, Default)]
    pub struct Metrics {
        pub total_trades: usize,
        pub win_rate_pct: f64,
        pub avg_win_pct: f64,
        pub avg_loss_pct: f64,
        pub expectancy_pct: f64,
        pub sharpe: f64,
        pub max_drawdown_pct: f64,
        pub total_return_pct: f64,
        pub total_pnl_cents: i64,
    }

    #[derive(Debug, Clone)]
    pub struct TradeRecord {
        pub timestamp: DateTime<Utc>,
        pub pnl_cents: i64,
        pub entry_price_cents: i64,
        pub exit_price_cents: i64,
        pub quantity: i64,
        pub commission_cents: i64,
    }

    pub fn compute(
        _equity_curve: &[(DateTime<Utc>, i64)],
        _trades: &[TradeRecord],
        _starting_capital: i64,
    ) -> Metrics {
        Metrics::default()
    }

    pub fn from_engine_trades(_trades: &[crate::paper_trading::Trade]) -> Vec<TradeRecord> {
        vec![]
    }
}

// Stub: optimizer (Unit 7) — will be replaced by actual implementation
pub mod optimizer {
    use std::collections::HashMap;
    use crate::market_data::Candle;

    pub type CandlePool = HashMap<(String, String), Vec<Candle>>;

    #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
    pub struct DualMacdGenome {
        pub params: crate::strategy::dual_macd::DualMacdParams,
        pub primary_interval: String,
        pub secondary_interval: String,
    }

    impl DualMacdGenome {
        pub fn default_genome() -> Self {
            Self {
                params: Default::default(),
                primary_interval: "1d".to_string(),
                secondary_interval: "1h".to_string(),
            }
        }

        pub fn to_toml(&self) -> String { String::new() }

        pub fn from_toml(_s: &str) -> Result<Self, Box<dyn std::error::Error>> {
            Ok(Self::default_genome())
        }
    }

    pub struct OptimizationResult {
        pub winner: DualMacdGenome,
        pub best_fitness: f64,
        pub generations: Vec<()>,
    }

    #[derive(Default)]
    pub struct OptimizerConfig {
        pub population_size: usize,
        pub max_generations: usize,
        pub initial_mutation: f64,
        pub assets: Vec<String>,
    }

    pub fn run(_cfg: OptimizerConfig, _pool: &CandlePool) -> OptimizationResult {
        OptimizationResult {
            winner: DualMacdGenome::default_genome(),
            best_fitness: 0.0,
            generations: vec![],
        }
    }
}
