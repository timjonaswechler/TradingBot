# TradingBot Context

This file documents the domain language used by TradingBot2 contributors and strategy authors. It is a glossary, not an implementation plan.

## Language

**Strategy State**:
Session-local memory that belongs to one running strategy and persists between Strategy Ticks. Strategy State is visible through the Strategy Context and is distinct from Market State, Compute State, and Portfolio State.
_Avoid_: State, Engine State, Portfolio State, Market State

**Portfolio State**:
The canonical trading state of a live or simulated trading session: realized cash balance, open position, and completed trade count. Equity is derived from Portfolio State and current market prices rather than treated as independent account truth; a run may start from an initial completed trade count and then continue counting completed trades from there.
_Avoid_: Strategy State, Context State, External Account Snapshot

**Open Position**:
An active long or short market exposure for one Runtime Asset, described by its side, entry price, quantity, entry time, and optional Entry Risk Parameters. An Open Position is part of Portfolio State; its profit/loss, equity impact, and lifecycle are Portfolio Transition semantics.
_Avoid_: DB Position, Portfolio Snapshot, size when quantity is meant

**Closed Position**:
The record that an Open Position has been closed, including the closed exposure, exit price, exit time, and realized PnL produced by the Portfolio Transition. A Closed Position is distinct from a backtest report row, DB trade row, broker order, or broker fill.
_Avoid_: Trade when reporting, persistence, orders, or fills are meant

**Runtime Portfolio Snapshot**:
A point-in-time view of one Trading Runtime's Portfolio State for a runtime step. It includes cash balance, open position, completed trade count, and current equity derived from the current mark price.
_Avoid_: External Account Snapshot, Account Balance

**External Account Snapshot**:
A point-in-time view of account resources outside one Trading Runtime, such as available cash, buying power, margin, broker-visible positions, open orders, or external exposure reported by a broker/trading provider or account adapter. An External Account Snapshot may inform live strategy execution, but it is not the runtime-local Portfolio State.
_Avoid_: Portfolio State, Strategy State, Provider State

**Strategy Engine**:
The component that executes a strategy and turns market context into a trading decision. Strategy Engine is a role inside the architecture, not necessarily a separate crate; Rhai strategy execution is one implementation of this role.
_Avoid_: Trading Engine when only script execution is meant

**Trading Runtime**:
The component that coordinates a trading session across market data, strategy execution, portfolio state, and execution. A Trading Runtime may be used by live trading or backtesting.
_Avoid_: Strategy Engine when portfolio/execution coordination is meant

**Runtime Session**:
A single running Trading Runtime instance bound to one strategy, one Runtime Asset, one runtime-local Portfolio State, one Market State, one Strategy State, and one ordered Runtime Event stream. A Runtime Session can be hosted by a Live Runner Session or a Backtest Session.
_Avoid_: Runner Session, Live Runner Session, Backtest Session, Strategy State

**Runner Session**:
A runner-owned execution context that hosts one or more Runtime Sessions and owns runner concerns such as market data source, IO adapters, timing or replay orchestration, shutdown policy, and reporting policy.
_Avoid_: Runtime Session, Trading Runtime, Strategy State

**Live Runner Session**:
A Runner Session for live trading. In the first model, a Live Runner Session hosts exactly one active Runtime Session while it coordinates live market input, live IO, and runner shutdown behavior.
_Avoid_: Runtime Session, Backtest Session, Market Session

**Backtest Session**:
A Runner Session for historical replay or research. A Backtest Session is the concrete execution of a direct backtest or Backtest Plan and may coordinate multiple Runtime Sessions for baselines, variants, or Synthetic Market Data comparisons, but it does not own Strategy Decisions or Portfolio Transitions.
_Avoid_: Backtest Plan, Runtime Session, Trading Runtime

**Backtest Plan**:
An operator-authored research workflow definition that assembles one or more Runtime-backed historical runs and comparisons into a structured report result. A Backtest Plan orchestrates datasets, run configuration, and research procedures, but it does not own Strategy Decisions or Portfolio Transitions and is distinct from the Backtest Session that executes it.
_Avoid_: Backtest Session, Trading Engine, Strategy Script

**Strategy Hook**:
A named strategy function that the runtime may call at a defined point in the trading session. Missing optional hooks use runtime defaults or no-op behavior.
_Avoid_: Callback, magic function

**Strategy Configuration**:
Strategy-declared runtime requirements that the Trading Runtime can inspect before Strategy Ticks, including the Primary Timeframe, Secondary-Timeframe requirements, and minimum warmup. Strategy Configuration defines the timeframe contract a strategy expects, but does not choose the Runtime Asset, live/backtest mode, market data source, or Portfolio State.
_Avoid_: Run Configuration, Strategy State

**Strategy Identity**:
An operator-owned identifier for a strategy across live or paper sessions, used to associate runtime-local persistence records with the intended strategy. Strategy Identity is distinct from a strategy file path, source-code hash, Strategy Configuration, and Strategy State.
_Avoid_: Strategy File, Strategy Configuration, Strategy State

**Run Configuration**:
Operator- or runner-owned configuration for a Trading Runtime, including the Runtime Asset, mode/source choices, initial portfolio inputs, and runner policies. Run Configuration binds a strategy's timeframe contract to a concrete asset/source, but does not independently choose Primary or Secondary Timeframes.
_Avoid_: Strategy Configuration, Strategy State

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

**Simulated Execution**:
Runtime-owned execution that applies Execution Planning and Portfolio Transitions without sending broker orders. Simulated Execution can be used by historical Backtest Sessions and live Paper Trading sessions.
_Avoid_: Backtester execution, daemon paper executor, fake strategy

**Paper Trading**:
A Live Runner Session that uses Simulated Execution instead of broker execution while still consuming live-style market input and runner policies. Paper Trading is distinct from a Backtest Session, even though both may share the same simulated Portfolio Transition semantics.
_Avoid_: Backtest Session, historical replay

**Portfolio Transition**:
A change to Portfolio State, such as opening a position, closing a position, or applying a Risk Exit. Portfolio Transitions are owned by the Trading Runtime in both live and simulated runs.
_Avoid_: DB update, backtest metric

**Warmup Requirement**:
The amount of market history a Trading Runtime needs before Strategy Ticks are allowed. In multi-timeframe runs, Warmup Requirements are understood per configured timeframe; the first model may resolve the same requirement for every configured timeframe. The Trading Runtime determines the Warmup Requirement; runners are responsible for fetching and supplying the required market data.
_Avoid_: Runner warmup policy, arbitrary startup delay

**Warmup Phase**:
The strategy-gating prefix of a run used to give indicators and compute state enough market history before the first Tradable Candle. In multi-timeframe runs, the Warmup Phase continues until the Primary Timeframe and every configured Secondary Timeframe have satisfied the Warmup Requirement. During Warmup, strategies are not evaluated and Strategy Decisions or Portfolio Transitions are not produced.
_Avoid_: Startup delay, manual lookback

**Warmup Input**:
Market data supplied to satisfy a Warmup Requirement for the Primary Timeframe or a configured Secondary Timeframe. Warmup Input rebuilds market history and compute state, but must not evaluate strategies or create Risk Exits, Strategy Ticks, or Portfolio Transitions.
_Avoid_: Tradable Candle, active market input

**Tradable Candle**:
A Primary Timeframe candle supplied after Warmup Input is complete and allowed to enter the active trading path. A Tradable Candle may create a Risk Exit or, if no Risk Exit occurs, a Strategy Tick.
_Avoid_: Warmup Input, historical preload

**Market Data Source**:
The origin of candles that drive a run. Live trading uses a provider-backed source that fetches new candles over time; backtesting uses a historical source that replays stored candles.
_Avoid_: Engine, Strategy

**Market Session**:
The market-data calendar or trading-hours context used to decide which candle intervals belong to a visible market session, such as regular trading hours, extended hours, holidays, or exchange session boundaries. A Market Session can affect provider fetching, dataset loading, and candle interpretation, but it is distinct from a Runtime Session, Runner Session, and Strategy State.
_Avoid_: Runtime Session, Runner Session, Strategy State

**Synthetic Market Data**:
Historical-derived candle data transformed for robustness testing before it is fed to a backtest. Synthetic Market Data may reorder, perturb, or regenerate candles, but the Trading Runtime treats it as ordinary market input.
_Avoid_: Runtime Mutation, Strategy Mutation

**Candle Timestamp**:
The timestamp that identifies a completed candle by the open/start boundary of its interval. A Candle is still completed market data; when runtime logic needs the close/end boundary, it derives Candle Close Time from the Candle Timestamp plus the candle's Timeframe duration.
_Avoid_: Candle Close Time, provider raw timestamp when runtime semantics are meant

**Candle Close Time**:
The derived close/end boundary of a completed candle interval, computed from the Candle Timestamp plus the candle's Timeframe duration. Use Candle Close Time only when logic needs to reason about when the interval became complete, not as the canonical candle identifier.
_Avoid_: Candle Timestamp when the open/start boundary is meant

**Candle Volume**:
The traded market quantity reported for one completed candle interval. Candle Volume is Market Data and may be transformed with a Synthetic Market Data candle payload, but it is not Strategy Decision quantity, Position size, or Portfolio State.
_Avoid_: Position size, Strategy Decision quantity, trade count

**Strategy Tick**:
A Tradable Candle on which the strategy is actually evaluated. Risk Exits can close a position on a Tradable Candle without producing a Strategy Tick. A Tradable Candle can also fail to produce a Strategy Tick when required market context is unavailable.
_Avoid_: Tradable Candle when the distinction from strategy evaluation matters, Tradable Tick

**Primary Timeframe**:
The strategy-declared timeframe whose completed candles can become Tradable Candles for a strategy. In a multi-timeframe strategy, only the Primary Timeframe should create trading decisions at first.
_Avoid_: Main interval, execution interval

**Secondary Timeframe**:
A timeframe used as additional market context by a strategy without independently triggering trading decisions. Secondary Timeframes can provide trend, regime, or confirmation data. After Warmup, a strategy may read the latest known completed Secondary-Timeframe candle; the Secondary Timeframe does not need a fresh candle for every Primary-Timeframe Strategy Tick. Missing or stale Secondary-Timeframe context is interpreted through that timeframe's Secondary Readiness Policy.
_Avoid_: Trigger timeframe, trading timeframe

**Secondary Readiness Policy**:
Whether a configured Secondary Timeframe is required or optional for Strategy Ticks after Warmup. A required Secondary Timeframe with unavailable context can block Strategy Ticks; an optional Secondary Timeframe with unavailable context leaves Strategy Ticks allowed. Secondary freshness is evaluated against the latest known completed Secondary-Timeframe candle using that timeframe's expected candle duration plus an allowed missing-candle tolerance.
_Avoid_: Indicator warmup, runner health check

**Required Secondary Timeframe**:
A Secondary Timeframe that must have valid context before a Strategy Tick may run. If a required Secondary Timeframe is unavailable, the Primary-Timeframe candle is still kept in Market State and Risk Exits may still be evaluated, but the strategy is not evaluated for that candle.
_Avoid_: Tradable timeframe, Primary Timeframe

**Optional Secondary Timeframe**:
A Secondary Timeframe that may enrich a Strategy Tick but is not required for the strategy to run. If an optional Secondary Timeframe is unavailable, the Strategy Tick may still run and Market View access for that timeframe returns no value.
_Avoid_: Required Secondary Timeframe

**Runtime Asset**:
The single symbol/instrument managed by one Trading Runtime instance. Multi-timeframe data may belong to the same Runtime Asset; multi-asset portfolio coordination is a later concern.
_Avoid_: Portfolio when only one runtime-managed symbol is meant

**Market State**:
The runtime-held market history available to strategy evaluation, potentially across multiple configured timeframes for the same asset. Market State is distinct from Portfolio State and Strategy State. Accepted market input is stored even when it does not produce a Strategy Tick, so Market State remains a market-data history rather than a history of evaluated strategy ticks. Market input for an unknown timeframe is an invariant violation between runner configuration and runtime configuration; it should not be stored as Market State.
_Avoid_: Candle argument when multiple timeframes are meant

**Market View**:
The strategy-facing view of Market State and market-derived structure outputs. A Market View exposes the current Primary Timeframe candle by default, can expose latest known completed Secondary-Timeframe candles when requested, and may expose anchored/structure-aware outputs derived from market history. Market View candle histories keep the existing strategy-facing convention that index 1 is the newest visible candle and higher indexes move backward in that timeframe. If an optional Secondary Timeframe is unavailable, the corresponding Market View access returns no value; if a required Secondary Timeframe is unavailable, the Strategy Tick is blocked before strategy evaluation.
_Avoid_: Context when market data is meant

**Market Structure Point**:
A market-derived point that describes chart structure, such as a confirmed pivot high, pivot low, swing high, or swing low. Market Structure Points are observations about Market State; they are not Strategy Decisions, Portfolio State, or Strategy State.
_Avoid_: Position, Strategy State, Portfolio State

**Structure Anchor**:
A Market Structure Point selected for building or evaluating a higher-level structure object. A Structure Anchor is the selected input point, not the whole structure model and not necessarily a manually drawn chart object.
_Avoid_: Market Structure when only the selected point is meant

**Structure Object**:
A market-derived construct built from one or more Structure Anchors or Market Structure Points, such as a trendline, Fibonacci retracement, or support/resistance level. A Structure Object can be active, invalidated, replaced, or historical, but it is distinct from Strategy State and Portfolio State.
_Avoid_: Indicator when the object has anchor/structure lifecycle semantics

**Structure Annotation**:
An explanatory record that a Market Structure Point, Structure Anchor, or Structure Object was detected, selected, created, touched, invalidated, or replaced. Structure Annotations support later charting, debugging, and backtest explanation; they are not by themselves trading decisions or portfolio transitions.
_Avoid_: Runtime Event when only chart/explanation markup is meant, Strategy Decision

**Strategy Context**:
The strategy-facing view of runtime session information that is not Market State. Strategy Context may expose Portfolio State, Strategy State access, and session metadata, but it is built by the Trading Runtime rather than by a runner.
_Avoid_: Market View, Runner Context

**Compute State**:
Runtime-held derived data used to avoid recalculating expensive indicators or quantitative features on every tick. Compute State may include indicator caches, feature caches, and incrementally updated analysis results.
_Avoid_: Strategy State, Portfolio State

**Protective Runner Shutdown**:
A runner policy that stops a live run because required market context has been unavailable repeatedly or for too long. If an open position exists, the runner should request a Force Close through the Trading Runtime with the best available Primary-Timeframe mark candle before stopping. A Protective Runner Shutdown is a response to data/session integrity failure, not a Strategy Exit, and is separate from normal user-requested shutdown policy such as `liquidate_on_shutdown`.
_Avoid_: Strategy Exit, Risk Exit, automatic secondary-data trade

**Runtime Input Error**:
A runtime-boundary error for market input that violates runner/runtime invariants, such as supplying a candle for an unknown timeframe. Runtime Input Errors are distinct from normal Runtime Events and from strategy-facing missing-data cases.
_Avoid_: Strategy Error, Diagnostic Runtime Event

**Runtime Event**:
An ordered, runner-neutral occurrence emitted by a Trading Runtime during a trading session. Runtime Events describe market input, tradable-candle handling, strategy decisions, blocked Strategy Ticks, portfolio transitions, and diagnostics without including database or reporting concerns. Event names should distinguish Tradable Candles from Strategy Ticks rather than using "Tradable Tick" ambiguously.
_Avoid_: DB Event, Backtest Metric, Event Store Record

**Paper Trading Persistence Adapter**:
A Live Runner adapter that records selected runtime-local Portfolio Transitions from Paper Trading into external storage. It projects Trading Runtime output into persistence records for a simulated live session and is distinct from Real-Money broker/account truth.
_Avoid_: Runtime Event Store, Trading Runtime Persistence, Real-Money Broker Fill, Portfolio State

**Batch Compute**:
Offline or research-oriented computation over many candles, symbols, parameters, or synthetic runs. Batch Compute may use CPU parallelism or GPU compute, but it is separate from the live tradable tick path.
_Avoid_: Live Tick Compute, Trading Runtime

## Flagged ambiguities

**Engine** is currently ambiguous. Use **Strategy Engine** for Rhai/script execution and **Trading Runtime** for the higher-level trading session coordinator.

**Session** is currently ambiguous. Use **Runtime Session** for one running Trading Runtime instance, **Live Runner Session** for the live runner's execution context, **Backtest Session** for a concrete historical replay or research execution, and **Market Session** for trading-hours/calendar context.

**Run configuration** owns concrete runner/session settings such as Runtime Asset, mode/source choices, and portfolio inputs. **Strategy Configuration** owns the strategy timeframe contract: the Primary Timeframe plus any Secondary-Timeframe requirements/defaults. Avoid defining timeframes independently in both places.

## Example dialogue

**Developer**: Should this state live in the engine?

**Domain expert**: Which state? If it is `context.state(...)`, call it **Strategy State** and keep it near the **Strategy Engine**. If it is balance, equity, position, or trades, call it **Portfolio State** and let the **Trading Runtime** coordinate it.

**Developer**: Can a strategy define optional functions besides `on_tick`?

**Domain expert**: Yes, those are **Strategy Hooks**. Required hooks define the minimum strategy contract; optional hooks let the **Trading Runtime** call into the strategy at additional lifecycle points.
