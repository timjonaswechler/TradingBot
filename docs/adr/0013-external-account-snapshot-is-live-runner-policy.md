# ADR 0013: External Account Snapshot is Live Runner policy

- Status: accepted
- Date: 2026-06-10

## Context

#39 clarified how live account state should influence future Real-Money execution without mixing broker/account truth into the Trading Runtime. The tempting alternatives were to expose raw broker values such as `available_cash` / `buying_power` to strategies through `context`, or to make `trading-runtime` own account snapshot reconciliation.

## Decision

External Account Snapshot is V1 Live Runner / adapter policy input, not Trading Runtime state and not raw strategy-facing context. The Live Runner evaluates snapshot availability, freshness, and validity before feeding each tradable Primary candle into `trading-runtime`; if policy blocks, that candle is not fed and no Strategy Tick or runtime-local Portfolio Transition is produced for it.

Raw broker/account fields such as `available_cash`, `buying_power`, margin, broker positions, and open orders are not exposed directly to strategies. Affordability based on buying power belongs to a later Broker Order Gate after Strategy Decision / Execution Planning, where the system can distinguish new exposure from exposure-reducing exits. Cross-strategy capital allocation belongs to a later Portfolio Coordinator that can translate External Account Snapshot data into per-runtime budgets/allocations.

## Consequences

- `trading-runtime` remains broker/account-free and keeps runtime-local Portfolio State separate from External Account Snapshot.
- Paper Trading Persistence remains simulated-live truth only and must not become Real-Money broker/account truth.
- The first broker-adapter-free #39 follow-up can implement a pure snapshot policy in `trading-daemon` without broker APIs, DB authority, order lifecycle, or Runtime changes.
- Strategies may later receive a computed per-runtime budget/allocation only after a separate accepted Portfolio Coordinator / allocation decision, not raw broker account state by default.
