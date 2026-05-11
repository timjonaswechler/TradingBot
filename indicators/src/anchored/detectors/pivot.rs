//! Pivot-high / pivot-low detector matching TradingView `ta.pivothigh/pivotlow(src, left, right)`.
//!
//! A bar at position P is a pivot high iff `high[P]` is strictly greater than
//! every `high` in `[P-left, P-1]` and every `high` in `[P+1, P+right]`.
//! Confirmation is delayed by `right` bars — the event fires when candle `P+right`
//! is consumed, but the event's `bar` field is `P` (the actual pivot).

use shared::Candle;
use std::collections::VecDeque;

use super::super::{AnchorEvent, RollingDetector};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PivotKind {
    High,
    Low,
    Both,
}

#[derive(Debug)]
pub struct PivotDetector {
    left: usize,
    right: usize,
    kind: PivotKind,
    /// Ring of the last `left + right + 1` candles with their absolute bar idx.
    buf: VecDeque<(u64, Candle)>,
}

impl PivotDetector {
    pub fn new(left: usize, right: usize, kind: PivotKind) -> Self {
        assert!(left >= 1 && right >= 1, "left/right must be >= 1");
        Self {
            left,
            right,
            kind,
            buf: VecDeque::with_capacity(left + right + 1),
        }
    }

    fn window(&self) -> usize {
        self.left + self.right + 1
    }
}

impl RollingDetector for PivotDetector {
    fn on_candle(&mut self, c: &Candle, bar: u64) -> Option<AnchorEvent> {
        self.buf.push_back((bar, c.clone()));
        if self.buf.len() > self.window() {
            self.buf.pop_front();
        }
        // Not enough history yet — need the full window before we can judge the middle.
        if self.buf.len() < self.window() {
            return None;
        }

        let mid = self.left; // index in buf of the candidate pivot
        let (cand_bar, cand) = &self.buf[mid];
        let cand_bar = *cand_bar;

        let mut is_high = matches!(self.kind, PivotKind::High | PivotKind::Both);
        let mut is_low = matches!(self.kind, PivotKind::Low | PivotKind::Both);

        for (i, (_, other)) in self.buf.iter().enumerate() {
            if i == mid {
                continue;
            }
            if is_high && other.high >= cand.high {
                is_high = false;
            }
            if is_low && other.low <= cand.low {
                is_low = false;
            }
            if !is_high && !is_low {
                break;
            }
        }

        // Tie-break: if both match (flat bar), prefer High — unlikely in practice.
        if is_high {
            return Some(AnchorEvent::PivotHigh {
                bar: cand_bar,
                price: cand.high,
                volume: cand.volume,
            });
        }
        if is_low {
            return Some(AnchorEvent::PivotLow {
                bar: cand_bar,
                price: cand.low,
                volume: cand.volume,
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: (h + l) / 2.0,
            volume: 1.0,
            timeframe: "1m".into(),
        }
    }

    #[test]
    fn detects_pivot_high_with_delay() {
        // Highs: 1, 2, 3, 5, 4, 3, 2 — pivot at bar index 3 (value 5), detected when bar 5 arrives (right=2).
        let highs = [1.0, 2.0, 3.0, 5.0, 4.0, 3.0, 2.0];
        let mut det = PivotDetector::new(2, 2, PivotKind::High);
        let mut hits = vec![];
        for (i, &h) in highs.iter().enumerate() {
            if let Some(ev) = det.on_candle(&candle(h, h - 0.5), i as u64) {
                hits.push(ev);
            }
        }
        assert_eq!(hits.len(), 1);
        match hits[0] {
            AnchorEvent::PivotHigh { bar, price, .. } => {
                assert_eq!(bar, 3);
                assert!((price - 5.0).abs() < 1e-9);
            }
            _ => panic!("expected pivot high"),
        }
    }

    #[test]
    fn detects_pivot_low() {
        // Lows (as highs for simplicity): dip at idx 3.
        let lows = [5.0, 4.0, 3.0, 1.0, 2.0, 3.0, 4.0];
        let mut det = PivotDetector::new(2, 2, PivotKind::Low);
        let mut hits = vec![];
        for (i, &l) in lows.iter().enumerate() {
            // Make high unique/monotone to avoid confounding.
            let c = Candle {
                timestamp: 0,
                symbol: "T".into(),
                open: l,
                high: l + 10.0,
                low: l,
                close: l,
                volume: 1.0,
                timeframe: "1m".into(),
            };
            if let Some(ev) = det.on_candle(&c, i as u64) {
                hits.push(ev);
            }
        }
        assert_eq!(hits.len(), 1);
        match hits[0] {
            AnchorEvent::PivotLow { bar, price, .. } => {
                assert_eq!(bar, 3);
                assert!((price - 1.0).abs() < 1e-9);
            }
            _ => panic!("expected pivot low"),
        }
    }

    #[test]
    fn no_pivot_on_monotone_series() {
        let mut det = PivotDetector::new(2, 2, PivotKind::Both);
        for i in 0..10u64 {
            let ev = det.on_candle(&candle(i as f64, i as f64 - 0.5), i);
            assert!(ev.is_none(), "unexpected pivot {:?}", ev);
        }
    }

    #[test]
    fn equal_highs_do_not_form_pivot() {
        // 1,2,3,3,3,2,1 — flat top, no strict pivot
        let highs = [1.0, 2.0, 3.0, 3.0, 3.0, 2.0, 1.0];
        let mut det = PivotDetector::new(2, 2, PivotKind::High);
        let mut hits = 0;
        for (i, &h) in highs.iter().enumerate() {
            if det.on_candle(&candle(h, h - 0.5), i as u64).is_some() {
                hits += 1;
            }
        }
        assert_eq!(hits, 0);
    }
}
