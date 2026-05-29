//! Runtime-owned anchored/structure-aware compute for Rhai Strategy Handling.
//!
//! The typed Rhai `anchored_config()` hook lowers to [`AnchoredConfiguration`].
//! Runtime market input then updates [`AnchoredRuntime`] from runtime-owned
//! [`MarketState`](crate::MarketState) history; strategy-facing outputs are read
//! through Market View bindings, not Strategy Context.

use indicators::anchored::{
    detectors::{PivotDetector, PivotKind},
    evaluators::{TrendLine, TrendlineEvaluator, TrendlineInvalidator, TrendlineSide},
    AnchorEvent, Invalidator, RollingDetector, SegmentState,
};
use shared::Candle;
use std::collections::{HashMap, HashSet};

/// Typed strategy-declared anchored compute configuration.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnchoredConfiguration {
    detectors: Vec<AnchoredDetectorSpec>,
    evaluators: Vec<AnchoredEvaluatorSpec>,
}

impl AnchoredConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_detector(mut self, detector: PivotDetectorConfiguration) -> Self {
        self.detectors.push(AnchoredDetectorSpec::Pivot {
            id: detector.id,
            left_bars: detector.left_bars,
            right_bars: detector.right_bars,
        });
        self
    }

    pub fn with_evaluator(mut self, evaluator: AnchoredEvaluatorConfiguration) -> Self {
        self.evaluators.push(AnchoredEvaluatorSpec::Trendline {
            expose_as: evaluator.expose_as,
            pivot_source: evaluator.pivot_source,
            side: evaluator.side.as_trendline_side(),
            pivot_buffer: evaluator.pivot_buffer,
            tolerance: evaluator.tolerance,
            min_touches: evaluator.min_touches,
            max_lines: evaluator.max_lines,
        });
        self
    }

    pub fn detectors(&self) -> &[AnchoredDetectorSpec] {
        &self.detectors
    }

    pub fn evaluators(&self) -> &[AnchoredEvaluatorSpec] {
        &self.evaluators
    }

    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty() && self.evaluators.is_empty()
    }

    pub fn validate(&self) -> Result<(), AnchoredConfigurationError> {
        let mut detector_ids = HashSet::new();
        for detector in &self.detectors {
            if detector.id().is_empty() {
                return Err(AnchoredConfigurationError::new(
                    "detector id must not be empty",
                ));
            }
            if !detector_ids.insert(detector.id().to_string()) {
                return Err(AnchoredConfigurationError::new(format!(
                    "duplicate detector id `{}`",
                    detector.id()
                )));
            }
        }

        let mut expose_names = HashSet::new();
        for evaluator in &self.evaluators {
            if evaluator.expose_as().is_empty() {
                return Err(AnchoredConfigurationError::new(
                    "evaluator expose name must not be empty",
                ));
            }
            if !expose_names.insert(evaluator.expose_as().to_string()) {
                return Err(AnchoredConfigurationError::new(format!(
                    "duplicate evaluator expose name `{}`",
                    evaluator.expose_as()
                )));
            }
            if !detector_ids.contains(evaluator.pivot_source()) {
                return Err(AnchoredConfigurationError::new(format!(
                    "evaluator `{}` references unknown pivot detector `{}`",
                    evaluator.expose_as(),
                    evaluator.pivot_source()
                )));
            }
        }

        Ok(())
    }
}

/// Typed pivot detector spec supported by the first anchored path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnchoredDetectorSpec {
    Pivot {
        id: String,
        left_bars: usize,
        right_bars: usize,
    },
}

impl AnchoredDetectorSpec {
    pub fn id(&self) -> &str {
        match self {
            Self::Pivot { id, .. } => id,
        }
    }
}

/// Typed evaluator spec supported by the first anchored path.
#[derive(Debug, Clone, PartialEq)]
pub enum AnchoredEvaluatorSpec {
    Trendline {
        expose_as: String,
        pivot_source: String,
        side: TrendlineSide,
        pivot_buffer: usize,
        tolerance: f64,
        min_touches: u32,
        max_lines: usize,
    },
}

impl AnchoredEvaluatorSpec {
    pub fn expose_as(&self) -> &str {
        match self {
            Self::Trendline { expose_as, .. } => expose_as,
        }
    }

    pub fn pivot_source(&self) -> &str {
        match self {
            Self::Trendline { pivot_source, .. } => pivot_source,
        }
    }
}

/// Builder returned by `pivot_detector::new(...)` in Rhai.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PivotDetectorConfiguration {
    id: String,
    left_bars: usize,
    right_bars: usize,
}

impl PivotDetectorConfiguration {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            left_bars: 2,
            right_bars: 2,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn with_left_bars(mut self, left_bars: usize) -> Self {
        self.left_bars = left_bars;
        self
    }

    pub fn with_right_bars(mut self, right_bars: usize) -> Self {
        self.right_bars = right_bars;
        self
    }
}

/// Builder returned by `anchored::trendline(...)` in Rhai.
#[derive(Debug, Clone, PartialEq)]
pub struct AnchoredEvaluatorConfiguration {
    expose_as: String,
    pivot_source: String,
    side: PivotSide,
    pivot_buffer: usize,
    tolerance: f64,
    min_touches: u32,
    max_lines: usize,
}

impl AnchoredEvaluatorConfiguration {
    pub fn trendline(expose_as: impl Into<String>, pivot_source: impl Into<String>) -> Self {
        Self {
            expose_as: expose_as.into(),
            pivot_source: pivot_source.into(),
            side: PivotSide::High,
            pivot_buffer: 6,
            tolerance: 0.01,
            min_touches: 3,
            max_lines: 1,
        }
    }

    pub fn expose_as(&self) -> &str {
        &self.expose_as
    }

    pub fn pivot_source(&self) -> &str {
        &self.pivot_source
    }

    pub fn with_side(mut self, side: PivotSide) -> Self {
        self.side = side;
        self
    }

    pub fn with_pivot_buffer(mut self, pivot_buffer: usize) -> Self {
        self.pivot_buffer = pivot_buffer;
        self
    }

    pub fn with_tolerance(mut self, tolerance: f64) -> Self {
        self.tolerance = tolerance;
        self
    }

    pub fn with_min_touches(mut self, min_touches: u32) -> Self {
        self.min_touches = min_touches;
        self
    }

    pub fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines;
        self
    }
}

/// Typed pivot side used by anchored config and `market.last_pivot(...)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PivotSide {
    High,
    Low,
}

impl PivotSide {
    pub fn high() -> Self {
        Self::High
    }

    pub fn low() -> Self {
        Self::Low
    }

    fn as_trendline_side(self) -> TrendlineSide {
        match self {
            Self::High => TrendlineSide::Resistance,
            Self::Low => TrendlineSide::Support,
        }
    }
}

/// Load-time validation error for typed anchored config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnchoredConfigurationError {
    message: String,
}

impl AnchoredConfigurationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for AnchoredConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "anchored_config: {}", self.message)
    }
}

impl std::error::Error for AnchoredConfigurationError {}

/// Output of one anchored evaluator, keyed by its exposed name.
#[derive(Debug, Clone, PartialEq)]
pub enum AnchoredOutput {
    Trendlines(Vec<TrendLine>),
}

/// Latest market-derived anchored outputs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AnchoredOutputs {
    pub values: HashMap<String, AnchoredOutput>,
    pub last_pivot_high: HashMap<String, PivotEvent>,
    pub last_pivot_low: HashMap<String, PivotEvent>,
}

/// Pivot event exposed through Market View.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PivotEvent {
    pub bar: u64,
    pub price: f64,
    pub volume: f64,
    pub side: PivotSide,
}

struct DetectorState {
    id: String,
    detector: Box<dyn RollingDetector + Send + Sync>,
    ring: SegmentState<AnchorEvent>,
    fired_this_tick: bool,
}

/// Runtime-owned anchored compute state.
pub struct AnchoredRuntime {
    detectors: Vec<DetectorState>,
    evaluators: Vec<AnchoredEvaluatorSpec>,
    active_trendlines: HashMap<String, Vec<(TrendLine, TrendlineInvalidator)>>,
    outputs: AnchoredOutputs,
}

impl AnchoredRuntime {
    pub fn from_config(config: &AnchoredConfiguration) -> Result<Self, AnchoredConfigurationError> {
        config.validate()?;

        let mut ring_caps: HashMap<String, usize> = HashMap::new();
        for evaluator in &config.evaluators {
            match evaluator {
                AnchoredEvaluatorSpec::Trendline {
                    pivot_source,
                    pivot_buffer,
                    ..
                } => {
                    let entry = ring_caps.entry(pivot_source.clone()).or_insert(1);
                    *entry = (*entry).max(*pivot_buffer);
                }
            }
        }

        let mut detectors = Vec::with_capacity(config.detectors.len());
        for detector in &config.detectors {
            match detector {
                AnchoredDetectorSpec::Pivot {
                    id,
                    left_bars,
                    right_bars,
                } => {
                    detectors.push(DetectorState {
                        id: id.clone(),
                        detector: Box::new(PivotDetector::new(
                            *left_bars,
                            *right_bars,
                            PivotKind::Both,
                        )),
                        ring: SegmentState::new((*ring_caps.get(id).unwrap_or(&6)).max(1)),
                        fired_this_tick: false,
                    });
                }
            }
        }

        Ok(Self {
            detectors,
            evaluators: config.evaluators.clone(),
            active_trendlines: HashMap::new(),
            outputs: AnchoredOutputs::default(),
        })
    }

    pub fn on_market_input_accepted(&mut self, candle: &Candle, history: &[Candle]) {
        let Some(bar) = history.len().checked_sub(1).map(|bar| bar as u64) else {
            return;
        };

        self.tick_primary(candle, bar, history);
    }

    pub fn outputs(&self) -> &AnchoredOutputs {
        &self.outputs
    }

    fn tick_primary(&mut self, candle: &Candle, bar: u64, candles: &[Candle]) {
        for detector in &mut self.detectors {
            detector.fired_this_tick = false;
            if let Some(event) = detector.detector.on_candle(candle, bar) {
                match event {
                    AnchorEvent::PivotHigh { bar, price, volume } => {
                        self.outputs.last_pivot_high.insert(
                            detector.id.clone(),
                            PivotEvent {
                                bar,
                                price,
                                volume,
                                side: PivotSide::High,
                            },
                        );
                    }
                    AnchorEvent::PivotLow { bar, price, volume } => {
                        self.outputs.last_pivot_low.insert(
                            detector.id.clone(),
                            PivotEvent {
                                bar,
                                price,
                                volume,
                                side: PivotSide::Low,
                            },
                        );
                    }
                    _ => {}
                }
                detector.ring.push(event);
                detector.fired_this_tick = true;
            }
        }

        for evaluator in &self.evaluators {
            match evaluator {
                AnchoredEvaluatorSpec::Trendline {
                    expose_as,
                    pivot_source,
                    side,
                    tolerance,
                    min_touches,
                    max_lines,
                    ..
                } => {
                    let Some(detector) = self
                        .detectors
                        .iter()
                        .find(|detector| detector.id == *pivot_source)
                    else {
                        continue;
                    };

                    if detector.fired_this_tick {
                        let fitter =
                            TrendlineEvaluator::new(*side, *tolerance, *min_touches, *max_lines);
                        let fresh = fitter.fit(&detector.ring, candles, 0, bar);
                        let paired = fresh
                            .iter()
                            .map(|line| (*line, TrendlineInvalidator(*line)))
                            .collect();
                        self.active_trendlines.insert(expose_as.clone(), paired);
                    }

                    let lines = self
                        .active_trendlines
                        .get(expose_as)
                        .map(|lines| lines.iter().map(|(line, _)| *line).collect())
                        .unwrap_or_default();
                    self.outputs
                        .values
                        .insert(expose_as.clone(), AnchoredOutput::Trendlines(lines));
                }
            }
        }

        for lines in self.active_trendlines.values_mut() {
            lines.retain(|(_, invalidator)| invalidator.still_valid(candle, bar));
        }
    }
}
