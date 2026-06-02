# Runtime Market Structure design (#84)

This document records the #84 design outcome. It is a design/decision artifact,
not an implementation plan or strategy-author reference for currently shipped
API. Current implemented API still uses `anchored_config()`, `market.anchored`,
and `market.last_pivot`; the target #84 direction is the explicit Market
Structure surface documented here and in ADR 0008.

## Scope

#84 decides the concept and boundaries for runtime-owned Market Structure before
adding Fibonacci or Secondary-Timeframe structure outputs.

In scope:

- review the current anchored behavior,
- decide whether `anchored` remains the long-term term,
- define the accepted declaration/read API direction,
- define ownership and lifecycle boundaries,
- define what needs append-only Structure Annotations,
- compare Runtime-selected, Strategy-selected, and hybrid models, and
- decide the ADR 0001 Fibonacci relationship.

Out of scope:

- implementing the new API,
- implementing Fibonacci,
- implementing Secondary-Timeframe structure outputs,
- UI/manual drawing behavior,
- DB persistence mapping,
- Portfolio/Execution behavior, and
- the broader Python-/Pine-like Rhai strategy-authoring design owned by #97.

## Code classification used for this review

- `trading-runtime/src/anchored.rs` — canonical current runtime-facing behavior
  for the typed Runtime path, but its `anchored` authoring term is not the
  long-term target term.
- `trading-runtime/src/rhai_strategy.rs` — canonical current typed Rhai adapter.
  Future Market Structure hooks/read APIs should be implemented here or in a
  runtime-owned submodule, not in `engine`.
- `indicators/src/anchored/*` — pure detector/evaluator donor primitives. They
  may supply algorithms but must not learn about Rhai, DB, runners, or runtime
  state.
- `engine/src/anchored.rs` — donor only. Do not extend it as target
  architecture.

## Current anchored behavior review

The current typed Runtime path from #79 has these useful properties:

- `anchored_config()` is optional.
- When present, it must return a typed `AnchoredConfig`.
- Load-time validation rejects duplicate detector IDs, duplicate evaluator
  expose names, invalid pivot windows, and evaluator references to unknown
  pivot detectors.
- Runtime-owned compute updates from accepted Primary-Timeframe Market State.
- Pivot detectors produce confirmed pivot events with right-bar delay.
- Trendline evaluators build active trendline outputs from a bounded pivot
  buffer.
- Market View exposes current outputs through `market.anchored(name)` and
  `market.last_pivot(...)`.
- Structure outputs are not exposed through `context.*` compatibility aliases.

The review also found lifecycle gaps that #84 must fix before adding richer
structure tools:

- `anchored` is too narrow as the long-term term; it does not describe the full
  domain of Market Structure Points, Structure Anchors, Structure Objects, and
  Structure Annotations.
- Current outputs are current-state snapshots only. They do not emit an
  append-only explanation stream for detected pivots, selected anchors, object
  creation, touches, invalidation, replacement, or expiry.
- Recomputed trendline output can replace the prior active set for an exposed
  name. Without annotations, a later report cannot reliably reconstruct why the
  active object changed.
- Invalidation exists as runtime-owned state, but the strategy-facing and
  explanation-facing lifecycle must be documented and tested explicitly in the
  future implementation issue. Silent overwrite-only behavior is not acceptable
  for the target model.
- Current outputs are Primary-Timeframe-only. Secondary-Timeframe structure
  outputs remain future scope and must not be added until the Market Structure
  model is implemented.

## Accepted concept

Market Structure is a runtime-owned derived domain:

1. Market State is the source input.
2. Runtime-owned Structure compute detects Market Structure Points.
3. Runtime-owned rules select Structure Anchors and maintain Structure Objects.
4. Runtime emits DB-free Structure Annotations / Runtime output for explanation.
5. Market View exposes active snapshots to Rhai strategies.
6. Rhai strategies may read, select, and filter snapshots, but they do not own
   persistent Structure Object truth or lifecycle.

Structure truth must not live in Strategy State, runner code, DB code, UI code,
or the old `engine` donor path.

## Accepted declaration API direction

Use `structure_config()` as the single explicit declaration surface.

```rhai
fn structure_config() {
    let s = structure_config::new();

    let swing_fast = s.points.pivots("swing_fast", 3, 3);
    let swing_slow = s.points.pivots("swing_slow", 10, 10);

    s.objects.trendline("fast_resistance", swing_fast)
        .with_side(structure_side::high())
        .with_pivot_buffer(6)
        .with_tolerance(0.01)
        .with_min_touches(3)
        .with_max_active(1);

    s.objects.trendline("slow_support", swing_slow)
        .with_side(structure_side::low())
        .with_pivot_buffer(6)
        .with_tolerance(0.015)
        .with_min_touches(3)
        .with_max_active(1);

    s
}
```

Declaration rules:

- `structure_config()` returns one registry object.
- `s.points.*` declares point sources.
- `s.objects.*` declares objects.
- Point/object IDs inside the hook are declaration IDs and references, not a
  second registration path.
- Do not require top-level constants plus separate registration.
- Do not auto-discover top-level typed declarations.
- Do not magically merge multiple `structure_config::new()` registries from one
  hook.
- Required point parameters should be constructor arguments if fluent mutation
  would be ambiguous in Rhai. If fluent methods are used after construction, the
  implementation must test that the returned final registry contains the fluent
  options rather than an unchanged copy.

Rejected duplicate pattern:

```rhai
const SWING = structure_points::pivots("swing", 3, 3);

fn structure_config() {
    structure_config::new().with_point_source(SWING) // duplicate path
}
```

Rejected auto-discovery pattern for #84:

```rhai
const SWING = structure_points::pivots("swing", 3, 3);
const RESISTANCE = structure_objects::trendline("resistance", SWING);
// Runtime does not auto-enable these top-level declarations in #84.
```

## Accepted read API direction

Use `market.structure.active("object_id")` for active snapshots of declared
Structure Objects.

```rhai
fn on_tick(market, context) {
    let lines = market.structure.active("fast_resistance");

    if lines.len() == 0 {
        return decision::hold();
    }

    let current_bar = market.candles().len() - 1;
    let line = lines[0];

    if market.candle().close > line.y_at(current_bar) {
        return decision::open_long(1.0)
            .with_reason("break above resistance");
    }

    decision::hold()
}
```

Read rules:

- The ID must refer to an object declared by `structure_config()`.
- Unknown object IDs are strategy/runtime errors, not silent `()` values.
- Declared but currently inactive objects return no active snapshots, e.g. an
  empty array for collection-shaped outputs.
- Strategy code can filter/select snapshots and build trading decisions from
  them.
- Strategy code cannot persist Structure Object truth by storing runtime object
  handles in Strategy State.

## Append-only Structure Annotation boundary

The active snapshot API may stay compact for strategy authors, but the runtime
must emit append-only explanation records for lifecycle events. Minimum required
annotation concepts:

- `point_detected` — a Market Structure Point was confirmed, including source
  point configuration, side/kind, bar/timestamp, price, and volume when
  available.
- `anchor_selected` — one or more points were selected as Structure Anchors for
  an object rule.
- `object_created` — a Structure Object became active, including object ID,
  kind, anchor references, and object parameters needed to draw/evaluate it.
- `object_touched` — an active object was confirmed/touched by later market
  data when the object kind tracks touches.
- `object_invalidated` — an active object became invalid for a documented
  reason, such as price crossing a trendline invalidator.
- `object_replaced` — a new object snapshot superseded an older one for the
  same declared object ID.
- `object_expired` / `object_removed` — an active object left the active set due
  to configured budget, age, or other lifecycle policy.

These annotations are DB-free Runtime output. Persistence mapping, if any,
belongs outside `trading-runtime`.

Implementation issues may choose exact event/type names, but they must not
rely only on final active maps if that loses historical explanation.

## Model comparison

### Runtime-selected Structure Objects

The runtime owns detection, anchor selection, active object truth, invalidation,
replacement, and annotations from strategy-declared rules.

This is safe for live/backtest parity and explanation because all object truth
comes from Market State and runtime-owned rules. It can be too rigid if
strategies cannot express selection/filtering logic, so the read API must expose
enough snapshots for strategy code to choose how to trade.

### Strategy-selected Structure Objects

The runtime would expose points and let the Rhai strategy create, mutate, store,
and delete objects itself.

This is deferred/rejected for #84. It requires broader decisions about
persistent session-local variables, object IDs, arrays/maps, user libraries, and
script-owned lifecycle. Those belong to #97. It would also risk moving Structure
Object truth into Strategy State, which #84 rejects.

### Hybrid model

Accepted for #84: strategy declares the structure pipelines and may read,
select, and filter active snapshots; runtime owns point detection, anchor/object
truth, lifecycle, invalidation/replacement, and annotations.

This preserves runtime ownership and explainability while still leaving strategy
logic programmable inside `on_tick`.

## Pine Script lessons used, not copied

Pine is useful as a lifecycle reference because it has explicit pivot
confirmation, line/object IDs, mutation/deletion, value-at-bar reads, and object
budgets. #84 adopts the lifecycle lesson: object creation, mutation,
invalidation, replacement, and deletion must be explicit enough to explain.

#84 does not adopt Pine syntax, a Pine parser, script-owned chart object IDs, or
persistent arrays/maps. Those broader authoring concerns belong to #97.

## ADR 0001 decision

ADR 0001 should be revised or superseded before strategic Fibonacci is
implemented. Its decision that the low-level Fibonacci helper is not the primary
strategy-facing workflow still stands. Its wording that the long-term model is
an `anchored evaluator` is superseded by the Market Structure direction: future
Fibonacci should be described as a Structure Object or related Market Structure
primitive declared through `structure_config()`.

## Follow-up implementation issue themes

Do not implement these in #84. Once a human maintainer is ready to split work,
focused child issues should be created around:

1. Introduce `structure_config()` and typed Market Structure declaration models.
2. Rename/reframe strategy-facing `anchored` APIs toward `market.structure.*`.
3. Add active snapshot reads with unknown-ID errors and inactive-empty behavior.
4. Add append-only Structure Annotation / Runtime output types.
5. Port current pivot/trendline behavior into the Market Structure naming model
   with tests preserving intended behavior.
6. Revise/supersede ADR 0001 and then implement strategic Fibonacci under the
   Market Structure model.
7. Add Secondary-Timeframe structure outputs after the structure API is stable.
