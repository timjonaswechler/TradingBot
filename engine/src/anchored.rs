//! Strategy-declared anchored indicator pipeline.
//!
//! Strategies optionally define `fn anchored_config()` returning a Rhai map of
//! `detectors` and `evaluators`. On engine load we parse the map into a typed
//! `AnchoredSpec`; on every `tick` we run the `AnchoredRuntime`, collect
//! outputs, and inject them into the `Context` exposed to `on_tick`.

use std::collections::{HashMap, HashSet};

use domain::Candle;
use indicators::anchored::{
    detectors::{PivotDetector, PivotKind, SessionDetector},
    evaluators::{TrendLine, TrendlineEvaluator, TrendlineInvalidator, TrendlineSide},
    AnchorEvent, AnchoredEvaluator, Invalidator, RollingDetector, Segment, SegmentState, SessionId,
};
use rhai::{Dynamic, Map, INT};

use crate::error::EngineError;

// ════════════════════════════════════════════════════════════════════════════
// Spec  —  typed representation of the strategy's anchored_config() map
// ════════════════════════════════════════════════════════════════════════════

#[derive(Debug, Clone)]
pub enum DetectorSpec {
    Pivot {
        id: String,
        left: usize,
        right: usize,
    },
    Session {
        id: String,
        session: SessionId,
        start_local: String,
        end_local: String,
        tz_hours: i32,
    },
}

impl DetectorSpec {
    pub fn id(&self) -> &str {
        match self {
            DetectorSpec::Pivot { id, .. } | DetectorSpec::Session { id, .. } => id,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EvaluatorSpec {
    Trendline {
        expose_as: String,
        side: TrendlineSide,
        pivot_source: String,
        pivot_buffer: usize,
        tolerance: f64,
        min_touches: u32,
        max_lines: usize,
    },
    SlopeBetweenPivots {
        expose_as: String,
        pivot_source: String,
        side: PivotKind, // High or Low
    },
}

impl EvaluatorSpec {
    pub fn expose_as(&self) -> &str {
        match self {
            EvaluatorSpec::Trendline { expose_as, .. }
            | EvaluatorSpec::SlopeBetweenPivots { expose_as, .. } => expose_as,
        }
    }
    pub fn pivot_source(&self) -> &str {
        match self {
            EvaluatorSpec::Trendline { pivot_source, .. }
            | EvaluatorSpec::SlopeBetweenPivots { pivot_source, .. } => pivot_source,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct AnchoredSpec {
    pub detectors: Vec<DetectorSpec>,
    pub evaluators: Vec<EvaluatorSpec>,
}

impl AnchoredSpec {
    pub fn is_empty(&self) -> bool {
        self.detectors.is_empty() && self.evaluators.is_empty()
    }

    pub fn from_rhai_map(map: Map) -> Result<Self, EngineError> {
        let mut out = AnchoredSpec::default();
        let mut detector_ids: HashSet<String> = HashSet::new();
        let mut expose_names: HashSet<String> = HashSet::new();

        // detectors: Array<Map>
        if let Some(v) = map.get("detectors") {
            let arr = v
                .clone()
                .try_cast::<rhai::Array>()
                .ok_or_else(|| spec_err("`detectors` must be an array"))?;
            for (i, item) in arr.into_iter().enumerate() {
                let m = item
                    .try_cast::<Map>()
                    .ok_or_else(|| spec_err(format!("detector[{i}] must be a map")))?;
                let d = parse_detector(&m, i)?;
                if !detector_ids.insert(d.id().to_string()) {
                    return Err(spec_err(format!("duplicate detector id `{}`", d.id())));
                }
                out.detectors.push(d);
            }
        }

        // evaluators: Array<Map>
        if let Some(v) = map.get("evaluators") {
            let arr = v
                .clone()
                .try_cast::<rhai::Array>()
                .ok_or_else(|| spec_err("`evaluators` must be an array"))?;
            for (i, item) in arr.into_iter().enumerate() {
                let m = item
                    .try_cast::<Map>()
                    .ok_or_else(|| spec_err(format!("evaluator[{i}] must be a map")))?;
                let e = parse_evaluator(&m, i)?;
                if !expose_names.insert(e.expose_as().to_string()) {
                    return Err(spec_err(format!(
                        "duplicate evaluator `expose_as: \"{}\"`",
                        e.expose_as()
                    )));
                }
                if !detector_ids.contains(e.pivot_source()) {
                    return Err(spec_err(format!(
                        "evaluator `{}` references unknown pivot_source `{}`",
                        e.expose_as(),
                        e.pivot_source()
                    )));
                }
                out.evaluators.push(e);
            }
        }

        Ok(out)
    }
}

fn spec_err(msg: impl Into<String>) -> EngineError {
    EngineError::Strategy(format!("anchored_config: {}", msg.into()))
}

fn map_get<'a>(m: &'a Map, k: &str) -> Option<&'a Dynamic> {
    m.get(k)
}

fn get_str(m: &Map, k: &str, ctx: &str) -> Result<String, EngineError> {
    map_get(m, k)
        .and_then(|v| v.clone().try_cast::<String>())
        .ok_or_else(|| spec_err(format!("{ctx}: missing string field `{k}`")))
}
fn get_int(m: &Map, k: &str, ctx: &str) -> Result<i64, EngineError> {
    map_get(m, k)
        .and_then(|v| v.clone().try_cast::<INT>())
        .map(|x| x as i64)
        .ok_or_else(|| spec_err(format!("{ctx}: missing int field `{k}`")))
}
fn get_float(m: &Map, k: &str, ctx: &str) -> Result<f64, EngineError> {
    map_get(m, k)
        .and_then(|v| {
            v.clone()
                .try_cast::<f64>()
                .or_else(|| v.clone().try_cast::<INT>().map(|i| i as f64))
        })
        .ok_or_else(|| spec_err(format!("{ctx}: missing float field `{k}`")))
}

fn parse_detector(m: &Map, idx: usize) -> Result<DetectorSpec, EngineError> {
    let ctx = format!("detector[{idx}]");
    let id = get_str(m, "id", &ctx)?;
    let kind = get_str(m, "kind", &ctx)?;
    match kind.as_str() {
        "pivot" => {
            let left = get_int(m, "left", &ctx)? as usize;
            let right = get_int(m, "right", &ctx)? as usize;
            if left < 1 || right < 1 {
                return Err(spec_err(format!("{ctx}: pivot left/right must be >= 1")));
            }
            Ok(DetectorSpec::Pivot { id, left, right })
        }
        "session" => {
            let start = get_str(m, "start", &ctx)?;
            let end = get_str(m, "end", &ctx)?;
            let tz = get_int(m, "tz", &ctx)? as i32;
            let session = match get_str(m, "session", &ctx)?.to_ascii_uppercase().as_str() {
                "ASIA" => SessionId::Asia,
                "LONDON" => SessionId::London,
                "NEWYORK" | "NY" | "NEW_YORK" => SessionId::NewYork,
                other => {
                    if let Ok(n) = other.parse::<u8>() {
                        SessionId::Custom(n)
                    } else {
                        return Err(spec_err(format!(
                            "{ctx}: session must be ASIA|LONDON|NEWYORK|<u8>, got `{other}`"
                        )));
                    }
                }
            };
            Ok(DetectorSpec::Session {
                id,
                session,
                start_local: start,
                end_local: end,
                tz_hours: tz,
            })
        }
        other => Err(spec_err(format!("{ctx}: unknown detector kind `{other}`"))),
    }
}

fn parse_evaluator(m: &Map, idx: usize) -> Result<EvaluatorSpec, EngineError> {
    let ctx = format!("evaluator[{idx}]");
    let expose_as = get_str(m, "expose_as", &ctx)?;
    let kind = get_str(m, "kind", &ctx)?;
    let pivot_source = get_str(m, "pivot_source", &ctx)?;
    match kind.as_str() {
        "trendline" => {
            let side = match get_str(m, "side", &ctx)?.to_ascii_lowercase().as_str() {
                "resistance" => TrendlineSide::Resistance,
                "support" => TrendlineSide::Support,
                other => {
                    return Err(spec_err(format!(
                        "{ctx}: side must be resistance|support, got `{other}`"
                    )))
                }
            };
            let pivot_buffer = get_int(m, "pivot_buffer", &ctx)? as usize;
            let tolerance = get_float(m, "tolerance", &ctx)?;
            let min_touches = get_int(m, "min_touches", &ctx)? as u32;
            let max_lines = get_int(m, "max_lines", &ctx)? as usize;
            if pivot_buffer < 3 {
                return Err(spec_err(format!("{ctx}: pivot_buffer must be >= 3")));
            }
            if !(tolerance > 0.0 && tolerance < 0.5) {
                return Err(spec_err(format!("{ctx}: tolerance must be in (0.0, 0.5)")));
            }
            if min_touches < 3 {
                return Err(spec_err(format!("{ctx}: min_touches must be >= 3")));
            }
            if max_lines < 1 {
                return Err(spec_err(format!("{ctx}: max_lines must be >= 1")));
            }
            Ok(EvaluatorSpec::Trendline {
                expose_as,
                side,
                pivot_source,
                pivot_buffer,
                tolerance,
                min_touches,
                max_lines,
            })
        }
        "slope_between_pivots" => {
            let side = match get_str(m, "side", &ctx)?.to_ascii_lowercase().as_str() {
                "high" => PivotKind::High,
                "low" => PivotKind::Low,
                other => {
                    return Err(spec_err(format!(
                        "{ctx}: side must be high|low, got `{other}`"
                    )))
                }
            };
            Ok(EvaluatorSpec::SlopeBetweenPivots {
                expose_as,
                pivot_source,
                side,
            })
        }
        other => Err(spec_err(format!("{ctx}: unknown evaluator kind `{other}`"))),
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Runtime  —  owns live state, called once per candle by Engine::tick
// ════════════════════════════════════════════════════════════════════════════

/// Per-detector state: the rolling detector plus a ring-buffer of its emitted events.
struct DetectorState {
    id: String,
    detector: Box<dyn RollingDetector + Send + Sync>,
    ring: SegmentState<AnchorEvent>,
    fired_this_tick: bool,
}

/// Output of one evaluator, keyed by `expose_as`.
#[derive(Debug, Clone)]
pub enum AnchoredOutput {
    Trendlines(Vec<TrendLine>),
    Slope(Option<f64>),
}

/// All outputs produced this tick, plus per-detector last events (for `context.last_pivot`).
#[derive(Debug, Clone, Default)]
pub struct AnchoredOutputs {
    pub values: HashMap<String, AnchoredOutput>,
    pub last_pivot_high: HashMap<String, (u64, f64, f64)>, // id -> (bar, price, volume)
    pub last_pivot_low: HashMap<String, (u64, f64, f64)>,
}

pub struct AnchoredRuntime {
    detectors: Vec<DetectorState>,
    evaluators: Vec<EvaluatorSpec>,
    /// Cached active trendlines (with their invalidators) per `expose_as`.
    active_trendlines: HashMap<String, Vec<(TrendLine, TrendlineInvalidator)>>,
    outputs: AnchoredOutputs,
}

impl AnchoredRuntime {
    pub fn from_spec(spec: &AnchoredSpec) -> Result<Self, EngineError> {
        let mut detectors: Vec<DetectorState> = Vec::with_capacity(spec.detectors.len());

        // pivot-buffer capacity: max across evaluators referencing a given detector.
        let mut ring_caps: HashMap<String, usize> = HashMap::new();
        for ev in &spec.evaluators {
            let cap = match ev {
                EvaluatorSpec::Trendline { pivot_buffer, .. } => *pivot_buffer,
                EvaluatorSpec::SlopeBetweenPivots { .. } => 2,
            };
            let entry = ring_caps.entry(ev.pivot_source().to_string()).or_insert(1);
            if cap > *entry {
                *entry = cap;
            }
        }

        for d in &spec.detectors {
            let (det, cap_hint): (Box<dyn RollingDetector + Send + Sync>, usize) = match d {
                DetectorSpec::Pivot { left, right, .. } => (
                    Box::new(PivotDetector::new(*left, *right, PivotKind::Both)),
                    6,
                ),
                DetectorSpec::Session {
                    session,
                    start_local,
                    end_local,
                    tz_hours,
                    ..
                } => {
                    let (s, e) =
                        SessionDetector::parse(&format!("{start_local}-{end_local}"), *tz_hours)
                            .ok_or_else(|| {
                                spec_err(format!(
                                    "session `{}`: invalid window `{}-{}`",
                                    d.id(),
                                    start_local,
                                    end_local
                                ))
                            })?;
                    (Box::new(SessionDetector::new(s, e, *session)), 4)
                }
            };
            let cap = *ring_caps.get(d.id()).unwrap_or(&cap_hint);
            detectors.push(DetectorState {
                id: d.id().to_string(),
                detector: det,
                ring: SegmentState::new(cap.max(1)),
                fired_this_tick: false,
            });
        }

        Ok(Self {
            detectors,
            evaluators: spec.evaluators.clone(),
            active_trendlines: HashMap::new(),
            outputs: AnchoredOutputs::default(),
        })
    }

    /// Run one tick. `bar` is the absolute bar index (0-based, monotonic).
    /// `candles` is the engine's candle buffer (origin bar = 0).
    pub fn tick(&mut self, candle: &Candle, bar: u64, candles: &[Candle]) {
        // 1) Detectors
        for d in &mut self.detectors {
            d.fired_this_tick = false;
            if let Some(ev) = d.detector.on_candle(candle, bar) {
                match ev {
                    AnchorEvent::PivotHigh { bar, price, volume } => {
                        self.outputs
                            .last_pivot_high
                            .insert(d.id.clone(), (bar, price, volume));
                    }
                    AnchorEvent::PivotLow { bar, price, volume } => {
                        self.outputs
                            .last_pivot_low
                            .insert(d.id.clone(), (bar, price, volume));
                    }
                    _ => {}
                }
                d.ring.push(ev);
                d.fired_this_tick = true;
            }
        }

        // 2) Evaluators: only recompute when their detector fired this tick.
        // Invalidation of already-active lines happens at the *end* of the tick
        // so the current candle still sees a just-broken line (strategies need
        // to observe the break).
        for ev in &self.evaluators {
            let src = ev.pivot_source();
            let det = match self.detectors.iter().find(|d| d.id == src) {
                Some(d) => d,
                None => continue,
            };
            let fire = det.fired_this_tick;

            match ev {
                EvaluatorSpec::Trendline {
                    expose_as,
                    side,
                    tolerance,
                    min_touches,
                    max_lines,
                    ..
                } => {
                    if fire {
                        let fitter =
                            TrendlineEvaluator::new(*side, *tolerance, *min_touches, *max_lines);
                        let fresh = fitter.fit(&det.ring, candles, 0, bar);
                        let paired: Vec<(TrendLine, TrendlineInvalidator)> = fresh
                            .iter()
                            .map(|l| (*l, TrendlineInvalidator(*l)))
                            .collect();
                        self.active_trendlines.insert(expose_as.clone(), paired);
                    }
                    let lines: Vec<TrendLine> = self
                        .active_trendlines
                        .get(expose_as)
                        .map(|v| v.iter().map(|(l, _)| *l).collect())
                        .unwrap_or_default();
                    self.outputs
                        .values
                        .insert(expose_as.clone(), AnchoredOutput::Trendlines(lines));
                }
                EvaluatorSpec::SlopeBetweenPivots {
                    expose_as, side, ..
                } => {
                    if fire {
                        let want_high = matches!(side, PivotKind::High);
                        let pts: Vec<(u64, f64)> = det
                            .ring
                            .iter()
                            .filter_map(|e| match (want_high, e) {
                                (true, AnchorEvent::PivotHigh { bar, price, .. }) => {
                                    Some((*bar, *price))
                                }
                                (false, AnchorEvent::PivotLow { bar, price, .. }) => {
                                    Some((*bar, *price))
                                }
                                _ => None,
                            })
                            .collect();
                        let val = if pts.len() >= 2 {
                            let a = pts[pts.len() - 2];
                            let b = pts[pts.len() - 1];
                            let seg = Segment {
                                start_bar: a.0,
                                end_bar: b.0,
                            };
                            indicators::anchored::evaluators::SlopeSegEvaluator
                                .evaluate(candles, 0, seg)
                        } else {
                            None
                        };
                        self.outputs
                            .values
                            .insert(expose_as.clone(), AnchoredOutput::Slope(val));
                    } else if !self.outputs.values.contains_key(expose_as) {
                        self.outputs
                            .values
                            .insert(expose_as.clone(), AnchoredOutput::Slope(None));
                    }
                }
            }
        }

        // 3) Post-exposure invalidation — prune lines broken by this bar's close
        //    so the *next* tick no longer sees them. The current tick's outputs
        //    above were already snapshotted.
        for lines in self.active_trendlines.values_mut() {
            lines.retain(|(_, inv)| inv.still_valid(candle, bar));
        }
    }

    pub fn outputs(&self) -> &AnchoredOutputs {
        &self.outputs
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Tests
// ════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use rhai::Engine as RhaiEngine;

    fn eval_config(src: &str) -> Result<AnchoredSpec, EngineError> {
        let rhai = RhaiEngine::new();
        let ast = rhai
            .compile(src)
            .map_err(|e| EngineError::Strategy(e.to_string()))?;
        let mut scope = rhai::Scope::new();
        rhai.run_ast_with_scope(&mut scope, &ast)
            .map_err(|e| EngineError::Strategy(e.to_string()))?;
        let result: Dynamic = rhai
            .call_fn(&mut scope, &ast, "anchored_config", ())
            .map_err(|e| EngineError::Strategy(e.to_string()))?;
        let map = result
            .try_cast::<Map>()
            .ok_or_else(|| EngineError::Strategy("anchored_config must return a map".into()))?;
        AnchoredSpec::from_rhai_map(map)
    }

    #[test]
    fn parses_minimal_trendline_spec() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [ #{ id: "p", kind: "pivot", left: 5, right: 5 } ],
        evaluators: [ #{
            expose_as: "res", kind: "trendline", side: "resistance",
            pivot_source: "p", pivot_buffer: 6,
            tolerance: 0.002, min_touches: 3, max_lines: 1
        } ],
    }
}
"#;
        let spec = eval_config(src).unwrap();
        assert_eq!(spec.detectors.len(), 1);
        assert_eq!(spec.evaluators.len(), 1);
    }

    #[test]
    fn rejects_duplicate_detector_id() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot", left: 5, right: 5 },
            #{ id: "p", kind: "pivot", left: 3, right: 3 },
        ],
        evaluators: [],
    }
}
"#;
        let err = eval_config(src).unwrap_err();
        assert!(format!("{err}").contains("duplicate detector id"));
    }

    #[test]
    fn rejects_duplicate_expose_as() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [ #{ id: "p", kind: "pivot", left: 5, right: 5 } ],
        evaluators: [
            #{ expose_as: "x", kind: "trendline", side: "resistance",
               pivot_source: "p", pivot_buffer: 6, tolerance: 0.002, min_touches: 3, max_lines: 1 },
            #{ expose_as: "x", kind: "trendline", side: "support",
               pivot_source: "p", pivot_buffer: 6, tolerance: 0.002, min_touches: 3, max_lines: 1 },
        ],
    }
}
"#;
        let err = eval_config(src).unwrap_err();
        assert!(format!("{err}").contains("duplicate evaluator"));
    }

    #[test]
    fn rejects_unknown_pivot_source() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [ #{ id: "p", kind: "pivot", left: 5, right: 5 } ],
        evaluators: [ #{ expose_as: "x", kind: "trendline", side: "resistance",
            pivot_source: "typo", pivot_buffer: 6, tolerance: 0.002, min_touches: 3, max_lines: 1 } ],
    }
}
"#;
        let err = eval_config(src).unwrap_err();
        assert!(format!("{err}").contains("unknown pivot_source"));
    }

    #[test]
    fn parses_session_detector_and_slope_eval() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [
            #{ id: "p", kind: "pivot", left: 3, right: 3 },
            #{ id: "lon", kind: "session", session: "LONDON",
               start: "0900", end: "1700", tz: 2 },
        ],
        evaluators: [
            #{ expose_as: "trend_slope", kind: "slope_between_pivots",
               pivot_source: "p", side: "low" },
        ],
    }
}
"#;
        let spec = eval_config(src).unwrap();
        assert_eq!(spec.detectors.len(), 2);
        match &spec.evaluators[0] {
            EvaluatorSpec::SlopeBetweenPivots { .. } => {}
            _ => panic!("expected slope_between_pivots"),
        }
    }

    #[test]
    fn runtime_builds_from_spec() {
        let src = r#"
fn anchored_config() {
    #{
        detectors: [ #{ id: "p", kind: "pivot", left: 2, right: 2 } ],
        evaluators: [ #{
            expose_as: "res", kind: "trendline", side: "resistance",
            pivot_source: "p", pivot_buffer: 6,
            tolerance: 0.01, min_touches: 3, max_lines: 1
        } ],
    }
}
"#;
        let spec = eval_config(src).unwrap();
        let _rt = AnchoredRuntime::from_spec(&spec).unwrap();
    }
}
