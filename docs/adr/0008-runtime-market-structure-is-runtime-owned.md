# ADR 0008: Runtime Market Structure is runtime-owned and explicit

- Status: accepted
- Date: 2026-06-02
- Related: #84, #97, ADR 0001, ADR 0005, ADR 0006

## Context

Issue #79 added the first typed Runtime path for market-derived structure
outputs: an optional Rhai `anchored_config()` hook, pivot detectors,
trendline evaluators, and Market View reads through `market.anchored(...)` and
`market.last_pivot(...)`.

That implementation proved the runtime path, but #84 re-reviewed whether
`anchored` is the right long-term strategy-authoring concept before adding
Fibonacci or Secondary-Timeframe structure outputs. The broader design needs to
explain:

- which Market Structure Points were detected,
- which points became Structure Anchors,
- which Structure Objects are active,
- when objects are created, touched, invalidated, replaced, or retained, and
- how later backtest/chart explanation can reconstruct object lifecycle.

The strategy-authoring API is typed/fluent per ADR 0005, while the Primary and
Secondary timeframe contract remains owned by `strategy_config()` per ADR 0006.
The broader Python-/Pine-like Rhai authoring surface belongs to #97, not #84.

## Decision

Market Structure is a runtime-owned derived domain.

It is derived from runtime-owned Market State, exposed to strategies through
Market View, and emitted for later explanation through DB-free Structure
Annotations / Runtime output. Rhai strategies may read, select, and filter
Market Structure snapshots, but persistent Structure Object truth, lifecycle,
and annotations remain owned by `trading-runtime`, not Strategy State, runner
code, DB code, UI code, or the old `engine` donor path.

The long-term strategy-authoring API should use explicit Market Structure
language instead of keeping `anchored` as the primary term. Current anchored code
and docs remain current implemented behavior until a focused implementation
issue replaces them, but future structure work should not expand `anchored` as
the target authoring surface.

### Declaration API

Use one explicit load-time hook as the single declaration surface:

```rhai
fn structure_config() {
    let s = structure_config::new();

    let swing = s.points.pivots("swing", 3, 3);

    s.objects.trendline("resistance", swing)
        .with_side(structure_side::high())
        .with_pivot_buffer(6)
        .with_tolerance(0.002)
        .with_min_touches(3)
        .with_max_active(1);

    s
}
```

Rules:

- `structure_config()` is the one explicit Market Structure declaration path.
- Do not auto-discover top-level typed Structure declarations for #84.
- Do not require duplicate top-level handles plus separate registration.
- The hook returns one namespaced registry object.
- `s.points.*` declares Market Structure Point sources.
- `s.objects.*` declares Structure Objects that depend on point sources.
- Multiple `structure_config::new()` registries in one hook are not magically
  merged.
- Required point parameters should prefer constructor arguments when this avoids
  copy-vs-mutation ambiguity in Rhai fluent calls.

### Read API

Strategies read active object snapshots through Market View:

```rhai
fn on_tick(market, context) {
    let lines = market.structure.active("resistance");

    if lines.len() == 0 {
        return decision::hold();
    }

    let current_bar = market.candles().len() - 1;
    let line = lines[0];

    if market.candle().close > line.y_at(current_bar) {
        decision::open_long(1.0).with_reason("break above resistance")
    } else {
        decision::hold()
    }
}
```

Rules:

- `market.structure.active("object_id")` addresses a declared Structure Object
  ID.
- Reading an unknown object ID is an error, not a silent empty value.
- A declared object with no currently active snapshots returns no active
  snapshots, e.g. an empty array for collection-shaped outputs.
- Strategy code reads snapshots and may filter/select them, but it does not own
  the persistent object lifecycle.

### Lifecycle and annotations

Runtime-owned Structure Objects need explicit lifecycle states and append-only
explanation records. At minimum, future implementation issues must preserve
append-only records for:

- Market Structure Point detected,
- Structure Anchor selected or changed,
- Structure Object created,
- Structure Object touched or otherwise confirmed,
- Structure Object invalidated,
- Structure Object replaced, expired, or removed from the active set.

The strategy may see only current active snapshots, but later UI/backtest
explanation must be able to reconstruct historical structure from annotations
without relying on silent overwrite-only state.

### Relationship to ADR 0001

ADR 0001 remains accepted for the narrow decision that the low-level
`fibonacci_retracements(low, high)` helper is not the long-term primary
strategy-facing Fibonacci workflow. Its `anchored evaluator` wording is
partially superseded by this ADR's Market Structure terminology and API
direction.

Before implementing strategic Fibonacci, ADR 0001 should be revised or
superseded so Fibonacci is described as a Market Structure Object or related
Market Structure primitive declared through `structure_config()`, not as a new
extension of the old `anchored` authoring term.

## Considered options

### Keep `anchored` as the long-term term

Rejected. It is too narrow for pivots, trendlines, Fibonacci, support/resistance
levels, annotations, and future explanation. It also hides the important split
between Market Structure Points, Structure Anchors, Structure Objects, and
Structure Annotations.

### Runtime-selected objects only

Partially accepted as the ownership model. Runtime owns detection and object
lifecycle from strategy-declared rules. On its own, this model can feel too
closed if strategies cannot express selection/filtering logic, so strategy code
must still be able to read and filter active snapshots.

### Strategy-selected objects from runtime points

Deferred to #97. Allowing strategies to own object IDs, persistent arrays,
mutation/deletion, and chart-object lifecycle would require a broader scripting
state model. That should be designed with the general Rhai authoring surface,
not smuggled into #84.

### Hybrid model

Accepted. Runtime owns point/object truth, lifecycle, and annotations;
strategies declare the structure pipelines at load time and may read, select,
and filter active snapshots at tick time.

### Top-level typed declarations with auto-discovery

Rejected for #84. Auto-discovery would introduce a new loader pattern, make
activation depend on top-level declaration side effects, and create ambiguity
around aliases/renames. `structure_config()` remains the explicit hook.

## Consequences

- New Market Structure behavior belongs in `trading-runtime` and remains
  DB-free.
- Runners, DB adapters, UI code, and Strategy State must not become Structure
  Object truth stores.
- Existing `trading-runtime/src/anchored.rs` is current runtime-facing behavior
  but should be treated as the behavior to reframe/port into explicit Market
  Structure names, not as the term to expand indefinitely.
- The old `engine` anchored path remains donor material only.
- Future implementation issues need tests for declaration validation, unknown ID
  reads, inactive declared objects, lifecycle annotations, and current active
  snapshot reads.
- #97 owns broader Rhai programmability, session-local persistent variable API,
  user modules/libraries, and Rust strategy/plugin alternatives.
