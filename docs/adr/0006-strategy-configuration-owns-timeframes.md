# ADR 0006: Strategy Configuration owns the timeframe contract

- Status: accepted
- Date: 2026-05-31

Strategy Configuration owns the timeframe contract for a Runtime strategy: the strategy declares exactly one Primary Timeframe and any Secondary-Timeframe requirements/defaults, while Run Configuration binds that strategy to a Runtime Asset, source/mode, portfolio inputs, and runner policies. We rejected defining Primary/Secondary Timeframes independently in runner configuration because it creates two sources of truth that can drift and make backtest/live dataset loading inconsistent.

## Consequences

- Runtime strategy loading should require a Strategy Configuration with a Primary Timeframe.
- Live runner configuration should stop using asset interval lists as the source of Primary/Secondary Timeframes.
- Backtest Plans should name the Runtime Asset and visible window, then derive Primary/Secondary dataset loading from the strategy's timeframe contract.
- Existing run-config-selected timeframe paths are transitional until the migration issue replaces them.
