use domain::Candle;

/// Money Flow Index — a volume-weighted RSI (0-100).
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `period + 1` candles.
pub fn mfi(candles: &[Candle], period: usize) -> Option<f64> {
    if period == 0 || candles.len() <= period {
        return None;
    }

    let slice = &candles[candles.len() - period - 1..];

    let tp: Vec<f64> = slice
        .iter()
        .map(|c| (c.high + c.low + c.close) / 3.0)
        .collect();

    let mut pos_flow = 0.0f64;
    let mut neg_flow = 0.0f64;

    for i in 1..=period {
        let raw_flow = tp[i] * slice[i].volume;
        if tp[i] > tp[i - 1] {
            pos_flow += raw_flow;
        } else {
            neg_flow += raw_flow;
        }
    }

    if neg_flow < 1e-12 {
        return Some(100.0);
    }
    let mfr = pos_flow / neg_flow;
    Some(100.0 - 100.0 / (1.0 + mfr))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64, v: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: c,
            volume: v,
            timeframe: "1d".parse().unwrap(),
        }
    }

    #[test]
    fn insufficient_data() {
        let c: Vec<Candle> = (1..=14)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64, 100.0))
            .collect();
        assert_eq!(mfi(&c, 14), None); // needs 15
    }

    #[test]
    fn all_up_days_returns_100() {
        let c: Vec<Candle> = (1..=15)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64, 100.0))
            .collect();
        assert_eq!(mfi(&c, 14), Some(100.0));
    }

    #[test]
    fn value_in_range() {
        let c: Vec<Candle> = [
            1.0, 2.0, 1.5, 3.0, 2.5, 4.0, 3.5, 5.0, 4.5, 6.0, 5.5, 7.0, 6.5, 8.0, 7.5,
        ]
        .iter()
        .enumerate()
        .map(|(i, &p)| candle(p + 0.5, p - 0.5, p, (i + 1) as f64 * 100.0))
        .collect();
        let r = mfi(&c, 14).unwrap();
        assert!(r >= 0.0 && r <= 100.0);
    }
}
