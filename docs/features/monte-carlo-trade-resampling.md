# PRD: Monte Carlo Trade Resampling Analysis

Source idea: YouTube video `jGhk-uSrtII` about Monte Carlo simulation for validating trading strategies beyond a single backtest equity curve.

## Problem Statement

TradingBot2 can evaluate a strategy against one historical candle path, and existing planning work already points toward synthetic candle-path testing. But a single backtest result is still only one realization of a stochastic process. The same distribution of trade outcomes can produce materially different equity paths when trades arrive in a different order.

As a strategy author, I need to know whether a profitable backtest reflects a robust edge or one lucky ordering of trades. I also need confidence intervals for per-trade expected value, terminal equity, max drawdown, max run-up, and probability of ruin before trusting a strategy in live trading.

The video describes three Monte Carlo approaches that help answer this:

1. Basic trade-return reshuffling.
2. Regime-switching trade-return resampling.
3. Parametric trade-return simulation.

TradingBot2 should turn these ideas into a reusable backtest-analysis feature that works from completed trade results and produces report-ready risk distributions.

## Solution

Add a Monte Carlo Trade Resampling Analysis feature for backtest results.

The feature consumes a completed historical backtest and its trade ledger, then generates many synthetic equity paths from the observed trade-return distribution. It does not rerun the Strategy Engine candle by candle. Instead, it treats completed trade returns as the distribution under study and asks: “What alternate equity paths could have happened if these trade outcomes arrived in another order, or if trade outcomes were sampled from a fitted distribution?”

V1 should support basic trade-return reshuffling:

- Extract realized return per completed trade.
- Sample trade returns with replacement from the observed distribution.
- Build synthetic equity paths of the same trade count as the original backtest.
- Repeat for a configurable number of iterations using a deterministic seed.
- Report terminal equity distribution, max drawdown distribution, max run-up distribution, per-trade expected-value confidence intervals, baseline percentile, and probability of loss/ruin.

V2 should support regime-switching resampling:

- Tag each completed trade with a market regime.
- Split trade returns into regime-specific distributions.
- Build a regime transition matrix from the original trade sequence or a longer Market State sample.
- Generate regime-aware synthetic trade sequences that preserve clustering.

V3 should support parametric simulation:

- Fit an explicit statistical distribution to trade returns.
- Sample synthetic returns from the fitted continuous distribution.
- Use it only where the chosen distribution is justified and report fit assumptions clearly.

## User Stories

1. As a strategy author, I want Monte Carlo analysis for a completed backtest, so that I can evaluate robustness beyond one equity curve.
2. As a strategy author, I want synthetic equity paths generated from completed trades, so that I can see path-dependence risk without rerunning candle-by-candle strategy logic.
3. As a backtester user, I want trade returns sampled with replacement, so that many alternate trade orderings can be explored from the same observed outcome distribution.
4. As a backtester user, I want deterministic seeds, so that Monte Carlo reports can be reproduced exactly.
5. As a backtester user, I want configurable iteration counts, so that I can trade off runtime and statistical precision.
6. As a strategy author, I want terminal equity percentiles, so that I can understand best-case, median, and bad-case outcomes.
7. As a strategy author, I want max drawdown percentiles, so that I can evaluate whether the strategy is psychologically and financially survivable.
8. As a strategy author, I want max run-up percentiles, so that I can understand favorable path variation without confusing it with guaranteed upside.
9. As a strategy author, I want a confidence interval for average return per trade, so that I can detect when a strategy’s edge is not statistically distinguishable from zero.
10. As a strategy author, I want the report to call out when the expected-value confidence interval spans zero, so that fragile strategies are not promoted as robust.
11. As a strategy author, I want probability of terminal loss, so that I can see how often alternate paths end below starting capital.
12. As a strategy author, I want probability of ruin or threshold breach, so that I can model account-failure risk.
13. As a report reader, I want the original backtest percentile among synthetic paths, so that I can see whether the baseline result was lucky or unlucky.
14. As a report reader, I want to compare original max drawdown to synthetic max drawdowns, so that I can detect hidden downside risk.
15. As a maintainer, I want synthetic equity paths to be analysis artifacts, so that they are not confused with canonical Portfolio State.
16. As a maintainer, I want this feature to consume completed trade data, so that it remains decoupled from Strategy Engine internals.
17. As a maintainer, I want a small typed result object, so that reports, UI, and future APIs can render Monte Carlo output consistently.
18. As a plan author, I want to call trade-resampling analysis from a backtest plan, so that robustness analysis can be part of a scripted workflow.
19. As a plan author, I want the analysis to fail clearly when there are too few trades, so that meaningless simulations are not reported as evidence.
20. As a plan author, I want minimum-trade warnings, so that tiny backtests are not over-interpreted.
21. As a quant researcher, I want regime-switching resampling, so that trade clustering and regime dependence can be preserved.
22. As a quant researcher, I want trades tagged with regimes, so that calm/trending and volatile/choppy periods can have different return distributions.
23. As a quant researcher, I want a regime transition matrix, so that synthetic paths can move between regime-specific trade distributions with observed probabilities.
24. As a quant researcher, I want support for more than two regimes, so that Bull/Bear/Sideways or custom regimes can be modeled.
25. As a strategy author, I want the regime-aware model to preserve clustering, so that losses or wins that cluster in real conditions also appear in synthetic paths.
26. As a strategy author, I want the basic reshuffling model available even without regime tags, so that every backtest can get a first robustness check.
27. As a maintainer, I want regime-switching resampling to reuse existing or future Market Regime features where possible, so that regime definitions do not diverge.
28. As a quant researcher, I want parametric simulation as an optional advanced mode, so that tail events can be explored when the observed trade sample is incomplete.
29. As a quant researcher, I want parametric assumptions reported, so that users know whether they are sampling from observed trades or an imposed distribution.
30. As a maintainer, I want parametric simulation to be clearly labeled as assumption-heavy, so that users do not mistake fitted tails for observed evidence.
31. As a report reader, I want visual-friendly summary data, so that future UI charts can render fan charts, histograms, and percentile bands.
32. As a CLI user, I want a Markdown summary, so that Monte Carlo results are readable in terminal output and saved reports.
33. As a future UI user, I want full synthetic path storage to be optional, so that the UI can render sample paths without forcing huge report payloads.
34. As a maintainer, I want memory usage bounded by configuration, so that large iteration counts do not accidentally store millions of full paths.
35. As a maintainer, I want reduced per-iteration metrics stored by default, so that reports stay small and fast.
36. As a strategy author, I want the analysis to distinguish between trade-count risk and time-in-market risk, so that trade resampling is not mistaken for candle-path simulation.
37. As a maintainer, I want documentation contrasting trade resampling with candle permutation, so that users choose the correct Monte Carlo method.
38. As a maintainer, I want no database writes during the analysis, so that Monte Carlo remains a pure backtest/reporting operation.
39. As a maintainer, I want the same input to produce the same output, so that CI snapshots and report comparisons remain stable.
40. As a user, I want the report to say that Monte Carlo estimates risk rather than proving profitability, so that expectations stay realistic.

## Implementation Decisions

- This feature is a backtest-analysis module, not a Trading Runtime execution feature. It consumes completed historical backtest results and produces statistical summaries.
- Synthetic equity paths produced here are analysis artifacts. They are not Portfolio State and must not be treated as live or simulated account truth.
- The initial input contract should require a completed trade ledger with realized trade return, entry timestamp, exit timestamp, and enough metadata to preserve ordering.
- V1 samples per-trade returns with replacement from the observed trade-return distribution.
- V1 synthetic paths should use the same number of trades as the baseline backtest unless the configuration explicitly asks for another horizon.
- The starting equity must be explicit. Synthetic equity paths compound sampled trade returns from that starting equity.
- A deterministic random-number generator and base seed are required for reproducibility.
- The module should return reduced per-iteration metrics by default: final equity, max drawdown, max run-up, mean return per trade, loss/ruin flags, and optionally a small sampled path for visualization.
- Full synthetic path storage should be optional and bounded by configuration.
- The report should include percentile bands such as p1, p5, p25, p50, p75, p95, and p99 for terminal equity and max drawdown.
- The report should include the baseline backtest’s percentile inside each synthetic distribution.
- The report should include a confidence interval for average return per trade and explicitly flag when it spans zero.
- The report should include probability of terminal loss and probability of hitting a configurable ruin threshold.
- The feature should fail or warn when the completed-trade sample is too small. A tiny trade sample can be resampled mechanically but should not be presented as strong statistical evidence.
- Regime-switching resampling is a separate mode after basic reshuffling. It requires a regime tag on each completed trade.
- Regime tags may come from a future Market Regime feature, a plan-provided classifier, or a backtest annotation step. The Monte Carlo module should consume tags rather than own all regime-classification logic.
- Regime-switching mode splits trade returns into one distribution per regime.
- Regime-switching mode builds a transition matrix over the ordered sequence of trade regime tags.
- Regime-switching mode samples the next regime according to the current regime’s transition probabilities, then samples a trade return from that regime’s return distribution.
- Empty or sparse regime distributions must be handled explicitly with clear errors or documented fallback smoothing.
- Parametric simulation is an advanced mode and should not be the default. It imposes distributional assumptions that may not fit real strategy outcomes.
- Parametric mode must report the chosen distribution family, fitted parameters, fit quality where available, and warnings about stops/take-profits truncating observed tails.
- This feature complements, rather than replaces, candle-path Monte Carlo. Candle permutation reruns the strategy on synthetic candle paths; trade resampling analyzes the distribution of completed trade outcomes.
- Existing backtest-plan work currently treats trade-PnL resampling as out of scope. This PRD should be implemented as a later extension once the backtest result and trade ledger boundaries are stable.
- The plan/runtime API should expose explicit host functions rather than a string-driven umbrella method. For example: basic trade reshuffling, regime-switching resampling, and parametric simulation should be separate calls.
- The module should be deterministic, pure, and easy to test in isolation.

## Testing Decisions

- Good tests should assert external statistical behavior, deterministic reproducibility, validation errors, and rendered summaries. They should not assert private RNG internals or cache layout.
- Basic reshuffling tests should verify sampling with replacement, fixed trade count, deterministic seeds, and compounding from starting equity.
- Metric tests should verify final equity, max drawdown, max run-up, mean return per trade, terminal-loss flags, and ruin-threshold flags on known synthetic paths.
- Percentile tests should verify p1/p5/p50/p95/p99 calculations and baseline percentile calculations.
- Confidence-interval tests should verify that per-trade expected-value intervals are computed correctly and that intervals spanning zero are flagged.
- Validation tests should cover no trades, too few trades, NaN returns, infinite returns, invalid starting equity, invalid iteration counts, and invalid ruin thresholds.
- Reproducibility tests should prove that the same seed and input produce identical reports.
- Report tests should prove that Markdown output includes configuration, warnings, percentile tables, baseline comparison, and edge-confidence warnings.
- Regime-switching tests should verify regime-specific distributions, transition-count construction, transition-matrix normalization, deterministic regime-path sampling, and sparse-regime errors.
- Regime-switching tests should include more than two regimes to avoid hard-coding calm/volatile assumptions.
- Parametric tests should verify parameter fitting on simple known distributions, deterministic sampling with a seed, assumption reporting, and invalid-fit errors.
- Memory-bound tests should prove that reduced metrics can be stored without retaining every full synthetic path.
- Integration tests should prove that a completed backtest result can flow into Monte Carlo analysis without rerunning the Strategy Engine.
- Plan integration tests should prove that scripted backtest workflows can include trade-resampling analysis and render the result in a stable report.

## Out of Scope

- Live trading execution changes.
- Automatic strategy approval or rejection.
- Claiming that Monte Carlo proves future profitability.
- Candle-level synthetic data generation in this feature.
- Rerunning the Strategy Engine on synthetic candle paths.
- Portfolio optimization across multiple Runtime Assets.
- Prop-firm challenge modeling in V1.
- Derivatives pricing in V1.
- Database persistence of every synthetic path.
- UI fan charts or histograms in the first implementation slice.

## Further Notes

This feature should be documented as a fast robustness analysis over completed trades. It answers a different question than candle-path Monte Carlo:

- Candle-path Monte Carlo asks: “What if the market candle path had been different and the strategy reacted to that path?”
- Trade-resampling Monte Carlo asks: “Given the observed distribution of completed trade outcomes, what alternate equity paths could have occurred due to path dependence?”

The video strongly emphasizes that a profitable backtest is not enough. TradingBot2 can encode that lesson by making Monte Carlo robustness reports a normal part of strategy validation.

Suggested first implementation slice:

1. Ensure backtest results expose a stable completed-trade ledger with realized return per trade.
2. Implement basic trade-return reshuffling with deterministic seeds.
3. Compute reduced per-iteration metrics and percentile summaries.
4. Add confidence intervals for average return per trade.
5. Render a Markdown Monte Carlo robustness report.
6. Add warnings for small samples and confidence intervals spanning zero.

Suggested later slices:

1. Add regime tags to completed trades.
2. Implement regime-switching trade resampling.
3. Add optional storage of sampled synthetic paths for visualization.
4. Add parametric simulation with explicit distribution assumptions.
5. Add prop-firm challenge / account-threshold modeling as a separate feature built on the same Monte Carlo core.
