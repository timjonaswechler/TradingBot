use domain::{Candle, OpenPosition, PositionRiskBoundaries, PositionSide, Timeframe};
use trading_runtime::{
    ClosedPosition, ExecutionCostModel, ExecutionCostModelError, ExecutionCostModelField,
    ExecutionFill, ExecutionFillSide, ExecutionFillSource, MarketInput, PortfolioState,
    PredeterminedStrategyHandler, RiskExitKind, RuntimeConfig, RuntimeEvent, StrategyDecision,
    TradingRuntime,
};

fn candle(timestamp: i64, close: f64) -> Candle {
    ohlc_candle(timestamp, close, close, close, close)
}

fn ohlc_candle(timestamp: i64, open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        timestamp,
        symbol: "BTC-USD".into(),
        open,
        high,
        low,
        close,
        volume: 1_000.0,
        timeframe: Timeframe::minutes(1),
    }
}

fn open_position(
    side: PositionSide,
    entry_price: f64,
    stop_loss: Option<f64>,
    take_profit: Option<f64>,
) -> OpenPosition {
    OpenPosition {
        symbol: "BTC-USD".into(),
        side,
        entry_price,
        quantity: 2.0,
        entry_time: 1,
        risk_boundaries: PositionRiskBoundaries {
            stop_loss,
            take_profit,
        },
    }
}

fn completed_primary_step(
    runtime: &mut TradingRuntime<PredeterminedStrategyHandler>,
    candle: Candle,
) -> trading_runtime::RuntimeStep {
    runtime
        .on_market_input(MarketInput::CompletedCandle(candle))
        .expect("completed primary candle should be accepted")
}

fn opened_fill(step: &trading_runtime::RuntimeStep) -> ExecutionFill {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::PositionOpened { fill, .. } => Some(*fill),
            _ => None,
        })
        .expect("position-opened event should expose an execution fill")
}

fn closed_event(step: &trading_runtime::RuntimeStep) -> (&ClosedPosition, ExecutionFill) {
    step.events
        .iter()
        .find_map(|event| match event {
            RuntimeEvent::PositionClosed {
                closed_position,
                fill,
                ..
            } => Some((closed_position, *fill)),
            _ => None,
        })
        .expect("position-closed event should expose an execution fill")
}

fn no_cost_fill(
    side: ExecutionFillSide,
    quantity: f64,
    base_execution_price: f64,
) -> ExecutionFill {
    ExecutionFill::simulated_no_cost(side, quantity, base_execution_price)
}

#[test]
fn runtime_config_defaults_to_no_cost_and_accepts_an_explicit_cost_model() {
    let default_config = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1));
    assert_eq!(
        default_config.execution_cost_model(),
        &ExecutionCostModel::no_cost()
    );

    let explicit_no_cost = ExecutionCostModel::try_new(0.0, 0.0, 0.0).unwrap();
    let configured = RuntimeConfig::single_timeframe("BTC-USD", Timeframe::minutes(1))
        .with_execution_cost_model(explicit_no_cost);

    assert_eq!(
        configured.execution_cost_model(),
        &ExecutionCostModel::no_cost()
    );
}

#[test]
fn invalid_cost_model_values_are_rejected_before_runtime_events_exist() {
    for (fixed_fee_per_fill, percent_fee_rate, fixed_spread, field) in [
        (f64::NAN, 0.0, 0.0, ExecutionCostModelField::FixedFeePerFill),
        (
            f64::INFINITY,
            0.0,
            0.0,
            ExecutionCostModelField::FixedFeePerFill,
        ),
        (-0.01, 0.0, 0.0, ExecutionCostModelField::FixedFeePerFill),
        (0.0, f64::NAN, 0.0, ExecutionCostModelField::PercentFeeRate),
        (
            0.0,
            f64::INFINITY,
            0.0,
            ExecutionCostModelField::PercentFeeRate,
        ),
        (0.0, -0.01, 0.0, ExecutionCostModelField::PercentFeeRate),
        (0.0, 0.0, f64::NAN, ExecutionCostModelField::FixedSpread),
        (
            0.0,
            0.0,
            f64::INFINITY,
            ExecutionCostModelField::FixedSpread,
        ),
        (0.0, 0.0, -0.01, ExecutionCostModelField::FixedSpread),
    ] {
        assert_eq!(
            ExecutionCostModel::try_new(fixed_fee_per_fill, percent_fee_rate, fixed_spread),
            Err(ExecutionCostModelError::InvalidValue { field })
        );
    }

    assert!(ExecutionCostModel::try_new(1.0, 0.0025, 0.5).is_ok());
}

#[test]
fn default_no_cost_long_open_and_strategy_exit_expose_simulated_fills_without_changing_accounting()
{
    let entry_candle = candle(1, 100.0);
    let exit_candle = candle(2, 115.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_long(2.0),
            StrategyDecision::close_long(),
        ]),
    );

    let open_step = completed_primary_step(&mut runtime, entry_candle.clone());
    let close_step = completed_primary_step(&mut runtime, exit_candle.clone());

    assert_eq!(
        opened_fill(&open_step),
        no_cost_fill(ExecutionFillSide::Buy, 2.0, entry_candle.close)
    );
    assert_eq!(open_step.portfolio_snapshot.realized_cash_balance, 1_000.0);
    assert_eq!(
        open_step
            .portfolio_snapshot
            .open_position
            .as_ref()
            .map(|position| position.entry_price),
        Some(entry_candle.close)
    );

    let (closed_position, fill) = closed_event(&close_step);
    assert_eq!(
        fill,
        no_cost_fill(ExecutionFillSide::Sell, 2.0, exit_candle.close)
    );
    assert_eq!(closed_position.exit_price, exit_candle.close);
    assert_eq!(closed_position.realized_pnl, 30.0);
    assert_eq!(close_step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(close_step.portfolio_snapshot.current_equity, 1_030.0);
    assert_eq!(close_step.portfolio_snapshot.completed_trade_count, 1);
    assert!(close_step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn default_no_cost_short_open_and_strategy_exit_expose_simulated_fills_without_changing_accounting()
{
    let entry_candle = candle(1, 100.0);
    let exit_candle = candle(2, 85.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([
            StrategyDecision::open_short(2.0),
            StrategyDecision::close_short(),
        ]),
    );

    let open_step = completed_primary_step(&mut runtime, entry_candle.clone());
    let close_step = completed_primary_step(&mut runtime, exit_candle.clone());

    assert_eq!(
        opened_fill(&open_step),
        no_cost_fill(ExecutionFillSide::Sell, 2.0, entry_candle.close)
    );
    assert_eq!(open_step.portfolio_snapshot.realized_cash_balance, 1_000.0);
    assert_eq!(
        open_step
            .portfolio_snapshot
            .open_position
            .as_ref()
            .map(|position| position.entry_price),
        Some(entry_candle.close)
    );

    let (closed_position, fill) = closed_event(&close_step);
    assert_eq!(
        fill,
        no_cost_fill(ExecutionFillSide::Buy, 2.0, exit_candle.close)
    );
    assert_eq!(closed_position.exit_price, exit_candle.close);
    assert_eq!(closed_position.realized_pnl, 30.0);
    assert_eq!(close_step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(close_step.portfolio_snapshot.current_equity, 1_030.0);
    assert_eq!(close_step.portfolio_snapshot.completed_trade_count, 1);
    assert!(close_step.portfolio_snapshot.open_position.is_none());
}

#[test]
fn default_no_cost_risk_exit_exposes_simulated_closing_fill_without_changing_accounting() {
    let exit_candle = ohlc_candle(2, 100.0, 105.0, 90.0, 99.0);
    let position = open_position(PositionSide::Long, 100.0, Some(90.0), None);
    let mut portfolio = PortfolioState::new(1_000.0);
    portfolio.open_position = Some(position);
    let mut runtime = TradingRuntime::new(
        portfolio,
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::close_long()]),
    );

    let step = completed_primary_step(&mut runtime, exit_candle);

    let (closed_position, fill) = closed_event(&step);
    assert_eq!(fill, no_cost_fill(ExecutionFillSide::Sell, 2.0, 90.0));
    assert_eq!(closed_position.exit_price, 90.0);
    assert_eq!(closed_position.realized_pnl, -20.0);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 980.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.events.iter().any(|event| matches!(
        event,
        RuntimeEvent::PositionClosed {
            exit_kind: trading_runtime::ExitKind::RiskExit {
                selected: RiskExitKind::StopLoss,
            },
            ..
        }
    )));
}

#[test]
fn default_no_cost_force_close_exposes_simulated_closing_fill_without_changing_accounting() {
    let entry_candle = candle(1, 100.0);
    let mark_candle = candle(2, 115.0);
    let mut runtime = TradingRuntime::new(
        PortfolioState::new(1_000.0),
        0,
        PredeterminedStrategyHandler::from_decisions([StrategyDecision::open_long(2.0)]),
    );
    completed_primary_step(&mut runtime, entry_candle);

    let step = runtime.force_close(mark_candle.clone(), "shutdown liquidation");

    let (closed_position, fill) = closed_event(&step);
    assert_eq!(
        fill,
        no_cost_fill(ExecutionFillSide::Sell, 2.0, mark_candle.close)
    );
    assert_eq!(closed_position.exit_price, mark_candle.close);
    assert_eq!(closed_position.realized_pnl, 30.0);
    assert_eq!(step.portfolio_snapshot.realized_cash_balance, 1_030.0);
    assert_eq!(step.portfolio_snapshot.current_equity, 1_030.0);
    assert_eq!(step.portfolio_snapshot.completed_trade_count, 1);
    assert!(step.portfolio_snapshot.open_position.is_none());
    assert_eq!(fill.source, ExecutionFillSource::Simulated);
}
