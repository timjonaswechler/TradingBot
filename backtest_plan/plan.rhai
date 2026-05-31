fn plan() {
    let dataset = dataset::load(
        "AAPL",
        timeframe("1d"),
        time("2020-01-01"),
        time("2021-01-01"),
    );
    let config = run_config::new().with_balance(10000.0);
    let baseline = baseline::run(dataset, config);
    let test = plan_test::new("AAPL baseline").with_baseline(baseline);

    plan_result::new()
        .with_title("Smoke test")
        .with_test(test)
}
