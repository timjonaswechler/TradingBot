# ADR 0002: Build trading-runtime as a new crate and absorb engine selectively

- Status: accepted
- Date: 2026-05-23

## Context

The runtime refactor originally considered mechanically renaming the current `engine` crate to `trading-runtime`. During the #32 architecture grilling session, we decided to build `trading-runtime` as a new crate/module instead, because the target runtime is broader than the current engine: it coordinates market input, strategy handling, runtime-local portfolio state, execution transitions, and ordered runtime events.

## Decision

We will create a new `trading-runtime` crate as the explicit target architecture. The current `engine` crate is a temporary donor for Rhai strategy handling, warmup detection, indicator bindings, anchored runtime behavior, and strategy state behavior; those pieces should be transferred selectively when the corresponding runtime modules are ready.

`trading-runtime` must not depend permanently on `engine`. Once the migration is complete, the old `engine` crate should be removed or fully absorbed.

## Consequences

This makes the first implementation slice slightly slower than a mechanical rename, but it avoids shaping the new runtime around the old engine boundary. It also keeps the architecture honest: runtime core, events, portfolio state, and execution semantics can be established before strategy handling is migrated.
