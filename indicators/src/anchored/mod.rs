//! Event-driven ("anchored") indicators.
//!
//! Complement to the rolling indicators in the parent modules. Anchored
//! indicators only recompute when a `RollingDetector` fires an `AnchorEvent`
//! — see crate-level docs on the `anchored` module for the architecture.

use domain::Candle;

pub mod detectors;
pub mod evaluators;
pub mod ring;

pub use ring::SegmentState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionId {
    Asia,
    London,
    NewYork,
    Custom(u8),
}

/// A point-event emitted by a `RollingDetector`.
#[derive(Debug, Clone, Copy)]
pub enum AnchorEvent {
    PivotHigh {
        bar: u64,
        price: f64,
        volume: f64,
    },
    PivotLow {
        bar: u64,
        price: f64,
        volume: f64,
    },
    SessionOpen {
        bar: u64,
        session: SessionId,
    },
    SessionClose {
        bar: u64,
        session: SessionId,
    },
    RangeContracted {
        bar: u64,
        high: f64,
        low: f64,
        width_pct: f64,
    },
    Sweep {
        bar: u64,
        side: Side,
        extreme: f64,
    },
    FvgConfirmed {
        bar: u64,
        side: Side,
        gap: f64,
    },
}

impl AnchorEvent {
    pub fn bar(&self) -> u64 {
        match self {
            AnchorEvent::PivotHigh { bar, .. }
            | AnchorEvent::PivotLow { bar, .. }
            | AnchorEvent::SessionOpen { bar, .. }
            | AnchorEvent::SessionClose { bar, .. }
            | AnchorEvent::RangeContracted { bar, .. }
            | AnchorEvent::Sweep { bar, .. }
            | AnchorEvent::FvgConfirmed { bar, .. } => *bar,
        }
    }
}

/// A closed interval `[start_bar, end_bar]` on the candle stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    pub start_bar: u64,
    pub end_bar: u64,
}

impl Segment {
    pub fn len(&self) -> u64 {
        self.end_bar.saturating_sub(self.start_bar) + 1
    }
    pub fn is_empty(&self) -> bool {
        self.end_bar < self.start_bar
    }
}

/// Consumes every candle; may emit an event.
pub trait RollingDetector {
    fn on_candle(&mut self, c: &Candle, bar: u64) -> Option<AnchorEvent>;
}

/// Stateless computation over a closed `Segment`.
///
/// `candles` is the full candle buffer the caller has available; the evaluator
/// indexes it using bar-indices absolute to that buffer's own origin — the
/// caller is responsible for passing a buffer that contains `seg.start_bar`.
pub trait AnchoredEvaluator {
    type Output;
    fn evaluate(
        &self,
        candles: &[Candle],
        buffer_origin_bar: u64,
        seg: Segment,
    ) -> Option<Self::Output>;
}

/// Cheap per-bar check on an active signal.
pub trait Invalidator {
    fn still_valid(&self, c: &Candle, bar: u64) -> bool;
}

/// Helper: slice a candle buffer by absolute bar range.
pub(crate) fn slice_by_bars(
    candles: &[Candle],
    buffer_origin_bar: u64,
    seg: Segment,
) -> Option<&[Candle]> {
    if seg.is_empty() {
        return None;
    }
    let start = seg.start_bar.checked_sub(buffer_origin_bar)? as usize;
    let end = seg.end_bar.checked_sub(buffer_origin_bar)? as usize;
    if end >= candles.len() {
        return None;
    }
    Some(&candles[start..=end])
}
