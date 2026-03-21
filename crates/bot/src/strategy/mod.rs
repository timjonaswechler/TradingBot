pub mod bollinger;
pub mod macd;
pub mod macd_enhanced;
pub mod rsi;
pub mod sma_crossover;

use crate::market_data::Candle;
use anyhow::Result;

/// Rückgabewert einer Strategie-Berechnung.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
}

/// Trait den jede Handelsstrategie implementieren muss.
///
/// Der Bot kennt nur diesen Trait – die konkrete Strategie ist austauschbar:
///   let s: Box<dyn Strategy> = Box::new(SmaCrossover { ... });
///
/// Die einzige Stelle wo die eigentliche Kalkulation stattfindet ist `signal()`.
pub trait Strategy: Send + Sync {
    /// Name der Strategie (für Logging und Trade-History).
    fn name(&self) -> &str;

    /// Wie viele Candles werden mindestens für die Berechnung benötigt?
    fn required_history(&self) -> usize;

    /// Kernfunktion: Berechnet anhand der Candle-Historie ein Handelssignal.
    /// `candles` ist absteigend sortiert – candles[0] ist die neueste Candle.
    fn signal(&self, candles: &[Candle]) -> Signal;
}

/// Erzeugt eine Strategie anhand des Namens aus der Config.
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
