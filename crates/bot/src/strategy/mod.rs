pub mod bollinger;
pub mod macd;
pub mod macd_enhanced;
pub mod rsi;
pub mod sma_crossover;

// STUB — replace when merging with the real dual_macd strategy.
pub mod dual_macd;

use crate::market_data::Candle;

#[derive(Debug, Clone, PartialEq)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
    Short,
}

/// Single-timeframe strategy trait used by the legacy binaries.
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    fn required_history(&self) -> usize;
    /// `candles` is newest-first.
    fn signal(&self, candles: &[Candle]) -> Signal;
}

/// Dual-timeframe strategy trait used by the optimizer.
// STUB — replace when merging.
pub trait DualStrategy: Send + Sync {
    fn name(&self) -> &str;
    fn required_history(&self) -> usize;
    /// Both slices are newest-first.
    fn signal(&self, primary: &[Candle], secondary: &[Candle]) -> Signal;
}

pub fn from_config(cfg: &crate::config::StrategyConfig) -> Result<Box<dyn Strategy>> {
    let s: Box<dyn Strategy> = match cfg.name.as_str() {
        "sma_crossover" => Box::new(sma_crossover::SmaCrossover {
            short_period: cfg.short_period,
            long_period:  cfg.long_period,
        }),
        "rsi" => Box::new(rsi::Rsi {
            period:     cfg.rsi_period.unwrap_or(14),
            oversold:   cfg.rsi_oversold.unwrap_or(30.0),
            overbought: cfg.rsi_overbought.unwrap_or(70.0),
        }),
        "macd" => Box::new(macd::Macd {
            fast_period:   cfg.macd_fast.unwrap_or(12),
            slow_period:   cfg.macd_slow.unwrap_or(26),
            signal_period: cfg.macd_signal.unwrap_or(9),
        }),
        "macd_enhanced" => Box::new(macd_enhanced::MacdEnhanced::new(
            macd_enhanced::MacdEnhancedParams {
                fast_period:   cfg.macd_fast.unwrap_or(12),
                slow_period:   cfg.macd_slow.unwrap_or(26),
                signal_period: cfg.macd_signal.unwrap_or(9),
                ..Default::default()
            }
        )),
        "bollinger" => Box::new(bollinger::BollingerBands {
            period: cfg.bb_period.unwrap_or(20),
            k:      cfg.bb_k.unwrap_or(2.0),
        }),
        name => anyhow::bail!("Unbekannte Strategie: '{name}'. Verfügbar: sma_crossover, rsi, macd, bollinger"),
    };
    Ok(s)
}
pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    /// Minimum number of CANDLES required in each slice for signal computation.
    fn required_history(&self) -> usize;
    /// Compute a trading signal.
    /// primary: large-interval candles (e.g. 1d), newest-first
    /// secondary: small-interval candles (e.g. 1h), newest-first
    fn signal(&self, primary: &[Candle], secondary: &[Candle]) -> Signal;
}

pub mod dual_macd;
