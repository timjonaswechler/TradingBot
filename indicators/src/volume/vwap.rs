use shared::Candle;

/// Volume Weighted Average Price over all supplied candles.
///
/// Typical usage: pass all candles of the current trading session.
/// Needs at least 1 candle with non-zero volume.
pub fn vwap(candles: &[Candle]) -> Option<f64> {
    if candles.is_empty() {
        return None;
    }
    let (cum_tp_vol, cum_vol) = candles.iter().fold((0.0f64, 0.0f64), |(tpv, vol), c| {
        let tp = (c.high + c.low + c.close) / 3.0;
        (tpv + tp * c.volume, vol + c.volume)
    });
    if cum_vol < 1e-12 {
        return None;
    }
    Some(cum_tp_vol / cum_vol)
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
            timeframe: "1d".into(),
        }
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(vwap(&[]), None);
    }

    #[test]
    fn zero_volume_returns_none() {
        assert_eq!(vwap(&[candle(2.0, 1.0, 1.5, 0.0)]), None);
    }

    #[test]
    fn single_candle_equals_typical_price() {
        // TP = (2+1+1.5)/3 = 1.5
        let r = vwap(&[candle(2.0, 1.0, 1.5, 100.0)]).unwrap();
        assert!((r - 1.5).abs() < 1e-10);
    }

    #[test]
    fn higher_volume_bar_weights_more() {
        // Bar1: TP=1.0, vol=1  Bar2: TP=3.0, vol=3
        // VWAP = (1*1 + 3*3)/(1+3) = 10/4 = 2.5
        let c = vec![candle(1.5, 0.5, 1.0, 1.0), candle(3.5, 2.5, 3.0, 3.0)];
        let r = vwap(&c).unwrap();
        assert!((r - 2.5).abs() < 1e-10);
    }
}
