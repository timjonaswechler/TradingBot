# ADR 0001: Model strategic Fibonacci as an anchored structure indicator

- Status: accepted; partially superseded by ADR 0008 for Market Structure terminology/API direction
- Date: 2026-05-11

## Context

The repo currently exposes:

- a low-level Rust helper `fibonacci_retracements(low, high)`
- a public Rhai function `indicators::fibonacci(candles, low, high)`

That public API computes price levels from explicit `low` and `high` inputs, but
it does not model how Fibonacci is usually used in strategy logic.

It does not:
- detect swings from candle structure
- derive ranges from confirmed pivots
- participate in the anchored event-driven pipeline
- expose a structured swing-aware output

This makes the current function useful as a primitive utility, but too shallow
for the main strategic Fibonacci workflow.

## Decision

We will treat the current public `indicators::fibonacci(candles, low, high)`
function as a **low-level utility**, not as the long-term primary Fibonacci
indicator.

The long-term strategy-facing Fibonacci model will be implemented as an
**anchored evaluator**.

### Version 1 direction

The first anchored Fibonacci version should:

- build on the existing pivot-detector pipeline
- derive the range from the **last confirmed opposite-side pivot pair**
- be configured as an anchored evaluator with:

```rhai
kind: "fibonacci_retracement"
```

- return a **minimal structured output**, including:
  - swing direction
  - segment boundaries
  - computed levels

This output should be documented as intentionally minimal in v1 and explicitly
open to future expansion.

### Update from ADR 0008

ADR 0008 reframes the long-term strategy-facing API from `anchored` to explicit
Market Structure terminology. Before implementing strategic Fibonacci, this ADR
should be revised or superseded so Fibonacci is described as a Structure Object
or related Market Structure primitive declared through `structure_config()`,
not as a new extension of the old `anchored` authoring term.

### Naming direction

The current helper may later be renamed to something clearer like:

- `fibonacci_levels`

if and when the public API is cleaned up. Its role should remain that of a
primitive calculation helper.

## Consequences

### Positive

- strategic Fibonacci becomes aligned with actual market-structure usage
- Fibonacci shares the same structural language as trendlines and pivot-derived
  evaluators
- the system avoids duplicating separate swing definitions
- event-driven recomputation fits the anchored architecture well

### Negative

- implementation is more complex than keeping Fibonacci as a simple math helper
- the first anchored version needs a new output contract and documentation
- there will be a period where both the low-level utility and the anchored
  model coexist

## Rejected alternatives

### Keep Fibonacci only as `indicators::fibonacci(candles, low, high)`

Rejected because this leaves the burden of swing detection entirely to strategy
authors and keeps Fibonacci outside the repo's structure-aware model.

### Add offset support to the current public Fibonacci helper

Rejected because the current helper is driven by explicit `low` and `high`
arguments rather than candle history. An offset parameter would add little real
meaning and could imply behavior that does not exist.

### Build a separate Fibonacci-specific swing detector

Rejected for now because the repo already has pivot detectors and should prefer
one shared structural language for swing-based tools.
