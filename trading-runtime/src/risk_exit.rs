//! Pure risk-exit selection and gap-aware pricing.

use domain::{Candle, OpenPosition, PositionSide};

/// Runtime-managed hard exit boundary selected from current Position Risk Boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RiskExitKind {
    StopLoss,
    TakeProfit,
}

/// Result of evaluating an open position's Position Risk Boundaries against one candle.
#[derive(Debug, Clone, PartialEq)]
pub struct RiskExitTriggered {
    pub side: PositionSide,
    pub selected: RiskExitKind,
    pub triggered: Vec<RiskExitKind>,
    pub exit_price: f64,
}

/// Evaluate whether one already-open position is closed by its configured Position Risk Boundaries.
///
/// This function is pure: it only reads the open position and candle and never mutates Portfolio
/// State. Open-gap triggers are evaluated before intrabar high/low touches because the candle open
/// is the first known tradable price.
pub fn evaluate_risk_exit(position: &OpenPosition, candle: &Candle) -> Option<RiskExitTriggered> {
    match position.side {
        PositionSide::Long => evaluate_long_risk_exit(position, candle),
        PositionSide::Short => evaluate_short_risk_exit(position, candle),
    }
}

fn evaluate_long_risk_exit(position: &OpenPosition, candle: &Candle) -> Option<RiskExitTriggered> {
    if let Some(stop_loss) = position.risk_boundaries.stop_loss {
        if candle.open <= stop_loss {
            return Some(selected(
                PositionSide::Long,
                RiskExitKind::StopLoss,
                candle.open,
            ));
        }
    }

    if let Some(take_profit) = position.risk_boundaries.take_profit {
        if candle.open >= take_profit {
            return Some(selected(
                PositionSide::Long,
                RiskExitKind::TakeProfit,
                candle.open,
            ));
        }
    }

    let stop_loss_triggered = position
        .risk_boundaries
        .stop_loss
        .map(|stop_loss| candle.low <= stop_loss)
        .unwrap_or(false);
    let take_profit_triggered = position
        .risk_boundaries
        .take_profit
        .map(|take_profit| candle.high >= take_profit)
        .unwrap_or(false);

    intrabar_result(
        PositionSide::Long,
        stop_loss_triggered,
        position.risk_boundaries.stop_loss,
        take_profit_triggered,
        position.risk_boundaries.take_profit,
    )
}

fn evaluate_short_risk_exit(position: &OpenPosition, candle: &Candle) -> Option<RiskExitTriggered> {
    if let Some(stop_loss) = position.risk_boundaries.stop_loss {
        if candle.open >= stop_loss {
            return Some(selected(
                PositionSide::Short,
                RiskExitKind::StopLoss,
                candle.open,
            ));
        }
    }

    if let Some(take_profit) = position.risk_boundaries.take_profit {
        if candle.open <= take_profit {
            return Some(selected(
                PositionSide::Short,
                RiskExitKind::TakeProfit,
                candle.open,
            ));
        }
    }

    let stop_loss_triggered = position
        .risk_boundaries
        .stop_loss
        .map(|stop_loss| candle.high >= stop_loss)
        .unwrap_or(false);
    let take_profit_triggered = position
        .risk_boundaries
        .take_profit
        .map(|take_profit| candle.low <= take_profit)
        .unwrap_or(false);

    intrabar_result(
        PositionSide::Short,
        stop_loss_triggered,
        position.risk_boundaries.stop_loss,
        take_profit_triggered,
        position.risk_boundaries.take_profit,
    )
}

fn intrabar_result(
    side: PositionSide,
    stop_loss_triggered: bool,
    stop_loss: Option<f64>,
    take_profit_triggered: bool,
    take_profit: Option<f64>,
) -> Option<RiskExitTriggered> {
    match (stop_loss_triggered, take_profit_triggered) {
        (true, true) => Some(RiskExitTriggered {
            side,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss, RiskExitKind::TakeProfit],
            exit_price: stop_loss.expect("triggered stop-loss should have a configured price"),
        }),
        (true, false) => Some(RiskExitTriggered {
            side,
            selected: RiskExitKind::StopLoss,
            triggered: vec![RiskExitKind::StopLoss],
            exit_price: stop_loss.expect("triggered stop-loss should have a configured price"),
        }),
        (false, true) => Some(RiskExitTriggered {
            side,
            selected: RiskExitKind::TakeProfit,
            triggered: vec![RiskExitKind::TakeProfit],
            exit_price: take_profit.expect("triggered take-profit should have a configured price"),
        }),
        (false, false) => None,
    }
}

fn selected(side: PositionSide, selected: RiskExitKind, exit_price: f64) -> RiskExitTriggered {
    RiskExitTriggered {
        side,
        selected,
        triggered: vec![selected],
        exit_price,
    }
}
