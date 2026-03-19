use super::{Signal, Strategy};
use crate::market_data::Candle;

/// Simple Moving Average Crossover Strategie.
///
/// Signal-Logik:
///   BUY  – kurzfristiger SMA kreuzt langfristigen SMA von unten nach oben
///   SELL – kurzfristiger SMA kreuzt langfristigen SMA von oben nach unten
///   HOLD – kein Crossover
pub struct SmaCrossover {
    pub short_period: usize, // z.B. 10 Tage
    pub long_period:  usize, // z.B. 50 Tage
}

impl Strategy for SmaCrossover {
    fn name(&self) -> &str {
        "SMA Crossover"
    }

    fn required_history(&self) -> usize {
        // +1 damit wir gestern vs. heute vergleichen können (Crossover-Erkennung)
        self.long_period + 1
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        if candles.len() < self.required_history() {
            return Signal::Hold;
        }

        // candles[0] = heute, candles[1] = gestern, ...
        let today_short = sma(&candles[..self.short_period]);
        let today_long  = sma(&candles[..self.long_period]);

        let prev_short = sma(&candles[1..=self.short_period]);
        let prev_long  = sma(&candles[1..=self.long_period]);

        if prev_short <= prev_long && today_short > today_long {
            Signal::Buy  // Crossover nach oben → bullish
        } else if prev_short >= prev_long && today_short < today_long {
            Signal::Sell // Crossover nach unten → bearish
        } else {
            Signal::Hold
        }
    }
}

fn sma(candles: &[Candle]) -> f64 {
    if candles.is_empty() {
        return 0.0;
    }
    let sum: i64 = candles.iter().map(|c| c.close).sum();
    sum as f64 / candles.len() as f64
}
