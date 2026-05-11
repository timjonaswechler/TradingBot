use shared::Candle;

/// On-Balance Volume — cumulative volume with direction based on close-vs-prev-close.
///
/// Returns the OBV value at the last bar.
/// Needs at least 2 candles.
pub fn obv(candles: &[Candle]) -> Option<f64> {
    if candles.len() < 2 {
        return None;
    }
    let mut val = 0.0f64;
    for i in 1..candles.len() {
        let cur = &candles[i];
        let prev = &candles[i - 1];
        if cur.close > prev.close {
            val += cur.volume;
        } else if cur.close < prev.close {
            val -= cur.volume;
        }
        // unchanged close → OBV unchanged
    }
    Some(val)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(c: f64, v: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: c,
            high: c + 0.5,
            low: c - 0.5,
            close: c,
            volume: v,
            timeframe: "1d".into(),
        }
    }

    #[test]
    fn insufficient_data() {
        assert_eq!(obv(&[candle(1.0, 100.0)]), None);
    }

    #[test]
    fn all_up_days() {
        let c = vec![candle(1.0, 100.0), candle(2.0, 200.0), candle(3.0, 300.0)];
        assert_eq!(obv(&c), Some(500.0));
    }

    #[test]
    fn all_down_days() {
        let c = vec![candle(3.0, 100.0), candle(2.0, 200.0), candle(1.0, 300.0)];
        assert_eq!(obv(&c), Some(-500.0));
    }

    #[test]
    fn flat_day_no_change() {
        let c = vec![candle(1.0, 100.0), candle(1.0, 999.0), candle(2.0, 50.0)];
        // flat → no change, then up → +50
        assert_eq!(obv(&c), Some(50.0));
    }
}
