# ADR 0012: Execution Cost Model is runtime-owned simulated fill accounting

- Status: accepted
- Date: 2026-06-09

## Context

#42 clarified how TradingBot2 should represent fees, spread, slippage, and fill assumptions without reintroducing separate live/backtest execution semantics. The #33 V1 Portfolio State deliberately had no costs: Strategy Entry/Exit used candle close, Risk Exits used their selected risk price, Force Close used the mark-candle close, and PnL was purely price × quantity. That default remains useful, but Backtest Sessions and Paper Trading need an explicit, typed way to model realistic costs without mixing in broker account truth.

## Decision

Execution cost semantics belong to `trading-runtime`. Runners configure an `ExecutionCostModel` for a Runtime Session, but they do not calculate their own PnL, fill-price, fee, or spread semantics. Rhai strategies do not configure the cost model.

V1 supports broker-neutral simulated costs:

- no fees / no spread as the default;
- fixed fee per fill;
- percent fee per fill, calculated from `abs(quantity * effective_fill_price)`;
- fixed plus percent fee per fill;
- fixed absolute spread, applied as half-spread against the fill side (`Buy` pays above the base price, `Sell` receives below the base price).

Slippage remains zero in Backtest and Paper Trading V1. Advanced simulated slippage and variable bid/ask spread from market data need separate #42 follow-ups. Broker-specific fee schedules, broker-reported fills/costs, pending orders, partial fills, and fill reconciliation remain #124 / later broker-integration work.

Runtime output should expose machine-readable Execution Fill data: fill side, quantity, base execution price, effective fill price, signed price adjustment, fixed fee, percent fee, total costs, and fill source. V1 fill source is simulated. Existing portfolio-transition events should carry this fill/cost breakdown rather than introducing a fake order-lifecycle event family.

Runtime-local accounting applies costs at fill time without introducing buying-power, margin, or notional-reservation semantics. Opening a position uses the effective entry fill price and subtracts entry fees from runtime-local realized cash, but still does not reserve/subtract notional. Closing uses the effective exit fill price, records gross PnL separately from costs, and treats `realized_pnl` as net realized PnL. Over the full position lifecycle:

```text
cash_start - entry_fee + gross_pnl - exit_fee = cash_start + net_realized_pnl
```

Invalid cost configuration is a technical configuration error at Runtime/Session construction time, not a Strategy Decision or Runtime Event. Fixed fees, percent rates, and fixed spread must be finite and non-negative.

## Consequences

- Backtest Sessions and Paper Trading share the same runtime-owned simulated fill/cost semantics.
- Backtester and Live Runner code may configure the cost model and consume Runtime Events, but must not own duplicate Portfolio/Execution semantics.
- `trading-runtime` remains DB-free and broker/account-free; #39 owns External Account Snapshot/account reconciliation, and #124 owns real-money broker account integration and broker-reported fill semantics.
- Paper Trading persistence should project the fill/cost breakdown into `paper_open_positions` / `paper_trades` when cost support is implemented, so restored and reported Paper Trading data stays explainable.
