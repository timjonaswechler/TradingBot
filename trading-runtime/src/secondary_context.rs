//! Shared Secondary-Timeframe context availability evaluation.

use crate::{MarketState, SecondaryContextUnavailableReason, SecondaryTimeframeConfig};
use domain::Candle;

pub(crate) fn secondary_context_unavailable_reason(
    market_state: &MarketState,
    primary_candle: &Candle,
    secondary: &SecondaryTimeframeConfig,
) -> Option<SecondaryContextUnavailableReason> {
    let Some(latest_secondary) = market_state.latest_completed_candle(secondary.timeframe) else {
        return Some(SecondaryContextUnavailableReason::Missing);
    };

    let primary_close_time = primary_candle.close_time();
    let latest_secondary_close_time = latest_secondary.close_time();

    if latest_secondary_close_time > primary_close_time {
        return Some(SecondaryContextUnavailableReason::Missing);
    }

    let duration_ms = secondary.timeframe.duration_ms();
    let allowed_until = latest_secondary_close_time
        .saturating_add(duration_ms.saturating_mul(i64::from(secondary.max_missing_candles) + 1));

    (primary_close_time > allowed_until).then_some(SecondaryContextUnavailableReason::Stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{RuntimeConfig, SecondaryTimeframeConfig};
    use domain::{Candle, Timeframe};

    fn candle(timestamp: i64, timeframe: Timeframe) -> Candle {
        Candle {
            timestamp,
            symbol: "BTC-USD".into(),
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 1_000.0,
            timeframe,
        }
    }

    fn market_state_with_secondary(
        primary_timeframe: Timeframe,
        secondary: SecondaryTimeframeConfig,
        secondary_candles: impl IntoIterator<Item = Candle>,
    ) -> MarketState {
        let config =
            RuntimeConfig::with_secondary_configs("BTC-USD", primary_timeframe, [secondary]);
        let mut market_state = MarketState::from_config(&config);
        for candle in secondary_candles {
            assert!(market_state.record_accepted_candle(candle));
        }
        market_state
    }

    #[test]
    fn missing_secondary_context_returns_missing_reason() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::hours(1), 0);
        let market_state =
            market_state_with_secondary(Timeframe::minutes(1), secondary.clone(), []);

        let reason = secondary_context_unavailable_reason(
            &market_state,
            &candle(60_000, Timeframe::minutes(1)),
            &secondary,
        );

        assert_eq!(reason, Some(SecondaryContextUnavailableReason::Missing));
    }

    #[test]
    fn fresh_secondary_context_returns_no_unavailable_reason() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::hours(1), 0);
        let market_state = market_state_with_secondary(
            Timeframe::minutes(1),
            secondary.clone(),
            [candle(0, Timeframe::hours(1))],
        );

        let reason = secondary_context_unavailable_reason(
            &market_state,
            &candle(3_540_000, Timeframe::minutes(1)),
            &secondary,
        );

        assert_eq!(reason, None);
    }

    #[test]
    fn secondary_context_is_not_available_before_its_close_time() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::hours(1), 0);
        let secondary_open = 10 * Timeframe::hours(1).duration_ms();
        let market_state = market_state_with_secondary(
            Timeframe::minutes(1),
            secondary.clone(),
            [candle(secondary_open, Timeframe::hours(1))],
        );

        let before_secondary_close = secondary_context_unavailable_reason(
            &market_state,
            &candle(
                secondary_open + 58 * Timeframe::minutes(1).duration_ms(),
                Timeframe::minutes(1),
            ),
            &secondary,
        );
        let at_secondary_close = secondary_context_unavailable_reason(
            &market_state,
            &candle(
                secondary_open + 59 * Timeframe::minutes(1).duration_ms(),
                Timeframe::minutes(1),
            ),
            &secondary,
        );
        let after_secondary_close = secondary_context_unavailable_reason(
            &market_state,
            &candle(
                secondary_open + 60 * Timeframe::minutes(1).duration_ms(),
                Timeframe::minutes(1),
            ),
            &secondary,
        );

        assert_eq!(
            before_secondary_close,
            Some(SecondaryContextUnavailableReason::Missing)
        );
        assert_eq!(at_secondary_close, None);
        assert_eq!(after_secondary_close, None);
    }

    #[test]
    fn stale_secondary_context_returns_stale_reason_after_allowed_window() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::hours(1), 0);
        let market_state = market_state_with_secondary(
            Timeframe::minutes(1),
            secondary.clone(),
            [candle(0, Timeframe::hours(1))],
        );

        let reason = secondary_context_unavailable_reason(
            &market_state,
            &candle(7_200_000, Timeframe::minutes(1)),
            &secondary,
        );

        assert_eq!(reason, Some(SecondaryContextUnavailableReason::Stale));
    }

    #[test]
    fn max_missing_candles_extends_freshness_by_secondary_durations() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::hours(1), 1);
        let market_state = market_state_with_secondary(
            Timeframe::minutes(1),
            secondary.clone(),
            [candle(0, Timeframe::hours(1))],
        );

        let still_fresh = secondary_context_unavailable_reason(
            &market_state,
            &candle(10_740_000, Timeframe::minutes(1)),
            &secondary,
        );
        let stale = secondary_context_unavailable_reason(
            &market_state,
            &candle(10_800_000, Timeframe::minutes(1)),
            &secondary,
        );

        assert_eq!(still_fresh, None);
        assert_eq!(stale, Some(SecondaryContextUnavailableReason::Stale));
    }

    #[test]
    fn freshness_uses_secondary_duration_not_primary_duration() {
        let secondary = SecondaryTimeframeConfig::required(Timeframe::minutes(1), 1);
        let market_state = market_state_with_secondary(
            Timeframe::hours(1),
            secondary.clone(),
            [candle(60_000, Timeframe::minutes(1))],
        );

        let reason = secondary_context_unavailable_reason(
            &market_state,
            &candle(180_001, Timeframe::hours(1)),
            &secondary,
        );

        assert_eq!(reason, Some(SecondaryContextUnavailableReason::Stale));
    }
}
