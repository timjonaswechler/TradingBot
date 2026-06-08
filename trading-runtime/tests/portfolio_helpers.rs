use domain::{Candle, PositionSide};
use trading_runtime::{
    MarketInput, PortfolioState, RhaiStrategy, RuntimeEvent, RuntimeStep, StrategyDecision,
    TradingRuntime,
};

fn candle(close: f64, timestamp: i64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".to_string(),
        open: close,
        high: close,
        low: close,
        close,
        volume: 1_000.0,
        timeframe: "1m".parse().expect("valid timeframe"),
    }
}

fn source_returning(body: &str) -> String {
    format!(
        r#"
fn strategy_config() {{
    strategy_config::new().with_primary(timeframe("1m"))
}}

fn on_tick(market, context) {{
    {body}
}}
"#
    )
}

fn run_tick(source: &str, portfolio: PortfolioState) -> RuntimeStep {
    let strategy = RhaiStrategy::load(source).expect("strategy should load");
    let mut runtime = TradingRuntime::new(portfolio, 0, strategy);

    runtime
        .on_market_input(MarketInput::CompletedCandle(candle(105.0, 2)))
        .expect("completed primary candle should be accepted")
}

fn produced_decision(step: &RuntimeStep) -> StrategyDecision {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::StrategyDecisionProduced { decision } => Some(decision.clone()),
            _ => None,
        })
        .expect("step should include a produced strategy decision")
}

fn assert_no_portfolio_transition(step: &RuntimeStep) {
    assert!(!step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::PositionOpened { .. }
            | RuntimeEvent::PositionClosed { .. }
            | RuntimeEvent::PortfolioUpdated { .. }
    )));
}

fn portfolio_with_position(
    side: PositionSide,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> PortfolioState {
    let mut portfolio = PortfolioState::new(1_000.0);
    let entry = candle(100.0, 1);
    match side {
        PositionSide::Long => portfolio
            .open_long_from_flat(&entry, 2.0, stop_loss, take_profit)
            .expect("long position should open"),
        PositionSide::Short => portfolio
            .open_short_from_flat(&entry, 2.0, stop_loss, take_profit)
            .expect("short position should open"),
    }
    portfolio
}

fn rhai_bool(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

#[test]
fn portfolio_helpers_reflect_flat_long_and_short_snapshots() {
    let cases = [
        (
            "flat",
            PortfolioState::new(1_000.0),
            true,
            false,
            false,
            false,
            None,
        ),
        (
            "long",
            portfolio_with_position(PositionSide::Long, None, None),
            false,
            true,
            true,
            false,
            Some(PositionSide::Long),
        ),
        (
            "short",
            portfolio_with_position(PositionSide::Short, None, None),
            false,
            true,
            false,
            true,
            Some(PositionSide::Short),
        ),
    ];

    for (
        label,
        portfolio,
        expect_flat,
        expect_has_position,
        expect_long,
        expect_short,
        expected_side_after_tick,
    ) in cases
    {
        let source = source_returning(&format!(
            r#"
let portfolio = context.portfolio;
if portfolio.is_flat() == {expect_flat}
        && portfolio.has_position() == {expect_has_position}
        && portfolio.is_long() == {expect_long}
        && portfolio.is_short() == {expect_short} {{
    decision::hold().with_reason("{label} portfolio helpers ok")
}} else {{
    decision::open_long(1.0).with_reason("{label} portfolio helpers mismatch")
}}
"#,
            expect_flat = rhai_bool(expect_flat),
            expect_has_position = rhai_bool(expect_has_position),
            expect_long = rhai_bool(expect_long),
            expect_short = rhai_bool(expect_short),
        ));

        let step = run_tick(&source, portfolio);
        let decision = produced_decision(&step);

        let expected_reason = format!("{label} portfolio helpers ok");
        assert_eq!(
            decision.reason.as_deref(),
            Some(expected_reason.as_str()),
            "{label}"
        );
        assert_eq!(
            step.portfolio_snapshot
                .open_position
                .as_ref()
                .map(|p| p.side),
            expected_side_after_tick,
            "{label} helpers should not mutate Portfolio State"
        );
        assert_no_portfolio_transition(&step);
    }
}

#[test]
fn position_helpers_reflect_side_and_risk_boundary_presence() {
    let cases = [
        (
            "long with risk",
            portfolio_with_position(PositionSide::Long, Some(90.0), Some(120.0)),
            true,
            false,
            true,
            true,
            PositionSide::Long,
        ),
        (
            "long without risk",
            portfolio_with_position(PositionSide::Long, None, None),
            true,
            false,
            false,
            false,
            PositionSide::Long,
        ),
        (
            "short with risk",
            portfolio_with_position(PositionSide::Short, Some(110.0), Some(80.0)),
            false,
            true,
            true,
            true,
            PositionSide::Short,
        ),
        (
            "short without risk",
            portfolio_with_position(PositionSide::Short, None, None),
            false,
            true,
            false,
            false,
            PositionSide::Short,
        ),
    ];

    for (
        label,
        portfolio,
        expect_long,
        expect_short,
        expect_stop_loss,
        expect_take_profit,
        expected_side_after_tick,
    ) in cases
    {
        let source = source_returning(&format!(
            r#"
let position = context.portfolio.position;
if position != ()
        && position.is_long() == {expect_long}
        && position.is_short() == {expect_short}
        && position.has_stop_loss() == {expect_stop_loss}
        && position.has_take_profit() == {expect_take_profit} {{
    decision::hold().with_reason("{label} position helpers ok")
}} else {{
    decision::close_long().with_reason("{label} position helpers mismatch")
}}
"#,
            expect_long = rhai_bool(expect_long),
            expect_short = rhai_bool(expect_short),
            expect_stop_loss = rhai_bool(expect_stop_loss),
            expect_take_profit = rhai_bool(expect_take_profit),
        ));

        let step = run_tick(&source, portfolio);
        let decision = produced_decision(&step);

        let expected_reason = format!("{label} position helpers ok");
        assert_eq!(
            decision.reason.as_deref(),
            Some(expected_reason.as_str()),
            "{label}"
        );
        assert_eq!(
            step.portfolio_snapshot
                .open_position
                .as_ref()
                .map(|p| p.side),
            Some(expected_side_after_tick),
            "{label} helpers should not mutate Portfolio State"
        );
        assert_no_portfolio_transition(&step);
    }
}

#[test]
fn existing_portfolio_and_position_property_access_remains_supported() {
    let source = source_returning(
        r#"
let portfolio = context.portfolio;
let position = portfolio.position;

let properties_ok = true;
if portfolio.position == () { properties_ok = false; }
if portfolio.equity != 1010.0 { properties_ok = false; }
if portfolio.realized_cash_balance != 1000.0 { properties_ok = false; }
if portfolio.completed_trades != 0 { properties_ok = false; }
if position == () { properties_ok = false; }
if position.side != "long" { properties_ok = false; }
if position.entry_price != 100.0 { properties_ok = false; }
if position.quantity != 2.0 { properties_ok = false; }
if position.entry_time != 1 { properties_ok = false; }
if position.stop_loss != 90.0 { properties_ok = false; }
if position.take_profit != 120.0 { properties_ok = false; }

if properties_ok {
    decision::hold().with_reason("properties ok")
} else {
    decision::close_long().with_reason("property access mismatch")
}
"#,
    );
    let portfolio = portfolio_with_position(PositionSide::Long, Some(90.0), Some(120.0));

    let step = run_tick(&source, portfolio);
    let decision = produced_decision(&step);

    assert_eq!(decision.reason.as_deref(), Some("properties ok"));
    assert_eq!(
        step.portfolio_snapshot
            .open_position
            .as_ref()
            .map(|p| p.side),
        Some(PositionSide::Long),
        "property reads should not mutate Portfolio State"
    );
    assert_no_portfolio_transition(&step);
}
