use shared::Candle;

/// Parabolic SAR (Stop and Reverse).
///
/// Returns the SAR value for the **current** (last) bar.
/// Input: candles in chronological order (oldest first).
/// Typical parameters: step = 0.02, max = 0.20.
/// Needs at least 2 candles.
pub fn sar(candles: &[Candle], step: f64, max: f64) -> Option<f64> {
    if candles.len() < 2 || step <= 0.0 || max <= 0.0 || step > max {
        return None;
    }

    // Determine initial trend from first two bars
    let mut is_long = candles[1].close > candles[0].close;
    let mut af = step;
    let mut ep: f64; // extreme point
    let mut sar: f64;

    if is_long {
        sar = candles[0].low;
        ep = candles[1].high;
    } else {
        sar = candles[0].high;
        ep = candles[1].low;
    }

    for i in 2..candles.len() {
        let c = &candles[i];

        // Advance SAR
        sar = sar + af * (ep - sar);

        if is_long {
            // SAR must not be above prior two lows
            let min_low = candles[i - 1].low.min(candles[i - 2].low);
            sar = sar.min(min_low);

            if c.low < sar {
                // Reversal to short
                is_long = false;
                sar = ep; // SAR flips to the prior EP (highest high)
                ep = c.low;
                af = step;
            } else {
                if c.high > ep {
                    ep = c.high;
                    af = (af + step).min(max);
                }
            }
        } else {
            // SAR must not be below prior two highs
            let max_high = candles[i - 1].high.max(candles[i - 2].high);
            sar = sar.max(max_high);

            if c.high > sar {
                // Reversal to long
                is_long = true;
                sar = ep; // SAR flips to the prior EP (lowest low)
                ep = c.high;
                af = step;
            } else {
                if c.low < ep {
                    ep = c.low;
                    af = (af + step).min(max);
                }
            }
        }
    }

    Some(sar)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64) -> Candle {
        Candle { timestamp: 0, symbol: "T".into(), open: l, high: h, low: l, close: c, volume: 1.0, timeframe: "1d".into() }
    }

    #[test]
    fn insufficient_data() {
        assert_eq!(sar(&[candle(2.0, 1.0, 1.5)], 0.02, 0.2), None);
    }

    #[test]
    fn computes_without_panic() {
        let candles: Vec<Candle> = (1..=10)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert!(sar(&candles, 0.02, 0.2).is_some());
    }

    #[test]
    fn sar_below_price_in_uptrend() {
        // Rising candles — SAR should be below the last close
        let candles: Vec<Candle> = (1..=15)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let s = sar(&candles, 0.02, 0.2).unwrap();
        let last_close = 15.0f64;
        assert!(s < last_close, "SAR {s} should be below close {last_close} in uptrend");
    }
}
