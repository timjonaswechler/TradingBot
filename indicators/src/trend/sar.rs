use shared::Candle;

/// Current trend side of the Parabolic SAR state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SarSide {
    Long,
    Short,
}

/// Result of the Parabolic SAR calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct SarResult {
    /// SAR value for the current (last) bar.
    pub value: f64,
    /// Trend side after processing the current bar.
    pub side: SarSide,
    /// Whether the current bar caused a stop-and-reverse flip.
    pub reversed: bool,
    /// Current extreme point after processing the current bar.
    pub ep: f64,
    /// Current acceleration factor after processing the current bar.
    pub af: f64,
}

/// Parabolic SAR (Stop and Reverse).
///
/// Returns the SAR state for the **current** (last) bar.
/// Input: candles in chronological order (oldest first).
/// Typical parameters: step = 0.02, max = 0.20.
/// Needs at least 2 candles.
pub fn sar(candles: &[Candle], step: f64, max: f64) -> Option<SarResult> {
    if candles.len() < 2 || step <= 0.0 || max <= 0.0 || step > max {
        return None;
    }

    // Determine initial trend from first two bars
    let mut is_long = candles[1].close > candles[0].close;
    let mut af = step;
    let mut ep: f64; // extreme point
    let mut sar: f64;
    let mut reversed = false;

    if is_long {
        sar = candles[0].low;
        ep = candles[1].high;
    } else {
        sar = candles[0].high;
        ep = candles[1].low;
    }

    for i in 2..candles.len() {
        let c = &candles[i];
        reversed = false;

        // Advance SAR
        sar = sar + af * (ep - sar);

        if is_long {
            // SAR must not be above prior two lows
            let min_low = candles[i - 1].low.min(candles[i - 2].low);
            sar = sar.min(min_low);

            if c.low < sar {
                // Reversal to short
                is_long = false;
                reversed = true;
                sar = ep; // SAR flips to the prior EP (highest high)
                ep = c.low;
                af = step;
            } else if c.high > ep {
                ep = c.high;
                af = (af + step).min(max);
            }
        } else {
            // SAR must not be below prior two highs
            let max_high = candles[i - 1].high.max(candles[i - 2].high);
            sar = sar.max(max_high);

            if c.high > sar {
                // Reversal to long
                is_long = true;
                reversed = true;
                sar = ep; // SAR flips to the prior EP (lowest low)
                ep = c.high;
                af = step;
            } else if c.low < ep {
                ep = c.low;
                af = (af + step).min(max);
            }
        }
    }

    Some(SarResult {
        value: sar,
        side: if is_long {
            SarSide::Long
        } else {
            SarSide::Short
        },
        reversed,
        ep,
        af,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            timeframe: "1d".parse().unwrap(),
        }
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
        assert!(
            s.value < last_close,
            "SAR {} should be below close {last_close} in uptrend",
            s.value
        );
        assert_eq!(s.side, SarSide::Long);
        assert!(!s.reversed);
        assert!(s.ep > s.value);
        assert!(s.af > 0.0);
    }

    #[test]
    fn flip_sets_reversed_and_updates_side() {
        let candles = vec![
            candle(10.5, 9.5, 10.0),
            candle(11.5, 10.5, 11.0),
            candle(12.5, 11.5, 12.0),
            candle(9.0, 7.5, 8.0),
        ];
        let s = sar(&candles, 0.02, 0.2).unwrap();
        assert_eq!(s.side, SarSide::Short);
        assert!(s.reversed);
        assert_eq!(s.af, 0.02);
        assert_eq!(s.ep, 7.5);
    }
}
