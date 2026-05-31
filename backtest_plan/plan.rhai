// Baseline-only Backtest Plan example.
//
// Run after seeding AAPL daily candles, for example:
//   just backtest --strategy strategies/sma_cross.rhai --plan backtest_plan/plan.rhai
//
// The plan names the Runtime Asset and visible Primary window. The strategy
// declares Primary/Secondary Timeframes; the backtester loads the hidden Runtime
// WarmupPlan prefix before START automatically.

fn plan() {
    let dataset = dataset::load(
        "AAPL",
        time("2021-01-04"),
        time("2022-01-03"),
    );
    let config = run_config::new().with_balance(10000.0);
    let baseline = baseline::run(dataset, config);
    let test = plan_test::new("Baseline: AAPL 1d 2021")
        .with_baseline(baseline);

    plan_result::new()
        .with_title("AAPL baseline Backtest Plan")
        .with_test(test)
}
