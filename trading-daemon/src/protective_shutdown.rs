//! Live-runner Protective Runner Shutdown policy.
//!
//! This module stays in `trading-daemon` because Protective Runner Shutdown is a
//! runner-owned safety policy. It consumes runner-neutral `trading-runtime`
//! events and never re-evaluates Secondary readiness itself.

use std::collections::{HashMap, HashSet};

use domain::Timeframe;
use trading_runtime::{BlockedSecondaryContext, RuntimeEvent, RuntimeStep};

use crate::config::ProtectiveShutdownConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectiveShutdownTrigger {
    pub runtime_asset: String,
    pub primary_timeframe: Timeframe,
    pub threshold: u32,
    pub blocked_contexts: Vec<BlockedSecondaryContext>,
    pub counters: Vec<ProtectiveShutdownCounter>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectiveShutdownCounter {
    pub timeframe: Timeframe,
    pub consecutive_blocked_primary_candles: u32,
}

/// Counts consecutive Primary candles blocked by required Secondary context.
#[derive(Debug, Clone)]
pub struct ProtectiveShutdownPolicy {
    runtime_asset: String,
    primary_timeframe: Timeframe,
    enabled: bool,
    threshold: u32,
    counters: HashMap<Timeframe, u32>,
}

impl ProtectiveShutdownPolicy {
    pub fn new(
        runtime_asset: impl Into<String>,
        primary_timeframe: Timeframe,
        config: ProtectiveShutdownConfig,
    ) -> Self {
        Self {
            runtime_asset: runtime_asset.into(),
            primary_timeframe,
            enabled: config.enabled,
            threshold: config.required_secondary_failure_threshold,
            counters: HashMap::new(),
        }
    }

    pub fn observe_step(&mut self, step: &RuntimeStep) -> Option<ProtectiveShutdownTrigger> {
        if !self.enabled {
            return None;
        }

        let blocked_contexts = step.events.iter().find_map(|event| match event {
            RuntimeEvent::StrategyTickBlocked {
                blocked_contexts, ..
            } => Some(blocked_contexts.as_slice()),
            _ => None,
        });

        if let Some(blocked_contexts) = blocked_contexts {
            return self.observe_blocked_contexts(blocked_contexts);
        }

        // A tradable Primary candle that was not blocked breaks the sequence of
        // consecutive blocked Primary candles. This includes Strategy Ticks and
        // Risk Exits, where the runtime intentionally does not evaluate
        // Secondary readiness after the protective exit.
        if step
            .events
            .iter()
            .any(|event| matches!(event, RuntimeEvent::TradableCandleAccepted { .. }))
        {
            self.counters.clear();
        }

        None
    }

    fn observe_blocked_contexts(
        &mut self,
        blocked_contexts: &[BlockedSecondaryContext],
    ) -> Option<ProtectiveShutdownTrigger> {
        let currently_blocked: HashSet<Timeframe> = blocked_contexts
            .iter()
            .map(|context| context.timeframe)
            .collect();

        // Keep counters per required Secondary timeframe consecutive. If a
        // timeframe that failed before is not blocked on this Primary candle,
        // its own failure streak is broken even if a different required
        // Secondary timeframe still blocks the Strategy Tick.
        self.counters
            .retain(|timeframe, _| currently_blocked.contains(timeframe));

        for context in blocked_contexts {
            *self.counters.entry(context.timeframe).or_insert(0) += 1;
        }

        if self.counters.values().any(|count| *count >= self.threshold) {
            Some(ProtectiveShutdownTrigger {
                runtime_asset: self.runtime_asset.clone(),
                primary_timeframe: self.primary_timeframe,
                threshold: self.threshold,
                blocked_contexts: blocked_contexts.to_vec(),
                counters: sorted_counters(&self.counters),
            })
        } else {
            None
        }
    }
}

fn sorted_counters(counters: &HashMap<Timeframe, u32>) -> Vec<ProtectiveShutdownCounter> {
    let mut counters: Vec<_> = counters
        .iter()
        .map(|(timeframe, count)| ProtectiveShutdownCounter {
            timeframe: *timeframe,
            consecutive_blocked_primary_candles: *count,
        })
        .collect();
    counters.sort_by_key(|counter| counter.timeframe.to_string());
    counters
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::{Candle, OpenPosition, PositionRiskBoundaries, PositionSide, Timeframe};
    use trading_runtime::{
        BlockedSecondaryContext, ExecutionFill, ExecutionFillSide, ExitKind, PortfolioState,
        RuntimeEvent, RuntimePortfolioSnapshot, RuntimeStep, SecondaryContextUnavailableReason,
        SecondaryReadiness,
    };

    fn config(enabled: bool, threshold: u32) -> ProtectiveShutdownConfig {
        ProtectiveShutdownConfig {
            enabled,
            required_secondary_failure_threshold: threshold,
        }
    }

    fn policy(threshold: u32) -> ProtectiveShutdownPolicy {
        ProtectiveShutdownPolicy::new("BTC-USD", Timeframe::minutes(1), config(true, threshold))
    }

    fn candle(timeframe: Timeframe) -> Candle {
        Candle {
            timestamp: 1_700_000_000_000,
            symbol: "BTC-USD".into(),
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 1_000.0,
            timeframe,
        }
    }

    fn snapshot(open_position: Option<OpenPosition>) -> RuntimePortfolioSnapshot {
        let mut portfolio = PortfolioState::new(10_000.0);
        portfolio.open_position = open_position;
        portfolio.snapshot(100.0)
    }

    fn fill(side: ExecutionFillSide, quantity: f64, base_execution_price: f64) -> ExecutionFill {
        ExecutionFill::simulated_no_cost(side, quantity, base_execution_price)
    }

    fn blocked_step(timeframes: &[Timeframe]) -> RuntimeStep {
        let primary = candle(Timeframe::minutes(1));
        let blocked_contexts = timeframes
            .iter()
            .map(|timeframe| BlockedSecondaryContext {
                timeframe: *timeframe,
                reason: SecondaryContextUnavailableReason::Missing,
            })
            .collect();

        RuntimeStep::new(
            vec![
                RuntimeEvent::TradableCandleAccepted {
                    candle: primary.clone(),
                },
                RuntimeEvent::StrategyTickBlocked {
                    candle: primary,
                    blocked_contexts,
                },
                RuntimeEvent::TradableCandleCompleted,
            ],
            snapshot(None),
        )
    }

    fn started_step() -> RuntimeStep {
        let primary = candle(Timeframe::minutes(1));
        RuntimeStep::new(
            vec![
                RuntimeEvent::TradableCandleAccepted {
                    candle: primary.clone(),
                },
                RuntimeEvent::StrategyTickStarted { candle: primary },
                RuntimeEvent::StrategyTickCompleted,
                RuntimeEvent::TradableCandleCompleted,
            ],
            snapshot(None),
        )
    }

    fn optional_unavailable_step() -> RuntimeStep {
        let primary = candle(Timeframe::minutes(1));
        RuntimeStep::new(
            vec![
                RuntimeEvent::TradableCandleAccepted {
                    candle: primary.clone(),
                },
                RuntimeEvent::SecondaryContextUnavailable {
                    candle: primary.clone(),
                    timeframe: Timeframe::hours(1),
                    readiness: SecondaryReadiness::Optional,
                    reason: SecondaryContextUnavailableReason::Missing,
                },
                RuntimeEvent::StrategyTickStarted { candle: primary },
                RuntimeEvent::StrategyTickCompleted,
                RuntimeEvent::TradableCandleCompleted,
            ],
            snapshot(None),
        )
    }

    #[test]
    fn repeated_required_secondary_blocks_trigger_at_threshold() {
        let h1 = Timeframe::hours(1);
        let mut policy = policy(2);

        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
        let trigger = policy
            .observe_step(&blocked_step(&[h1]))
            .expect("second consecutive block should trigger shutdown");

        assert_eq!(trigger.runtime_asset, "BTC-USD");
        assert_eq!(trigger.primary_timeframe, Timeframe::minutes(1));
        assert_eq!(trigger.threshold, 2);
        assert_eq!(
            trigger.counters,
            vec![ProtectiveShutdownCounter {
                timeframe: h1,
                consecutive_blocked_primary_candles: 2,
            }]
        );
    }

    #[test]
    fn counters_are_per_required_secondary_timeframe() {
        let h1 = Timeframe::hours(1);
        let h4 = Timeframe::hours(4);
        let mut policy = policy(2);

        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
        assert!(policy.observe_step(&blocked_step(&[h4])).is_none());
        let trigger = policy
            .observe_step(&blocked_step(&[h4]))
            .expect("h4 should trigger after its own second consecutive block");

        assert_eq!(
            trigger.counters,
            vec![ProtectiveShutdownCounter {
                timeframe: h4,
                consecutive_blocked_primary_candles: 2,
            }]
        );
    }

    #[test]
    fn multiple_blocked_secondaries_increment_together_and_any_can_trigger() {
        let h1 = Timeframe::hours(1);
        let h4 = Timeframe::hours(4);
        let mut policy = policy(2);

        assert!(policy.observe_step(&blocked_step(&[h1, h4])).is_none());
        let trigger = policy
            .observe_step(&blocked_step(&[h1, h4]))
            .expect("both counters reaching the threshold should trigger shutdown");

        assert_eq!(
            trigger.counters,
            vec![
                ProtectiveShutdownCounter {
                    timeframe: h1,
                    consecutive_blocked_primary_candles: 2,
                },
                ProtectiveShutdownCounter {
                    timeframe: h4,
                    consecutive_blocked_primary_candles: 2,
                },
            ]
        );
    }

    #[test]
    fn strategy_tick_started_resets_required_secondary_counters() {
        let h1 = Timeframe::hours(1);
        let mut policy = policy(2);

        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
        assert!(policy.observe_step(&started_step()).is_none());
        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
    }

    #[test]
    fn optional_secondary_diagnostics_do_not_increment_or_trigger() {
        let h1 = Timeframe::hours(1);
        let mut policy = policy(1);

        assert!(policy.observe_step(&optional_unavailable_step()).is_none());
        let trigger = policy
            .observe_step(&blocked_step(&[h1]))
            .expect("first required block should still trigger when threshold is one");

        assert_eq!(trigger.blocked_contexts[0].timeframe, h1);
    }

    #[test]
    fn disabled_policy_never_triggers() {
        let h1 = Timeframe::hours(1);
        let mut policy =
            ProtectiveShutdownPolicy::new("BTC-USD", Timeframe::minutes(1), config(false, 1));

        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
    }

    #[test]
    fn risk_exit_primary_step_breaks_block_sequence_without_triggering_shutdown() {
        let h1 = Timeframe::hours(1);
        let primary = candle(Timeframe::minutes(1));
        let position = OpenPosition {
            symbol: "BTC-USD".into(),
            side: PositionSide::Long,
            entry_price: 100.0,
            quantity: 1.0,
            entry_time: 1_699_999_940_000,
            risk_boundaries: PositionRiskBoundaries {
                stop_loss: Some(90.0),
                take_profit: None,
            },
        };
        let mut policy = policy(2);
        let risk_exit_step = RuntimeStep::new(
            vec![
                RuntimeEvent::TradableCandleAccepted {
                    candle: primary.clone(),
                },
                RuntimeEvent::RiskExitTriggered {
                    risk_exit: trading_runtime::RiskExitTriggered {
                        side: PositionSide::Long,
                        selected: trading_runtime::RiskExitKind::StopLoss,
                        triggered: vec![trading_runtime::RiskExitKind::StopLoss],
                        exit_price: 90.0,
                    },
                },
                RuntimeEvent::PositionClosed {
                    closed_position: trading_runtime::ClosedPosition {
                        position,
                        exit_price: 90.0,
                        exit_time: primary.timestamp,
                        realized_pnl: -10.0,
                    },
                    exit_kind: ExitKind::RiskExit {
                        selected: trading_runtime::RiskExitKind::StopLoss,
                    },
                    fill: fill(ExecutionFillSide::Sell, 1.0, 90.0),
                },
                RuntimeEvent::TradableCandleCompleted,
            ],
            snapshot(None),
        );

        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
        assert!(policy.observe_step(&risk_exit_step).is_none());
        assert!(policy.observe_step(&blocked_step(&[h1])).is_none());
    }
}
