// Synthetic Market Data Monte Carlo Backtest Plan example.
//
// Run after seeding AAPL daily candles, for example:
//   just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/candle_permutation_monte_carlo.rhai
//
// This uses the #20 candle-permutation procedure: each iteration mutates a copy
// of the historical candle dataset, preserves candle population/invariants, and
// reruns the strategy through the same Runtime-backed backtest path.

fn plan() {
    let dataset = dataset::load(
        "AAPL",
        time("2021-01-04"),
        time("2022-01-03"),
    );
    let config = run_config::new().with_balance(10000.0);
    let baseline = baseline::run(dataset, config);
    let synthetic = monte_carlo::candle_permutation(
        baseline,
        monte_carlo_config::new(25, 42),
    );
    let test = plan_test::new("Synthetic Market Data: candle permutation")
        .with_baseline(baseline)
        .with_synthetic(synthetic);

    plan_result::new()
        .with_title("AAPL candle-path robustness")
        .with_test(test)
}
