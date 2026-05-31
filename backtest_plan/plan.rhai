fn plan() {
    plan_result::new()
        .with_title("Smoke test")
        .with_test(
            plan_test::new("AAPL baseline")
                .with_baseline(baseline::run("AAPL", "1d", 10000.0))
        )
}
