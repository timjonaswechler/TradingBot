//! `AnchoredEvaluator` wrapper around the free `slope()` function.

use shared::Candle;

use super::super::{slice_by_bars, AnchoredEvaluator, Segment};
use crate::slope::slope;

#[derive(Debug, Default, Clone, Copy)]
pub struct SlopeSegEvaluator;

impl AnchoredEvaluator for SlopeSegEvaluator {
    type Output = f64;

    fn evaluate(&self, candles: &[Candle], buffer_origin_bar: u64, seg: Segment) -> Option<f64> {
        let slice = slice_by_bars(candles, buffer_origin_bar, seg)?;
        if slice.len() < 2 {
            return None;
        }
        let closes: Vec<f64> = slice.iter().map(|c| c.close).collect();
        slope(&closes, closes.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::Candle;

    fn c(close: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 0.0,
            timeframe: "1m".into(),
        }
    }

    #[test]
    fn positive_slope_on_uptrend_segment() {
        let candles: Vec<Candle> = (0..10).map(|i| c(i as f64)).collect();
        let ev = SlopeSegEvaluator;
        let s = ev
            .evaluate(
                &candles,
                0,
                Segment {
                    start_bar: 2,
                    end_bar: 7,
                },
            )
            .unwrap();
        assert!((s - 1.0).abs() < 1e-9);
    }

    #[test]
    fn handles_non_zero_buffer_origin() {
        let candles: Vec<Candle> = (0..5).map(|i| c(i as f64 * 2.0)).collect();
        // origin=100 means candles[0].bar = 100, candles[4].bar = 104.
        let ev = SlopeSegEvaluator;
        let s = ev
            .evaluate(
                &candles,
                100,
                Segment {
                    start_bar: 101,
                    end_bar: 104,
                },
            )
            .unwrap();
        assert!((s - 2.0).abs() < 1e-9);
    }

    #[test]
    fn out_of_range_returns_none() {
        let candles: Vec<Candle> = (0..3).map(|i| c(i as f64)).collect();
        let ev = SlopeSegEvaluator;
        assert!(ev
            .evaluate(
                &candles,
                0,
                Segment {
                    start_bar: 0,
                    end_bar: 10
                }
            )
            .is_none());
    }
}
