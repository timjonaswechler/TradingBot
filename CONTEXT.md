# TradingBot Context

This file documents the domain language used by TradingBot2 contributors and strategy authors. It is a glossary, not an implementation plan.

## Language

**Strategy State**:
Memory that belongs to one running strategy and persists between ticks. Strategy State is visible to Rhai strategies through the strategy context and is not the authoritative account or portfolio state.
_Avoid_: State, Engine State

**Portfolio State**:
The canonical trading state of a live or simulated trading session: realized cash balance, open position, and completed trade count. Equity is derived from Portfolio State and current market prices rather than treated as independent account truth; a run may start from an initial completed trade count and then continue counting completed trades from there.
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
The strategy-produced intent for the current Strategy Tick, using explicit direction-aware intents such as HOLD, OPEN_LONG, CLOSE_LONG, OPEN_SHORT, and CLOSE_SHORT. Opening decisions use quantity to mean asset units/contracts, not a balance fraction; a Strategy Decision is interpreted by the Trading Runtime before any portfolio transition occurs.
_Avoid_: Trade Decision, BUY/SELL when the intended position transition is ambiguous, Execution Action, size when a balance fraction is meant

**Entry Risk Parameters**:
Optional stop-loss and take-profit prices attached to an opening Strategy Decision. They opt the resulting open position into runtime-managed hard exits; strategies that want to manage exits themselves can omit them.
_Avoid_: Dynamic Risk Update, Position Patch, soft strategy exit

**Stop-Loss**:
An Entry Risk Parameter that defines the hard protective price at which the runtime may close an open position to limit adverse movement.
_Avoid_: Stop signal, soft stop, close reason

**Take-Profit**:
An Entry Risk Parameter that defines the hard target price at which the runtime may close an open position to capture favorable movement.
_Avoid_: Target signal, soft target, close reason

**Risk Exit**:
A portfolio transition that closes an open position because its Stop-Loss or Take-Profit was reached by market movement. A Risk Exit is distinct from a Strategy Decision to close a position.
_Avoid_: Strategy Exit, manual close

**Strategy Exit**:
A Strategy Decision that closes an open position because the strategy's own logic chose to exit. Strategy Exits are evaluated during Strategy Ticks and are distinct from runtime-managed Risk Exits.
_Avoid_: Risk Exit, hard stop, hard target

**Execution Planning**:
The runtime interpretation step that maps a Strategy Decision and the current Portfolio State to an execution action or an ignored decision. Execution Planning does not by itself change Portfolio State.
_Avoid_: Execution State Machine when no pending/fill states are meant

**Portfolio Transition**:
A change to Portfolio State, such as opening a position, closing a position, or applying a Risk Exit. Portfolio Transitions are owned by the Trading Runtime in both live and simulated runs.
_Avoid_: DB update, backtest metric

**Warmup Requirement**:
The amount of market history a Trading Runtime needs before Strategy Ticks are allowed. The Trading Runtime determines the Warmup Requirement; runners are responsible for fetching and supplying the required market data.
_Avoid_: Runner warmup policy, arbitrary startup delay

**Warmup Phase**:
The strategy-gating prefix of a run used to give indicators, compute state, and strategy state enough market history before the first Tradable Candle. During Warmup, Strategy Decisions and Portfolio Transitions are not produced.
_Avoid_: Startup delay, manual lookback

**Warmup Input**:
Market data supplied to satisfy a Warmup Requirement. Warmup Input rebuilds market history, compute state, and strategy state, but must not create Risk Exits, Strategy Ticks, or Portfolio Transitions.
_Avoid_: Tradable Candle, active market input

**Tradable Candle**:
A Primary Timeframe candle supplied after Warmup Input is complete and allowed to enter the active trading path. A Tradable Candle may create a Risk Exit or, if no Risk Exit occurs, a Strategy Tick.
_Avoid_: Warmup Input, historical preload

**Market Data Source**:
The origin of candles that drive a run. Live trading uses a provider-backed source that fetches new candles over time; backtesting uses a historical source that replays stored candles.
_Avoid_: Engine, Strategy

**Strategy Tick**:
A Tradable Candle on which the strategy is actually evaluated. Risk Exits can close a position on a Tradable Candle without producing a Strategy Tick.
_Avoid_: Tradable Candle when the distinction from strategy evaluation matters, Tradable Tick

**Primary Timeframe**:
The timeframe whose completed candles can become Tradable Candles for a strategy. In a multi-timeframe strategy, only the Primary Timeframe should create trading decisions at first.
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
