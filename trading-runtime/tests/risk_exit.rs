use domain::{Candle, EntryRiskParameters, OpenPosition, PositionSide, Timeframe};
use trading_runtime::{evaluate_risk_exit, RiskExitKind, RiskExitTriggered};

fn position(side: PositionSide, stop_loss: Option<f64>, take_profit: Option<f64>) -> OpenPosition {
    OpenPosition {
        symbol: "BTC-USD".into(),
        side,
        entry_price: 100.0,
        quantity: 2.0,
        entry_time: 1,
        entry_risk: EntryRiskParameters {
            stop_loss,
            take_profit,
        },
    }
}

fn candle(open: f64, high: f64, low: f64, close: f64) -> Candle {
    Candle {
        timestamp: 2,
        symbol: "BTC-USD".into(),
        open,
        high,
        low,
        close,
        volume: 1_000.0,
        timeframe: Timeframe::minutes(1),
    }
}

fn expected(
    side: PositionSide,
    selected: RiskExitKind,
    triggered: Vec<RiskExitKind>,
    exit_price: f64,
) -> Option<RiskExitTriggered> {
    Some(RiskExitTriggered {
        side,
        selected,
        triggered,
        exit_price,
    })
}

#[test]
fn position_without_entry_risk_never_produces_risk_exit() {
    let open_position = position(PositionSide::Long, None, None);

    let result = evaluate_risk_exit(&open_position, &candle(50.0, 150.0, 40.0, 120.0));

    assert_eq!(result, None);
    assert_eq!(open_position, position(PositionSide::Long, None, None));
}

#[test]
fn long_stop_loss_exits_at_open_gap_or_stop_loss_price() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), None),
            &candle(89.0, 100.0, 80.0, 95.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            89.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), None),
            &candle(100.0, 105.0, 90.0, 95.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            90.0,
        ),
    );
}

#[test]
fn stop_loss_open_gap_equality_exits_at_open_price() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), None),
            &candle(90.0, 100.0, 85.0, 95.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            90.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), None),
            &candle(110.0, 115.0, 100.0, 105.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            110.0,
        ),
    );
}

#[test]
fn long_take_profit_exits_at_open_gap_or_take_profit_price() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, None, Some(120.0)),
            &candle(120.0, 125.0, 110.0, 121.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            120.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, None, Some(120.0)),
            &candle(100.0, 120.0, 95.0, 115.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            120.0,
        ),
    );
}

#[test]
fn short_stop_loss_exits_at_open_gap_or_stop_loss_price() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), None),
            &candle(111.0, 120.0, 100.0, 112.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            111.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), None),
            &candle(100.0, 110.0, 95.0, 105.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss],
            110.0,
        ),
    );
}

#[test]
fn short_take_profit_exits_at_open_gap_or_take_profit_price() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, None, Some(80.0)),
            &candle(80.0, 90.0, 75.0, 79.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            80.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, None, Some(80.0)),
            &candle(100.0, 105.0, 80.0, 85.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            80.0,
        ),
    );
}

#[test]
fn intrabar_touching_stop_loss_and_take_profit_selects_stop_loss_and_reports_both() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), Some(120.0)),
            &candle(100.0, 120.0, 90.0, 105.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss, RiskExitKind::TakeProfit],
            90.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), Some(80.0)),
            &candle(100.0, 110.0, 80.0, 95.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::StopLoss,
            vec![RiskExitKind::StopLoss, RiskExitKind::TakeProfit],
            110.0,
        ),
    );
}

#[test]
fn open_gap_trigger_wins_before_intrabar_boundaries() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), Some(120.0)),
            &candle(121.0, 130.0, 80.0, 100.0),
        ),
        expected(
            PositionSide::Long,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            121.0,
        ),
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), Some(80.0)),
            &candle(79.0, 120.0, 70.0, 100.0),
        ),
        expected(
            PositionSide::Short,
            RiskExitKind::TakeProfit,
            vec![RiskExitKind::TakeProfit],
            79.0,
        ),
    );
}

#[test]
fn configured_boundaries_that_are_not_touched_do_not_produce_risk_exit() {
    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Long, Some(90.0), Some(120.0)),
            &candle(100.0, 119.0, 91.0, 105.0),
        ),
        None,
    );

    assert_eq!(
        evaluate_risk_exit(
            &position(PositionSide::Short, Some(110.0), Some(80.0)),
            &candle(100.0, 109.0, 81.0, 95.0),
        ),
        None,
    );
}
