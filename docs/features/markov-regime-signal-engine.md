# PRD: Markov Regime Signal Engine

Source idea: YouTube video `ZVMTeDBmSrI` about a Markov/hedge-fund-style regime method for trading signals.

## Problem Statement

TradingBot2 strategy authors can express candle-by-candle trading logic, indicators, warmup behavior, and backtests, but there is not yet a first-class quantitative regime model that turns historical Market State into explicit probabilities for the next market regime.

The video describes a workflow used by quantitative traders: classify historical candles into Bull, Bear, and Sideways regimes, count how often the market transitions between regimes, convert those counts into a transition matrix, and derive a probabilistic signal from `P(Bull next) - P(Bear next)`. That is a useful fit for TradingBot2 because it can become a reusable Compute State feature instead of being reimplemented ad hoc in individual Rhai strategies.

The main user problem is: as a strategy author, I want regime probabilities and signal strength to be computed consistently and without lookahead bias, so that my strategy can use quantified Market State instead of subjective chart interpretation.

## Solution

Add a Markov Regime Signal Engine as a deep, testable feature module that can be used by backtesting first and later by live Trading Runtime paths.

V1 should support:

- Rule-based regime classification into `Bull`, `Bear`, and `Sideways`.
- Configurable lookback window, defaulting to 20 completed Primary Timeframe candles.
- Configurable thresholds, defaulting to `+5%` for Bull and `-5%` for Bear over the lookback window.
- Historical state-sequence generation from candles.
- A 3x3 transition matrix from current regime to next regime.
- Row-normalized probabilities for tomorrow/next tick’s regime.
- Persistence/stickiness scores from the matrix diagonal.
- Multi-step forecasts by raising the transition matrix to a requested horizon.
- Signal generation using `P(Bull next) - P(Bear next)`.
- Walk-forward evaluation for backtests so that each Tradable Tick only uses information available before that tick.

The feature should initially expose regime data as a quantitative feature to strategies and reports. It should not automatically place trades by itself. Strategy authors can decide how to combine the regime signal with their existing Strategy Decisions.

## User Stories

1. As a strategy author, I want a reusable Markov regime feature, so that I do not need to reimplement regime math in every Rhai strategy.
2. As a strategy author, I want the market classified as Bull, Bear, or Sideways, so that strategy logic can branch on explicit Market State rather than chart vibes.
3. As a strategy author, I want regime classification to use completed candles only, so that signals do not depend on future data.
4. As a strategy author, I want the lookback window to be configurable, so that I can test whether 20 candles, 50 candles, or another value fits a Runtime Asset better.
5. As a strategy author, I want Bull and Bear thresholds to be configurable, so that regime definitions can be tuned per asset or timeframe.
6. As a backtester user, I want defaults matching the video’s method, so that I can quickly reproduce the original idea.
7. As a backtester user, I want every historical candle to receive a regime label after enough warmup history exists, so that transition counts are deterministic.
8. As a backtester user, I want a clear Warmup Phase requirement, so that the first Tradable Tick has enough Market State for regime computation.
9. As a maintainer, I want regime labels to be represented as a small domain enum, so that invalid string states cannot leak through internal APIs.
10. As a strategy author, I want the current regime exposed to my strategy, so that I can avoid long entries in Bear regimes or short entries in Bull regimes.
11. As a strategy author, I want next-regime probabilities exposed to my strategy, so that I can use probabilities instead of binary indicator flags.
12. As a strategy author, I want a single scalar regime signal, so that I can map positive values to long bias and negative values to short bias.
13. As a strategy author, I want the magnitude of the regime signal, so that position sizing rules can react to signal strength.
14. As a risk-aware strategy author, I want Sideways probability exposed separately, so that strategies can choose to reduce exposure in uncertain regimes.
15. As a backtester user, I want the transition matrix included in reports, so that I can inspect whether the model’s probabilities make sense.
16. As a backtester user, I want persistence/stickiness scores for each regime, so that I can see whether Bull, Bear, or Sideways states tend to continue.
17. As a quant researcher, I want multi-step forecasts, so that I can compare one-tick, two-tick, and longer-horizon probability decay.
18. As a quant researcher, I want long-horizon forecasts to be explicitly marked as lower-confidence, so that stationary-distribution convergence is not mistaken for a strong signal.
19. As a maintainer, I want transition-matrix math isolated behind a small interface, so that it can be tested independently from the Strategy Engine and Trading Runtime.
20. As a maintainer, I want signal generation isolated from classification, so that future signal formulas can be added without rewriting regime labeling.
21. As a backtester user, I want walk-forward backtesting, so that the model at each Tradable Tick is trained only on earlier candles.
22. As a maintainer, I want lookahead bias tests, so that accidental use of future candles is caught before strategy results are trusted.
23. As a live-trading user, I want the same regime calculation to be usable in a live Trading Runtime later, so that backtested and live semantics do not fork.
24. As a strategy author, I want regime features to live in Compute State, so that expensive transition counts are not recalculated from scratch when incremental updates are enough.
25. As a strategy author, I want the feature to work on the Primary Timeframe first, so that the first version has clear Tradable Tick semantics.
26. As a future multi-timeframe strategy author, I want the design to allow Secondary Timeframe regime context later, so that higher-timeframe confirmation can be added safely.
27. As a maintainer, I want the feature to treat the Runtime Asset as the modeling boundary in V1, so that multi-asset portfolio coordination remains out of scope.
28. As a report reader, I want regime probabilities rendered in a clear table, so that I can understand what the model believed during a backtest.
29. As a report reader, I want summary statistics over regime signals, so that I can see how often the strategy was in strong Bull, strong Bear, or ambiguous regimes.
30. As a future feature developer, I want Hidden Markov Model support to be a separate extension, so that V1 can ship with simpler rule-based regimes first.
31. As a quant researcher, I want a later Hidden Markov Model mode, so that regimes can be inferred from price behavior rather than only from human-chosen thresholds.
32. As a maintainer, I want HMM output compared against rule-based labels before being used for trading, so that inferred regimes can be validated rather than blindly trusted.
33. As a strategy author, I want the feature to return HOLD/no-bias-friendly values when there is insufficient history, so that early-run behavior is safe and explicit.
34. As a maintainer, I want invalid configurations to fail fast, so that negative lookback windows or impossible thresholds do not produce silent bad signals.
35. As a user, I want documentation explaining that this is a probability feature and not investment advice, so that expectations stay realistic.

## Implementation Decisions

- Build the Markov Regime Signal Engine as a quantitative feature rather than as an autonomous trading strategy. It produces regime data and signal strength; strategies still produce Strategy Decisions.
- Use the project’s existing domain language: the feature consumes Market State, caches derived values in Compute State, and is coordinated by the Trading Runtime or backtester rather than by the Strategy Engine alone.
- V1 models one Runtime Asset on one Primary Timeframe. Secondary Timeframe and multi-asset extensions are future work.
- The V1 regime set is exactly three states: Bull, Bear, and Sideways.
- The default rule-based classifier uses the percentage return over the trailing completed lookback window. `>= +5%` is Bull, `<= -5%` is Bear, and values between are Sideways.
- The default lookback is 20 completed Primary Timeframe candles. The feature should describe this as a candle lookback rather than always as “20 days,” because TradingBot2 can run on non-daily timeframes.
- Regime classification must be based on completed candles only. A signal for the next Tradable Tick must not inspect candles that would not have been available at that moment.
- Transition counting builds a state sequence and counts observed current-state-to-next-state transitions.
- The transition matrix is row-normalized: each row represents the current regime and must sum to 1.0 after probabilities are computed.
- Empty transition rows must be handled explicitly. V1 should either fail with a clear insufficient-history error or use a documented smoothing/default policy. Silent division by zero is not allowed.
- Persistence/stickiness is the diagonal value for each regime: Bull-to-Bull, Bear-to-Bear, and Sideways-to-Sideways.
- Multi-step forecasts are computed by matrix exponentiation. They are useful for analysis, but V1 strategy integration should prioritize one-step forecasts.
- Signal generation uses `bull_probability - bear_probability`. Positive values indicate long bias, negative values indicate short bias, and values close to zero indicate weak or ambiguous directional edge.
- Position sizing is not hard-coded in V1. The feature may expose signal magnitude, but each strategy decides whether and how that maps to quantity or risk.
- Walk-forward backtesting is mandatory for trustworthy evaluation. At each Tradable Tick, the model must be rebuilt or updated using only prior Market State.
- The implementation should favor deep modules with small interfaces: regime classification, transition-matrix math, signal generation, and walk-forward evaluation should be separable.
- Backtesting/reporting should be the first integration target. Live Trading Runtime integration can reuse the same calculation once the runtime boundary is ready.
- The Rhai-facing surface should be small and stable. Strategy authors should be able to read current regime, next-regime probabilities, stickiness scores, and scalar signal without depending on internal Rust structures.
- Hidden Markov Model support is not part of V1. It should be planned as a later model mode that infers regimes from observations and can be compared with the rule-based classifier.
- TradingView/PineScript visualization from the video is out of scope for the Rust implementation, but the report should provide enough matrix data to make a future visualization possible.
- The feature must not write to the database during ordinary backtest execution. It derives data from candles and returns computed outputs.
- The feature should be deterministic: the same candles and configuration must produce the same regime labels, matrix, probabilities, and signals.

## Testing Decisions

- Good tests should assert external behavior: regime labels, matrix probabilities, signal values, walk-forward no-lookahead behavior, and report output. They should not assert private cache layout or internal loop structure.
- Regime-classifier tests should cover Bull, Bear, Sideways, exact threshold boundaries, insufficient history, flat prices, and non-daily timeframes.
- Transition-matrix tests should cover all state-pair counts, row normalization, row sums, empty-row behavior, and deterministic output ordering.
- Stickiness tests should verify that diagonal probabilities are reported for the correct current regimes.
- Matrix-power tests should verify one-step identity behavior, two-step forecasts, longer-horizon convergence behavior, and numerical tolerance.
- Signal-generation tests should verify positive long bias, negative short bias, neutral/near-zero bias, and preservation of Sideways probability.
- Walk-forward tests should prove that changing future candles does not change earlier generated signals.
- Warmup tests should prove that the feature does not emit tradable regime signals before the required lookback and transition history exists.
- Backtester integration tests should prove that adding the regime feature does not fork candle-by-candle execution semantics.
- Strategy-facing tests should prove that Rhai strategies can read regime features and still return normal Strategy Decisions.
- Report tests should prove that matrices, probabilities, signal summaries, and configuration values render clearly and deterministically.
- Configuration tests should reject invalid lookbacks, invalid thresholds, missing required data, NaN values, and impossible probability states.

## Out of Scope

- Automatic trading decisions made directly by the Markov module.
- Portfolio-level allocation across multiple Runtime Assets.
- Secondary Timeframe regime confirmation in V1.
- Hidden Markov Model inference in V1.
- Machine-learning training infrastructure.
- TradingView/PineScript implementation.
- Real-money execution changes.
- Database persistence of regime outputs.
- Optimization or parameter search across thresholds/lookbacks.
- Claiming the method guarantees profit or replaces risk management.

## Further Notes

The video’s core method is valuable because it turns subjective trading language into measurable probabilities. For TradingBot2, the safest version is to make it a reusable Compute State feature that strategies can consult, not a black-box strategy that opens positions by itself.

Terminology note: the video calls the three labels “states.” In TradingBot2 documentation, use **regime** or **Market Regime** for Bull/Bear/Sideways labels to avoid confusion with **Strategy State** and **Portfolio State**.

Suggested first implementation slice:

1. Implement rule-based regime classification and configuration validation.
2. Implement transition-matrix construction and one-step probabilities.
3. Implement `P(Bull) - P(Bear)` signal generation.
4. Add walk-forward backtester integration and no-lookahead tests.
5. Expose read-only regime features to Rhai strategies.
6. Add report output for matrix, stickiness, and signal summaries.

Suggested later slice:

1. Add matrix-power forecasts.
2. Add Secondary Timeframe regime context.
3. Add Hidden Markov Model research mode.
4. Compare HMM-inferred regimes against rule-based regimes before allowing strategy use.
