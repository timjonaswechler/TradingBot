//! 3+-touch trendline fitting over a bounded buffer of pivots.
//!
//! Mirrors the Pine v6 "3+ Touch Trend Lines" strategy:
//!   - For each pair (i, j) of pivots in the buffer, form a line.
//!   - Reject if any intermediate pivot pierces the line beyond `tolerance`.
//!   - Count touches (pivots within `|y - lineY| / y <= tolerance`, incl. i, j).
//!   - Require `touches >= min_touches`.
//!   - Reject if any close after the second anchor has broken the line.
//!   - Score by touches; ties broken by steepness (side-dependent).
//!
//! Cost: `O(K²)` per evaluation, `K` = pivot-buffer capacity (typ. 6).
//! The evaluation is event-driven (on each new pivot) — not per bar.

use shared::Candle;

use super::super::{AnchorEvent, Invalidator, SegmentState};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrendlineSide {
    Resistance,
    Support,
}

/// A fitted trendline, parametrised in bar-space: `y(bar) = slope * bar + intercept`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrendLine {
    pub side: TrendlineSide,
    pub slope: f64,
    pub intercept: f64,
    pub anchor_start_bar: u64,
    pub anchor_end_bar: u64,
    pub touches: u32,
}

impl TrendLine {
    pub fn y_at(&self, bar: u64) -> f64 {
        self.slope * bar as f64 + self.intercept
    }
}

#[derive(Debug, Clone)]
pub struct TrendlineEvaluator {
    pub side: TrendlineSide,
    /// Relative tolerance: `|y - lineY| / y <= tolerance` to count as a touch.
    pub tolerance: f64,
    pub min_touches: u32,
    /// Maximum lines returned per call, sorted best-first.
    pub max_lines: usize,
}

impl TrendlineEvaluator {
    pub fn new(side: TrendlineSide, tolerance: f64, min_touches: u32, max_lines: usize) -> Self {
        assert!(tolerance > 0.0 && tolerance < 0.5);
        assert!(min_touches >= 3);
        assert!(max_lines >= 1);
        Self {
            side,
            tolerance,
            min_touches,
            max_lines,
        }
    }

    /// Fit trendlines from the current pivot buffer. `candles` is the per-symbol
    /// candle buffer with origin `buffer_origin_bar` used for the break-check.
    pub fn fit(
        &self,
        pivots: &SegmentState<AnchorEvent>,
        candles: &[Candle],
        buffer_origin_bar: u64,
        current_bar: u64,
    ) -> Vec<TrendLine> {
        // Extract pivots of the side we care about.
        let want_high = self.side == TrendlineSide::Resistance;
        let pts: Vec<(u64, f64)> = pivots
            .iter()
            .filter_map(|ev| match (want_high, ev) {
                (true, AnchorEvent::PivotHigh { bar, price, .. }) => Some((*bar, *price)),
                (false, AnchorEvent::PivotLow { bar, price, .. }) => Some((*bar, *price)),
                _ => None,
            })
            .collect();
        if pts.len() < self.min_touches as usize {
            return vec![];
        }

        let n = pts.len();
        let mut candidates: Vec<TrendLine> = Vec::new();

        for i in 0..n - 1 {
            for j in (i + 1)..n {
                let (x1, y1) = (pts[i].0 as f64, pts[i].1);
                let (x2, y2) = (pts[j].0 as f64, pts[j].1);
                if (x2 - x1).abs() < 1e-9 {
                    continue;
                }
                let slope = (y2 - y1) / (x2 - x1);
                let intercept = y1 - slope * x1;

                // 1) Intermediate pivots (strictly between i and j) must not pierce the line.
                let mut ok = true;
                for k in (i + 1)..j {
                    let (xk, yk) = (pts[k].0 as f64, pts[k].1);
                    let line_y = slope * xk + intercept;
                    let tol_abs = yk.abs() * self.tolerance;
                    let pierces = match self.side {
                        TrendlineSide::Resistance => yk > line_y + tol_abs,
                        TrendlineSide::Support => yk < line_y - tol_abs,
                    };
                    if pierces {
                        ok = false;
                        break;
                    }
                }
                if !ok {
                    continue;
                }

                // 2) Count touches across all buffer pivots.
                let mut touches: u32 = 2;
                for (k, &(xk, yk)) in pts.iter().enumerate() {
                    if k == i || k == j {
                        continue;
                    }
                    let line_y = slope * xk as f64 + intercept;
                    if yk.abs() < 1e-12 {
                        continue;
                    }
                    if ((yk - line_y).abs() / yk.abs()) <= self.tolerance {
                        touches += 1;
                    }
                }
                if touches < self.min_touches {
                    continue;
                }

                // 3) Break-check: no close after the second anchor has crossed the line.
                if !self.is_unbroken(
                    slope,
                    intercept,
                    pts[j].0,
                    candles,
                    buffer_origin_bar,
                    current_bar,
                ) {
                    continue;
                }

                candidates.push(TrendLine {
                    side: self.side,
                    slope,
                    intercept,
                    anchor_start_bar: pts[i].0,
                    anchor_end_bar: pts[j].0,
                    touches,
                });
            }
        }

        // Rank: more touches first; tie-break by slope (steepest for resistance,
        // most-negative for support — i.e. the "tightest" trendline).
        candidates.sort_by(|a, b| {
            b.touches.cmp(&a.touches).then_with(|| match self.side {
                TrendlineSide::Resistance => b
                    .slope
                    .partial_cmp(&a.slope)
                    .unwrap_or(std::cmp::Ordering::Equal),
                TrendlineSide::Support => a
                    .slope
                    .partial_cmp(&b.slope)
                    .unwrap_or(std::cmp::Ordering::Equal),
            })
        });
        candidates.truncate(self.max_lines);
        candidates
    }

    fn is_unbroken(
        &self,
        slope: f64,
        intercept: f64,
        after_bar: u64,
        candles: &[Candle],
        buffer_origin_bar: u64,
        current_bar: u64,
    ) -> bool {
        if current_bar <= after_bar {
            return true;
        }
        let first = after_bar + 1;
        let Some(first_idx) = first.checked_sub(buffer_origin_bar) else {
            return true;
        };
        let first_idx = first_idx as usize;
        let last_idx = match current_bar.checked_sub(buffer_origin_bar) {
            Some(v) => (v as usize).min(candles.len().saturating_sub(1)),
            None => return true,
        };
        if first_idx > last_idx || first_idx >= candles.len() {
            return true;
        }

        for idx in first_idx..=last_idx {
            let bar = buffer_origin_bar + idx as u64;
            let line_y = slope * bar as f64 + intercept;
            let close = candles[idx].close;
            match self.side {
                TrendlineSide::Resistance => {
                    if close > line_y {
                        return false;
                    }
                }
                TrendlineSide::Support => {
                    if close < line_y {
                        return false;
                    }
                }
            }
        }
        true
    }
}

/// Per-bar invalidator for an already-emitted trendline: checks if the current
/// close has broken through. Cheap: O(1).
#[derive(Debug, Clone, Copy)]
pub struct TrendlineInvalidator(pub TrendLine);

impl Invalidator for TrendlineInvalidator {
    fn still_valid(&self, c: &Candle, bar: u64) -> bool {
        let y = self.0.y_at(bar);
        match self.0.side {
            TrendlineSide::Resistance => c.close <= y,
            TrendlineSide::Support => c.close >= y,
        }
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
            timeframe: "1m".parse().unwrap(),
        }
    }

    fn push_ph(buf: &mut SegmentState<AnchorEvent>, bar: u64, price: f64) {
        buf.push(AnchorEvent::PivotHigh {
            bar,
            price,
            volume: 1.0,
        });
    }
    fn push_pl(buf: &mut SegmentState<AnchorEvent>, bar: u64, price: f64) {
        buf.push(AnchorEvent::PivotLow {
            bar,
            price,
            volume: 1.0,
        });
    }

    #[test]
    fn resistance_three_touches_flat() {
        // Three pivot highs at the same price 100 on bars 10, 20, 30.
        let mut buf = SegmentState::new(6);
        push_ph(&mut buf, 10, 100.0);
        push_ph(&mut buf, 20, 100.0);
        push_ph(&mut buf, 30, 100.0);
        // Candles from bar 0..=40 with close well below 100 — line is unbroken.
        let candles: Vec<Candle> = (0..=40).map(|_| c(90.0)).collect();
        let ev = TrendlineEvaluator::new(TrendlineSide::Resistance, 0.01, 3, 5);
        let lines = ev.fit(&buf, &candles, 0, 40);
        assert!(!lines.is_empty());
        assert_eq!(lines[0].touches, 3);
        assert!(lines[0].slope.abs() < 1e-9);
    }

    #[test]
    fn broken_line_is_rejected() {
        let mut buf = SegmentState::new(6);
        push_ph(&mut buf, 10, 100.0);
        push_ph(&mut buf, 20, 100.0);
        push_ph(&mut buf, 30, 100.0);
        // After bar 30, a close at 110 breaks the 100-resistance.
        let mut candles: Vec<Candle> = (0..=40).map(|_| c(90.0)).collect();
        candles[35] = c(110.0);
        let ev = TrendlineEvaluator::new(TrendlineSide::Resistance, 0.01, 3, 5);
        let lines = ev.fit(&buf, &candles, 0, 40);
        assert!(lines.is_empty(), "line should be broken, got {:?}", lines);
    }

    #[test]
    fn support_ascending_line() {
        // Three pivot lows on a rising line y = 1 * bar + 50
        let mut buf = SegmentState::new(6);
        push_pl(&mut buf, 10, 60.0);
        push_pl(&mut buf, 20, 70.0);
        push_pl(&mut buf, 30, 80.0);
        // Closes comfortably above the line → unbroken (close >= line_y).
        let candles: Vec<Candle> = (0..=40).map(|i| c(100.0 + i as f64)).collect();
        let ev = TrendlineEvaluator::new(TrendlineSide::Support, 0.02, 3, 5);
        let lines = ev.fit(&buf, &candles, 0, 40);
        assert!(!lines.is_empty());
        assert!((lines[0].slope - 1.0).abs() < 1e-9);
    }

    #[test]
    fn invalidator_detects_break() {
        let line = TrendLine {
            side: TrendlineSide::Resistance,
            slope: 0.0,
            intercept: 100.0,
            anchor_start_bar: 10,
            anchor_end_bar: 30,
            touches: 3,
        };
        let inv = TrendlineInvalidator(line);
        assert!(inv.still_valid(&c(99.0), 50));
        assert!(!inv.still_valid(&c(101.0), 50));
    }
}
