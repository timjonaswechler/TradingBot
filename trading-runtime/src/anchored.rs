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
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex},
};

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

/// Typed strategy-declared Market Structure configuration registry.
///
/// The Rhai `structure_config()` hook returns this registry. Namespaced
/// `points` and `objects` handles share the same inner state so fluent object
/// configuration mutates the returned registry instead of a detached copy.
#[derive(Debug, Clone, Default)]
pub struct StructureConfiguration {
    inner: Arc<Mutex<StructureConfigurationState>>,
}

#[derive(Debug, Clone, Default)]
struct StructureConfigurationState {
    detectors: Vec<AnchoredDetectorSpec>,
    evaluators: Vec<AnchoredEvaluatorSpec>,
    sealed: bool,
}

impl StructureConfiguration {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn points(&self) -> StructurePointRegistry {
        StructurePointRegistry {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn objects(&self) -> StructureObjectRegistry {
        StructureObjectRegistry {
            inner: Arc::clone(&self.inner),
        }
    }

    pub fn is_empty(&self) -> bool {
        let state = self.lock_state();
        state.detectors.is_empty() && state.evaluators.is_empty()
    }

    pub fn validate(&self) -> Result<(), StructureConfigurationError> {
        let state = self.lock_state();
        validate_structure_state(&state)
    }

    pub fn to_anchored_configuration(&self) -> AnchoredConfiguration {
        let state = self.lock_state();
        AnchoredConfiguration {
            detectors: state.detectors.clone(),
            evaluators: state.evaluators.clone(),
        }
    }

    pub fn object_ids(&self) -> HashSet<String> {
        let state = self.lock_state();
        state
            .evaluators
            .iter()
            .map(|evaluator| evaluator.expose_as().to_string())
            .collect()
    }

    pub fn seal(&self) {
        let mut state = self.lock_state();
        state.sealed = true;
    }

    fn lock_state(&self) -> std::sync::MutexGuard<'_, StructureConfigurationState> {
        self.inner
            .lock()
            .expect("structure configuration state should not be poisoned")
    }
}

/// Namespaced `s.points` registry returned to Rhai strategies.
#[derive(Debug, Clone)]
pub struct StructurePointRegistry {
    inner: Arc<Mutex<StructureConfigurationState>>,
}

impl StructurePointRegistry {
    pub fn pivots(
        &self,
        id: impl Into<String>,
        left_bars: usize,
        right_bars: usize,
    ) -> Result<StructurePointSource, StructureConfigurationError> {
        let id = id.into();
        let mut state = self
            .inner
            .lock()
            .expect("structure configuration state should not be poisoned");

        ensure_structure_state_is_mutable(&state)?;

        if id.is_empty() {
            return Err(StructureConfigurationError::new(
                "point id must not be empty",
            ));
        }
        if state.detectors.iter().any(|detector| detector.id() == id) {
            return Err(StructureConfigurationError::new(format!(
                "duplicate point id `{id}`"
            )));
        }

        state.detectors.push(AnchoredDetectorSpec::Pivot {
            id: id.clone(),
            left_bars,
            right_bars,
        });

        Ok(StructurePointSource { id })
    }
}

/// Typed point-source handle returned by `s.points.*` declarations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructurePointSource {
    id: String,
}

impl StructurePointSource {
    pub fn id(&self) -> &str {
        &self.id
    }
}

/// Namespaced `s.objects` registry returned to Rhai strategies.
#[derive(Debug, Clone)]
pub struct StructureObjectRegistry {
    inner: Arc<Mutex<StructureConfigurationState>>,
}

impl StructureObjectRegistry {
    pub fn trendline(
        &self,
        object_id: impl Into<String>,
        point_source: StructurePointSource,
    ) -> Result<StructureObjectConfiguration, StructureConfigurationError> {
        let object_id = object_id.into();
        let mut state = self
            .inner
            .lock()
            .expect("structure configuration state should not be poisoned");

        ensure_structure_state_is_mutable(&state)?;

        if object_id.is_empty() {
            return Err(StructureConfigurationError::new(
                "object id must not be empty",
            ));
        }
        if state
            .evaluators
            .iter()
            .any(|evaluator| evaluator.expose_as() == object_id)
        {
            return Err(StructureConfigurationError::new(format!(
                "duplicate object id `{object_id}`"
            )));
        }
        if !state
            .detectors
            .iter()
            .any(|detector| detector.id() == point_source.id())
        {
            return Err(StructureConfigurationError::new(format!(
                "object `{object_id}` references unknown point source `{}`",
                point_source.id()
            )));
        }

        state.evaluators.push(AnchoredEvaluatorSpec::Trendline {
            expose_as: object_id.clone(),
            pivot_source: point_source.id().to_string(),
            side: PivotSide::High.as_trendline_side(),
            pivot_buffer: 6,
            tolerance: 0.01,
            min_touches: 3,
            max_lines: 1,
        });

        Ok(StructureObjectConfiguration {
            inner: Arc::clone(&self.inner),
            object_id,
        })
    }
}

/// Typed object handle returned by `s.objects.*` declarations.
#[derive(Debug, Clone)]
pub struct StructureObjectConfiguration {
    inner: Arc<Mutex<StructureConfigurationState>>,
    object_id: String,
}

impl StructureObjectConfiguration {
    pub fn with_side(self, side: PivotSide) -> Result<Self, StructureConfigurationError> {
        self.update_trendline(|evaluator_side, _, _, _, _| {
            *evaluator_side = side.as_trendline_side();
        })?;
        Ok(self)
    }

    pub fn with_pivot_buffer(
        self,
        pivot_buffer: usize,
    ) -> Result<Self, StructureConfigurationError> {
        self.update_trendline(|_, evaluator_pivot_buffer, _, _, _| {
            *evaluator_pivot_buffer = pivot_buffer;
        })?;
        Ok(self)
    }

    pub fn with_tolerance(self, tolerance: f64) -> Result<Self, StructureConfigurationError> {
        self.update_trendline(|_, _, evaluator_tolerance, _, _| {
            *evaluator_tolerance = tolerance;
        })?;
        Ok(self)
    }

    pub fn with_min_touches(self, min_touches: u32) -> Result<Self, StructureConfigurationError> {
        self.update_trendline(|_, _, _, evaluator_min_touches, _| {
            *evaluator_min_touches = min_touches;
        })?;
        Ok(self)
    }

    pub fn with_max_active(self, max_active: usize) -> Result<Self, StructureConfigurationError> {
        self.update_trendline(|_, _, _, _, evaluator_max_lines| {
            *evaluator_max_lines = max_active;
        })?;
        Ok(self)
    }

    fn update_trendline(
        &self,
        update: impl FnOnce(&mut TrendlineSide, &mut usize, &mut f64, &mut u32, &mut usize),
    ) -> Result<(), StructureConfigurationError> {
        let mut state = self
            .inner
            .lock()
            .expect("structure configuration state should not be poisoned");
        ensure_structure_state_is_mutable(&state)?;

        let Some(evaluator) = state
            .evaluators
            .iter_mut()
            .find(|evaluator| evaluator.expose_as() == self.object_id)
        else {
            return Err(StructureConfigurationError::new(format!(
                "unknown object id `{}`",
                self.object_id
            )));
        };

        match evaluator {
            AnchoredEvaluatorSpec::Trendline {
                side,
                pivot_buffer,
                tolerance,
                min_touches,
                max_lines,
                ..
            } => update(side, pivot_buffer, tolerance, min_touches, max_lines),
        }

        Ok(())
    }
}

/// Load-time validation error for typed Market Structure config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructureConfigurationError {
    message: String,
}

impl StructureConfigurationError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for StructureConfigurationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "structure_config: {}", self.message)
    }
}

impl std::error::Error for StructureConfigurationError {}

fn ensure_structure_state_is_mutable(
    state: &StructureConfigurationState,
) -> Result<(), StructureConfigurationError> {
    if state.sealed {
        Err(StructureConfigurationError::new(
            "registry is sealed after load-time evaluation",
        ))
    } else {
        Ok(())
    }
}

fn validate_structure_state(
    state: &StructureConfigurationState,
) -> Result<(), StructureConfigurationError> {
    let mut point_ids = HashSet::new();
    for detector in &state.detectors {
        if detector.id().is_empty() {
            return Err(StructureConfigurationError::new(
                "point id must not be empty",
            ));
        }
        if !point_ids.insert(detector.id().to_string()) {
            return Err(StructureConfigurationError::new(format!(
                "duplicate point id `{}`",
                detector.id()
            )));
        }
    }

    let mut object_ids = HashSet::new();
    for evaluator in &state.evaluators {
        if evaluator.expose_as().is_empty() {
            return Err(StructureConfigurationError::new(
                "object id must not be empty",
            ));
        }
        if !object_ids.insert(evaluator.expose_as().to_string()) {
            return Err(StructureConfigurationError::new(format!(
                "duplicate object id `{}`",
                evaluator.expose_as()
            )));
        }
        if !point_ids.contains(evaluator.pivot_source()) {
            return Err(StructureConfigurationError::new(format!(
                "object `{}` references unknown point source `{}`",
                evaluator.expose_as(),
                evaluator.pivot_source()
            )));
        }
    }

    Ok(())
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
