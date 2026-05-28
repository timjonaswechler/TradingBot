use crate::trend::ema::ema_series;
use crate::volatility::atr::atr_series;
use shared::Candle;

/// Result of Keltner Channels.
#[derive(Debug, Clone, PartialEq)]
pub struct KeltnerResult {
    pub upper: f64,
    pub middle: f64, // EMA of close
    pub lower: f64,
}

/// Keltner Channels.
///
/// `middle` = EMA(close, period), `upper/lower` = middle +/- multiplier * ATR(period).
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `period + 1` candles (ATR requirement).
pub fn keltner(candles: &[Candle], period: usize, multiplier: f64) -> Option<KeltnerResult> {
    if period == 0 || candles.len() < period + 1 {
        return None;
    }

    let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
    let ema_vals = ema_series(&closes, period)?;
    let atr_vals = atr_series(candles, period)?;

    // Both series start at the same point (index `period` of the original data)
    let middle = *ema_vals.last()?;
    let atr = *atr_vals.last()?;

    Some(KeltnerResult {
        upper: middle + multiplier * atr,
        middle,
        lower: middle - multiplier * atr,
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
        let c: Vec<Candle> = (1..=14)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert_eq!(keltner(&c, 14, 2.0), None);
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        assert!(keltner(&c, 14, 2.0).is_some());
    }

    #[test]
    fn upper_above_lower() {
        let c: Vec<Candle> = (1..=30)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        let r = keltner(&c, 14, 2.0).unwrap();
        assert!(r.upper > r.middle && r.middle > r.lower);
    }
}
