# ADR 0010: Paper trading persistence projects runtime portfolio transitions

- Status: accepted
- Date: 2026-06-05

## Context

The Trading Runtime emits ordered, DB-free Runtime Events. During the #37 persistence-seam clarification, we needed to decide whether those events become a persisted event store, whether existing `live_positions` / `live_trades` tables remain the target, and whether the Runtime should know about paper vs real-money execution modes.

## Decision

Runtime Events are not a persisted event store in V1. Paper Trading persistence is a runner/adapter projection: the Paper Trading Persistence Adapter consumes runtime-local Portfolio Transition events such as `PositionOpened` and `PositionClosed` and projects them into dedicated paper persistence tables (`paper_open_positions` and `paper_trades`) using deterministic projection keys and idempotent/atomic DB operations.

Real-Money Live execution must not treat this Paper Trading projection as broker truth. For real-money trading, the broker/trading provider is the source of truth for actual exposure, orders, fills, buying power, and account state; DB records may be cache, audit, or metadata only until separate broker/account reconciliation decisions exist.

The Trading Runtime itself must not know whether a runner is backtest, paper, or real-money live. Runners and adapters provide the appropriate runtime inputs or commands so all modes keep using one runtime boundary without embedding broker/DB mode flags in `trading-runtime`.

## Consequences

- `trading-runtime` remains DB-free and mode-free.
- Paper Trading can safely restore runtime-local Portfolio State from paper persistence records.
- Existing `live_positions` / `live_trades` stay transitional legacy storage for old PaperExecutor paths until replacement/deletion criteria are met.
- Real-Money Live account reconciliation remains owned by #39 / later broker-execution decisions, not by #37 Paper Trading persistence.
