# ADR 0011: Position Risk Updates use explicit partial boundary-change semantics

- Status: accepted
- Date: 2026-06-08

## Context

Position Risk Updates let a strategy change the current Stop-Loss and Take-Profit boundaries of an existing Open Position after entry. This is deliberately separate from Entry Risk Parameters on opening decisions: Entry Risk Parameters create initial Position Risk Boundaries, while Position Risk Updates change the current boundaries later.

## Decision

Position Risk Updates are strategy-produced Strategy Decisions only in V1. They apply to the current Open Position of the Runtime Session, not to side-specific intents, position IDs, runner commands, or broker/order-modification APIs.

The Strategy Decision uses explicit per-boundary change semantics: `Unchanged`, `Set(price)`, or `Clear`. Strategy-facing Rhai should expose this through a typed/fluent API such as `decision::update_position_risk().set_stop_loss(price).clear_take_profit().with_reason(...)`; update methods use `set_*` / `clear_*` names rather than opening-decision `with_stop_loss` / `with_take_profit` names.

The Runtime validates newly set boundaries against the current Primary Tradable Candle close. A newly accepted boundary must not already be crossed at that mark price; accepted boundaries can first trigger on the next Tradable Candle. Clearing is idempotent, and requested changes that already match the current boundary state are successful no-op outcomes.

If one requested boundary change is valid and another is invalid, V1 partially applies the valid change and reports the invalid change with a machine-readable reason. This favors risk-safety: a valid protective Stop-Loss update should not be lost merely because an independent Take-Profit update was invalid.

## Consequences

A Position Risk Update is a Portfolio Transition because it changes the Open Position's current Position Risk Boundaries, but it is not a broker fill, does not realize PnL, and does not increment completed trade count. Runtime output should use a result event such as `PositionRiskUpdateEvaluated` with applied and rejected boundary-change lists, and should emit `PortfolioUpdated` only when the boundary state actually changes.

Runtime implementation and Paper Trading persistence projection are separate slices. Persistent Paper Trading must project accepted Position Risk Boundary changes into `paper_open_positions` before relying on strategy risk updates, otherwise restart would restore stale boundaries.
