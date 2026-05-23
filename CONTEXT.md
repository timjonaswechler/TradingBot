# TradingBot Context

This file documents the domain language used by TradingBot2 contributors and strategy authors. It is a glossary, not an implementation plan.

## Language

**Strategy State**:
Memory that belongs to one running strategy and persists between ticks. Strategy State is visible to Rhai strategies through the strategy context and is not the authoritative account or portfolio state.
_Avoid_: State, Engine State

**Portfolio State**:
The canonical trading state of a live or simulated trading session: realized cash balance, open position, and completed trade count. Equity is derived from Portfolio State and current market prices rather than treated as independent account truth.
_Avoid_: Strategy State, Context State, External Account Snapshot

**Runtime Portfolio Snapshot**:
A point-in-time view of one Trading Runtime's Portfolio State for a runtime step. It includes cash balance, open position, completed trade count, and current equity derived from the current mark price.
_Avoid_: External Account Snapshot, Account Balance

**External Account Snapshot**:
A point-in-time view of account resources outside one Trading Runtime, such as available cash, buying power, margin, or external exposure. An External Account Snapshot may inform live strategy execution, but it is not the runtime-local Portfolio State.
_Avoid_: Portfolio State, Strategy State

**Strategy Engine**:
The component that executes a strategy and turns market context into a trading decision. Strategy Engine is a role inside the architecture, not necessarily a separate crate; Rhai strategy execution is one implementation of this role.
_Avoid_: Trading Engine when only script execution is meant

**Trading Runtime**:
The component that coordinates a trading session across market data, strategy execution, portfolio state, and execution. A Trading Runtime may be used by live trading or backtesting.
_Avoid_: Strategy Engine when portfolio/execution coordination is meant

**Strategy Hook**:
A named strategy function that the runtime may call at a defined point in the trading session. Missing optional hooks use runtime defaults or no-op behavior.
_Avoid_: Callback, magic function

**Strategy Decision**:
The strategy-produced intent for the current Tradable Tick, using explicit direction-aware intents such as HOLD, OPEN_LONG, CLOSE_LONG, OPEN_SHORT, and CLOSE_SHORT. Opening decisions use quantity to mean asset units/contracts, not a balance fraction; a Strategy Decision is interpreted by the Trading Runtime before any portfolio transition occurs.
_Avoid_: Trade Decision, BUY/SELL when the intended position transition is ambiguous, Execution Action, size when a balance fraction is meant

**Warmup Phase**:
The non-trading prefix of a run used to give indicators, compute state, and strategy state enough market history before the first Tradable Tick. During Warmup, market history advances but strategy decisions do not create portfolio transitions.
_Avoid_: Startup delay, manual lookback

**Market Data Source**:
The origin of candles that drive a run. Live trading uses a provider-backed source that fetches new candles over time; backtesting uses a historical source that replays stored candles.
_Avoid_: Engine, Strategy

**Tradable Tick**:
A candle/event that is allowed to produce trading decisions after warmup has completed. Both live trading and backtesting should feed tradable ticks through the same strategy evaluation path.
_Avoid_: Backtest-only tick, Live-only tick

**Primary Timeframe**:
The timeframe whose completed candles trigger tradable ticks for a strategy. In a multi-timeframe strategy, only the Primary Timeframe should create trading decisions at first.
_Avoid_: Main interval, execution interval

**Secondary Timeframe**:
A timeframe used as additional market context by a strategy without independently triggering trading decisions. Secondary Timeframes can provide trend, regime, or confirmation data.
_Avoid_: Trigger timeframe, trading timeframe

**Runtime Asset**:
The single symbol/instrument managed by one Trading Runtime instance. Multi-timeframe data may belong to the same Runtime Asset; multi-asset portfolio coordination is a later concern.
_Avoid_: Portfolio when only one runtime-managed symbol is meant

**Market State**:
The runtime-held market history available to strategy evaluation, potentially across multiple timeframes for the same asset. Market State is distinct from Portfolio State and Strategy State.
_Avoid_: Candle argument when multiple timeframes are meant

**Market View**:
The strategy-facing view of Market State. A Market View exposes the current Primary Timeframe candle by default and can expose Secondary Timeframe candles when requested.
_Avoid_: Context when market data is meant

**Strategy Context**:
The strategy-facing view of runtime session information that is not Market State. Strategy Context may expose Portfolio State, Strategy State access, and session metadata, but it is built by the Trading Runtime rather than by a runner.
_Avoid_: Market View, Runner Context

**Compute State**:
Runtime-held derived data used to avoid recalculating expensive indicators or quantitative features on every tick. Compute State may include indicator caches, feature caches, and incrementally updated analysis results.
_Avoid_: Strategy State, Portfolio State

**Runtime Event**:
An ordered, runner-neutral occurrence emitted by a Trading Runtime during a trading session. Runtime Events describe market input, strategy decisions, portfolio transitions, and diagnostics without including database or reporting concerns.
_Avoid_: DB Event, Backtest Metric

**Batch Compute**:
Offline or research-oriented computation over many candles, symbols, parameters, or synthetic runs. Batch Compute may use CPU parallelism or GPU compute, but it is separate from the live tradable tick path.
_Avoid_: Live Tick Compute, Trading Runtime

## Flagged ambiguities

**Engine** is currently ambiguous. Use **Strategy Engine** for Rhai/script execution and **Trading Runtime** for the higher-level trading session coordinator.

**Run configuration** is not yet fully resolved. The current direction is that market/source/runtime settings live outside the strategy, while the strategy may contribute warmup requirements or optional defaults.

## Example dialogue

**Developer**: Should this state live in the engine?

**Domain expert**: Which state? If it is `context.state(...)`, call it **Strategy State** and keep it near the **Strategy Engine**. If it is balance, equity, position, or trades, call it **Portfolio State** and let the **Trading Runtime** coordinate it.

**Developer**: Can a strategy define optional functions besides `on_tick`?

**Domain expert**: Yes, those are **Strategy Hooks**. Required hooks define the minimum strategy contract; optional hooks let the **Trading Runtime** call into the strategy at additional lifecycle points.
