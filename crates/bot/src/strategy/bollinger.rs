use super::{Signal, Strategy};
use crate::market_data::Candle;

/// Bollinger Bands Strategie.
///
/// Oberes Band  = SMA + k * Standardabweichung
/// Unteres Band = SMA − k * Standardabweichung
///
/// Signal-Logik:
///   BUY  – Kurs war unter unterem Band und kehrt darüber zurück (Mean Reversion)
///   SELL – Kurs war über oberem Band und fällt darunter zurück
///   HOLD – Kurs innerhalb der Bänder
pub struct BollingerBands {
    pub period: usize, // typisch 20
    pub k:      f64,   // typisch 2.0 (Anzahl Standardabweichungen)
}

impl Default for BollingerBands {
    fn default() -> Self {
        Self { period: 20, k: 2.0 }
    }
}

impl Strategy for BollingerBands {
    fn name(&self) -> &str {
        "Bollinger Bands"
    }

    fn required_history(&self) -> usize {
        self.period + 1
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        if candles.len() < self.required_history() {
            return Signal::Hold;
        }

        let today_price = candles[0].close as f64;
        let prev_price  = candles[1].close as f64;

        let (today_upper, today_lower) = bands(&candles[..self.period], self.k);
        let (prev_upper,  prev_lower)  = bands(&candles[1..=self.period], self.k);

        // Kurs war unter unterem Band, jetzt darüber → BUY
        if prev_price <= prev_lower && today_price > today_lower {
            return Signal::Buy;
        }
        // Kurs war über oberem Band, jetzt darunter → SELL
        if prev_price >= prev_upper && today_price < today_upper {
            return Signal::Sell;
        }

        Signal::Hold
    }
}

/// Gibt (oberes Band, unteres Band) zurück für ein Candle-Slice (neueste zuerst).
fn bands(candles: &[Candle], k: f64) -> (f64, f64) {
    let n = candles.len() as f64;
    let mean: f64 = candles.iter().map(|c| c.close as f64).sum::<f64>() / n;
    let variance: f64 = candles.iter()
        .map(|c| {
            let diff = c.close as f64 - mean;
            diff * diff
        })
        .sum::<f64>() / n;
    let std = variance.sqrt();
    (mean + k * std, mean - k * std)
}
