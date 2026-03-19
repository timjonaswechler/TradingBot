use super::{Signal, Strategy};
use crate::market_data::Candle;

/// Relative Strength Index (RSI) Strategie.
///
/// Signal-Logik:
///   BUY  – RSI fällt unter `oversold` (default 30) und dreht nach oben
///   SELL – RSI steigt über `overbought` (default 70) und dreht nach unten
///   HOLD – RSI im neutralen Bereich
pub struct Rsi {
    pub period:     usize, // typisch 14
    pub oversold:   f64,   // typisch 30.0
    pub overbought: f64,   // typisch 70.0
}

impl Default for Rsi {
    fn default() -> Self {
        Self { period: 14, oversold: 30.0, overbought: 70.0 }
    }
}

impl Strategy for Rsi {
    fn name(&self) -> &str {
        "RSI"
    }

    fn required_history(&self) -> usize {
        // +1 für RSI-Delta (heute vs. gestern)
        self.period + 1
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        if candles.len() < self.required_history() {
            return Signal::Hold;
        }

        let today_rsi = rsi(&candles[..self.period]);
        let prev_rsi  = rsi(&candles[1..=self.period]);

        // Oversold-Zone verlassen → BUY
        if prev_rsi <= self.oversold && today_rsi > self.oversold {
            return Signal::Buy;
        }
        // Overbought-Zone verlassen → SELL
        if prev_rsi >= self.overbought && today_rsi < self.overbought {
            return Signal::Sell;
        }

        Signal::Hold
    }
}

/// Berechnet den RSI für ein Candle-Slice (neueste zuerst).
fn rsi(candles: &[Candle]) -> f64 {
    if candles.len() < 2 {
        return 50.0;
    }

    let mut gains = 0.0f64;
    let mut losses = 0.0f64;

    // candles[0] = heute, candles[1] = gestern, ...
    // Wir berechnen Änderungen von alt → neu, also candles[i+1] → candles[i]
    for i in 0..candles.len() - 1 {
        let change = (candles[i].close - candles[i + 1].close) as f64;
        if change > 0.0 {
            gains += change;
        } else {
            losses += change.abs();
        }
    }

    let n = (candles.len() - 1) as f64;
    let avg_gain = gains / n;
    let avg_loss = losses / n;

    if avg_loss == 0.0 {
        return 100.0;
    }

    let rs = avg_gain / avg_loss;
    100.0 - (100.0 / (1.0 + rs))
}
