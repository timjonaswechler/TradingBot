use super::{Signal, Strategy};
use crate::market_data::Candle;

/// MACD (Moving Average Convergence Divergence) Strategie.
///
/// MACD-Linie    = EMA(fast) − EMA(slow)
/// Signal-Linie  = EMA(macd_line, signal_period)
///
/// Signal-Logik:
///   BUY  – MACD-Linie kreuzt Signal-Linie von unten nach oben
///   SELL – MACD-Linie kreuzt Signal-Linie von oben nach unten
///   HOLD – kein Crossover
pub struct Macd {
    pub fast_period:   usize, // typisch 12
    pub slow_period:   usize, // typisch 26
    pub signal_period: usize, // typisch 9
}

impl Default for Macd {
    fn default() -> Self {
        Self { fast_period: 12, slow_period: 26, signal_period: 9 }
    }
}

impl Strategy for Macd {
    fn name(&self) -> &str {
        "MACD"
    }

    fn required_history(&self) -> usize {
        // Brauchen genug für slow EMA + signal EMA + 1 für Crossover-Vergleich
        self.slow_period + self.signal_period + 1
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        if candles.len() < self.required_history() {
            return Signal::Hold;
        }

        // Berechne MACD-Linie für die letzten signal_period+1 Punkte
        let needed = self.signal_period + 1;
        let mut macd_series: Vec<f64> = Vec::with_capacity(needed);

        for offset in 0..needed {
            let slice = &candles[offset..];
            let fast = ema(slice, self.fast_period);
            let slow = ema(slice, self.slow_period);
            macd_series.push(fast - slow);
        }
        // macd_series[0] = heute, [1] = gestern, ...

        // Signal-Linie = EMA der MACD-Linie
        let today_signal = ema_of_values(&macd_series[..self.signal_period]);
        let prev_signal  = ema_of_values(&macd_series[1..=self.signal_period]);

        let today_macd = macd_series[0];
        let prev_macd  = macd_series[1];

        if prev_macd <= prev_signal && today_macd > today_signal {
            Signal::Buy
        } else if prev_macd >= prev_signal && today_macd < today_signal {
            Signal::Sell
        } else {
            Signal::Hold
        }
    }
}

/// Exponential Moving Average über `period` Candles (neueste zuerst).
fn ema(candles: &[Candle], period: usize) -> f64 {
    if candles.len() < period {
        return 0.0;
    }
    // Initialisierung mit SMA der ältesten `period` Werte
    let prices: Vec<f64> = candles.iter().map(|c| c.close as f64).collect();
    // Wir rechnen von alt → neu, also umkehren
    let rev: Vec<f64> = prices[..period + candles.len().saturating_sub(period).min(candles.len() - period)]
        .iter().rev().cloned().collect();
    ema_of_values_from_oldest(&rev, period)
}

fn ema_of_values(values: &[f64]) -> f64 {
    // values[0] = neueste, values[n-1] = älteste → umkehren für Berechnung
    let rev: Vec<f64> = values.iter().rev().cloned().collect();
    ema_of_values_from_oldest(&rev, values.len())
}

/// Berechnet EMA aus aufsteigend (älteste zuerst) sortierten Werten.
fn ema_of_values_from_oldest(values: &[f64], period: usize) -> f64 {
    if values.len() < period {
        return 0.0;
    }
    let k = 2.0 / (period as f64 + 1.0);
    // Startwert = SMA der ersten `period` Werte
    let mut ema_val: f64 = values[..period].iter().sum::<f64>() / period as f64;
    for &v in &values[period..] {
        ema_val = v * k + ema_val * (1.0 - k);
    }
    ema_val
}
