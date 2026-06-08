//! Paper Trading persistence adapter.
//!
//! This module is a daemon-side adapter: it restores runtime-local Paper Trading
//! Portfolio State from dedicated paper persistence records and projects selected
//! runtime Portfolio Transition events into paper persistence. It intentionally
//! does not add DB details, persistence IDs, or paper/real-money mode flags to
//! `trading-runtime`.

use std::sync::Arc;

use db_layer::{
    get_paper_open_position, get_paper_trades, open_paper_position, record_paper_position_closed,
    DbConnection, DbError, PaperExitKind, PaperOpenPosition, PaperTrade,
};
use domain::{ClosedPosition, OpenPosition, PositionRiskBoundaries, PositionSide};
use thiserror::Error;
use trading_runtime::{ExitKind, PortfolioState, RiskExitKind, RuntimeEvent, RuntimeStep};

const OPEN_POSITION_KEY_PREFIX: &str = "paper-open-v1";
const COMPLETED_TRADE_KEY_PREFIX: &str = "paper-trade-v1";
const OPEN_POSITION_HASH_VERSION: &str = "paper-open-position:v1";
const COMPLETED_TRADE_HASH_VERSION: &str = "paper-completed-trade:v1";

/// Errors surfaced by the Paper Trading Persistence Adapter.
#[derive(Debug, Error)]
pub enum PaperTradingPersistenceError {
    #[error("paper persistence database error: {0}")]
    Db(#[from] DbError),

    #[error("paper persistence store error: {0}")]
    Store(String),

    #[error("persisted paper position has unknown side '{side}'")]
    UnknownPositionSide { side: String },

    #[error(
        "paper persistence record boundary mismatch for projection key '{projection_key}': record belongs to strategy_identity '{record_strategy_identity}' and runtime_asset '{record_runtime_asset}', but adapter is scoped to strategy_identity '{adapter_strategy_identity}' and runtime_asset '{adapter_runtime_asset}'"
    )]
    RecordBoundaryMismatch {
        projection_key: String,
        record_strategy_identity: String,
        record_runtime_asset: String,
        adapter_strategy_identity: String,
        adapter_runtime_asset: String,
    },
}

/// Low-level paper persistence operations used by the daemon adapter.
///
/// Implementations should delegate to `db-layer` helpers or deterministic test
/// doubles. They must not interpret `RuntimeStep` or `RuntimeEvent`; projection
/// is owned by [`PaperTradingPersistenceAdapter`].
pub trait PaperTradingPersistenceStore {
    fn load_open_position(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError>;

    fn load_trades(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError>;

    fn open_position(
        &self,
        position: &PaperOpenPosition,
    ) -> Result<(), PaperTradingPersistenceError>;

    fn record_position_closed(
        &self,
        open_projection_key: &str,
        trade: &PaperTrade,
    ) -> Result<(), PaperTradingPersistenceError>;
}

impl<T: PaperTradingPersistenceStore + ?Sized> PaperTradingPersistenceStore for &T {
    fn load_open_position(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError> {
        (**self).load_open_position(strategy_identity, runtime_asset)
    }

    fn load_trades(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError> {
        (**self).load_trades(strategy_identity, runtime_asset)
    }

    fn open_position(
        &self,
        position: &PaperOpenPosition,
    ) -> Result<(), PaperTradingPersistenceError> {
        (**self).open_position(position)
    }

    fn record_position_closed(
        &self,
        open_projection_key: &str,
        trade: &PaperTrade,
    ) -> Result<(), PaperTradingPersistenceError> {
        (**self).record_position_closed(open_projection_key, trade)
    }
}

impl<T: PaperTradingPersistenceStore + ?Sized> PaperTradingPersistenceStore for Arc<T> {
    fn load_open_position(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError> {
        (**self).load_open_position(strategy_identity, runtime_asset)
    }

    fn load_trades(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError> {
        (**self).load_trades(strategy_identity, runtime_asset)
    }

    fn open_position(
        &self,
        position: &PaperOpenPosition,
    ) -> Result<(), PaperTradingPersistenceError> {
        (**self).open_position(position)
    }

    fn record_position_closed(
        &self,
        open_projection_key: &str,
        trade: &PaperTrade,
    ) -> Result<(), PaperTradingPersistenceError> {
        (**self).record_position_closed(open_projection_key, trade)
    }
}

impl PaperTradingPersistenceStore for DbConnection {
    fn load_open_position(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError> {
        Ok(get_paper_open_position(
            self,
            strategy_identity,
            runtime_asset,
        ))
    }

    fn load_trades(
        &self,
        strategy_identity: &str,
        runtime_asset: &str,
    ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError> {
        Ok(get_paper_trades(self, strategy_identity, runtime_asset))
    }

    fn open_position(
        &self,
        position: &PaperOpenPosition,
    ) -> Result<(), PaperTradingPersistenceError> {
        open_paper_position(self, position).map_err(PaperTradingPersistenceError::from)
    }

    fn record_position_closed(
        &self,
        open_projection_key: &str,
        trade: &PaperTrade,
    ) -> Result<(), PaperTradingPersistenceError> {
        record_paper_position_closed(self, open_projection_key, trade)
            .map_err(PaperTradingPersistenceError::from)
    }
}

/// Summary of persistable Runtime Events projected from one RuntimeStep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PaperProjectionReport {
    pub persisted_transition_count: usize,
}

/// Daemon-side adapter for one Strategy Identity × Runtime Asset Paper Trading
/// persistence boundary.
pub struct PaperTradingPersistenceAdapter<S> {
    store: S,
    strategy_identity: String,
    runtime_asset: String,
}

impl<S> PaperTradingPersistenceAdapter<S> {
    pub fn new(
        store: S,
        strategy_identity: impl Into<String>,
        runtime_asset: impl Into<String>,
    ) -> Self {
        Self {
            store,
            strategy_identity: strategy_identity.into(),
            runtime_asset: runtime_asset.into(),
        }
    }

    pub fn strategy_identity(&self) -> &str {
        &self.strategy_identity
    }

    pub fn runtime_asset(&self) -> &str {
        &self.runtime_asset
    }

    pub fn into_store(self) -> S {
        self.store
    }
}

impl<S: PaperTradingPersistenceStore> PaperTradingPersistenceAdapter<S> {
    /// Restore runtime-local Paper Trading Portfolio State from paper records.
    ///
    /// This is Paper Trading only: realized cash is the configured starting
    /// balance plus persisted paper trade PnL, completed trade count is the
    /// number of persisted paper trades, and any persisted paper open position
    /// becomes the runtime-local open position.
    pub fn restore_portfolio_state(
        &self,
        configured_starting_balance: f64,
    ) -> Result<PortfolioState, PaperTradingPersistenceError> {
        let trades = self
            .store
            .load_trades(&self.strategy_identity, &self.runtime_asset)?;
        for trade in &trades {
            self.ensure_trade_boundary(trade)?;
        }

        let open_position = self
            .store
            .load_open_position(&self.strategy_identity, &self.runtime_asset)?;
        if let Some(position) = &open_position {
            self.ensure_open_position_boundary(position)?;
        }

        let realized_cash_balance = configured_starting_balance
            + trades.iter().map(|trade| trade.realized_pnl).sum::<f64>();
        let completed_trade_count = trades.len();
        let open_position = open_position
            .as_ref()
            .map(paper_open_position_to_domain)
            .transpose()?;

        Ok(PortfolioState {
            realized_cash_balance,
            open_position,
            completed_trade_count,
        })
    }

    /// Project persistable Runtime Portfolio Transition events in RuntimeStep
    /// order. Non-transition events, including PortfolioUpdated, are ignored for
    /// Paper Trading persistence V1.
    pub fn project_step(
        &self,
        step: &RuntimeStep,
    ) -> Result<PaperProjectionReport, PaperTradingPersistenceError> {
        let mut report = PaperProjectionReport::default();

        for event in &step.events {
            match event {
                RuntimeEvent::PositionOpened { position } => {
                    let record = paper_open_position_from_runtime(
                        &self.strategy_identity,
                        &self.runtime_asset,
                        position,
                    );
                    self.store.open_position(&record)?;
                    report.persisted_transition_count += 1;
                }
                RuntimeEvent::PositionClosed {
                    closed_position,
                    exit_kind,
                } => {
                    let open_projection_key = open_position_projection_key(
                        &self.strategy_identity,
                        &self.runtime_asset,
                        &closed_position.position,
                    );
                    let trade = paper_trade_from_runtime(
                        &self.strategy_identity,
                        &self.runtime_asset,
                        closed_position,
                        *exit_kind,
                    );
                    self.store
                        .record_position_closed(&open_projection_key, &trade)?;
                    report.persisted_transition_count += 1;
                }
                _ => {}
            }
        }

        Ok(report)
    }

    fn ensure_open_position_boundary(
        &self,
        position: &PaperOpenPosition,
    ) -> Result<(), PaperTradingPersistenceError> {
        if position.strategy_identity == self.strategy_identity
            && position.runtime_asset == self.runtime_asset
        {
            return Ok(());
        }

        Err(PaperTradingPersistenceError::RecordBoundaryMismatch {
            projection_key: position.projection_key.clone(),
            record_strategy_identity: position.strategy_identity.clone(),
            record_runtime_asset: position.runtime_asset.clone(),
            adapter_strategy_identity: self.strategy_identity.clone(),
            adapter_runtime_asset: self.runtime_asset.clone(),
        })
    }

    fn ensure_trade_boundary(
        &self,
        trade: &PaperTrade,
    ) -> Result<(), PaperTradingPersistenceError> {
        if trade.strategy_identity == self.strategy_identity
            && trade.runtime_asset == self.runtime_asset
        {
            return Ok(());
        }

        Err(PaperTradingPersistenceError::RecordBoundaryMismatch {
            projection_key: trade.projection_key.clone(),
            record_strategy_identity: trade.strategy_identity.clone(),
            record_runtime_asset: trade.runtime_asset.clone(),
            adapter_strategy_identity: self.strategy_identity.clone(),
            adapter_runtime_asset: self.runtime_asset.clone(),
        })
    }
}

/// Deterministic key for a runtime-local open Paper Trading position.
///
/// Position Risk Boundaries are intentionally not part of identity; they are
/// persisted as data and compared by the paper reducer/helper.
pub fn open_position_projection_key(
    strategy_identity: &str,
    runtime_asset: &str,
    position: &OpenPosition,
) -> String {
    let mut bytes = Vec::new();
    append_str(&mut bytes, OPEN_POSITION_HASH_VERSION);
    append_open_position_identity(&mut bytes, strategy_identity, runtime_asset, position);
    format!("{OPEN_POSITION_KEY_PREFIX}:{:016x}", stable_hash64(&bytes))
}

/// Deterministic key for a completed Paper Trading position.
///
/// This extends the open-position identity with exit time, exit price, realized
/// PnL, and typed exit kind. Position Risk Boundaries remain persisted data,
/// not primary identity fields.
pub fn completed_trade_projection_key(
    strategy_identity: &str,
    runtime_asset: &str,
    closed_position: &ClosedPosition,
    exit_kind: ExitKind,
) -> String {
    let mut bytes = Vec::new();
    append_str(&mut bytes, COMPLETED_TRADE_HASH_VERSION);
    append_open_position_identity(
        &mut bytes,
        strategy_identity,
        runtime_asset,
        &closed_position.position,
    );
    append_i64(&mut bytes, closed_position.exit_time);
    append_f64(&mut bytes, closed_position.exit_price);
    append_f64(&mut bytes, closed_position.realized_pnl);
    append_exit_kind(&mut bytes, exit_kind);
    format!(
        "{COMPLETED_TRADE_KEY_PREFIX}:{:016x}",
        stable_hash64(&bytes)
    )
}

pub fn paper_open_position_from_runtime(
    strategy_identity: &str,
    runtime_asset: &str,
    position: &OpenPosition,
) -> PaperOpenPosition {
    PaperOpenPosition {
        projection_key: open_position_projection_key(strategy_identity, runtime_asset, position),
        strategy_identity: strategy_identity.to_string(),
        runtime_asset: runtime_asset.to_string(),
        side: position.side.to_string(),
        entry_price: position.entry_price,
        quantity: position.quantity,
        entry_time: position.entry_time,
        stop_loss: position.risk_boundaries.stop_loss,
        take_profit: position.risk_boundaries.take_profit,
        entry_metadata: None,
    }
}

pub fn paper_trade_from_runtime(
    strategy_identity: &str,
    runtime_asset: &str,
    closed_position: &ClosedPosition,
    exit_kind: ExitKind,
) -> PaperTrade {
    PaperTrade {
        projection_key: completed_trade_projection_key(
            strategy_identity,
            runtime_asset,
            closed_position,
            exit_kind,
        ),
        strategy_identity: strategy_identity.to_string(),
        runtime_asset: runtime_asset.to_string(),
        side: closed_position.position.side.to_string(),
        entry_price: closed_position.position.entry_price,
        exit_price: closed_position.exit_price,
        quantity: closed_position.position.quantity,
        realized_pnl: closed_position.realized_pnl,
        entry_time: closed_position.position.entry_time,
        exit_time: closed_position.exit_time,
        stop_loss: closed_position.position.risk_boundaries.stop_loss,
        take_profit: closed_position.position.risk_boundaries.take_profit,
        exit_kind: paper_exit_kind(exit_kind),
        entry_metadata: None,
        exit_metadata: None,
    }
}

fn paper_open_position_to_domain(
    position: &PaperOpenPosition,
) -> Result<OpenPosition, PaperTradingPersistenceError> {
    Ok(OpenPosition {
        symbol: position.runtime_asset.clone(),
        side: paper_position_side(&position.side)?,
        entry_price: position.entry_price,
        quantity: position.quantity,
        entry_time: position.entry_time,
        risk_boundaries: PositionRiskBoundaries {
            stop_loss: position.stop_loss,
            take_profit: position.take_profit,
        },
    })
}

fn paper_position_side(side: &str) -> Result<PositionSide, PaperTradingPersistenceError> {
    match side {
        "long" => Ok(PositionSide::Long),
        "short" => Ok(PositionSide::Short),
        other => Err(PaperTradingPersistenceError::UnknownPositionSide {
            side: other.to_string(),
        }),
    }
}

fn paper_exit_kind(exit_kind: ExitKind) -> PaperExitKind {
    match exit_kind {
        ExitKind::StrategyExit => PaperExitKind::StrategyExit,
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        } => PaperExitKind::RiskExitStopLoss,
        ExitKind::RiskExit {
            selected: RiskExitKind::TakeProfit,
        } => PaperExitKind::RiskExitTakeProfit,
        ExitKind::ForceClose => PaperExitKind::ForceClose,
    }
}

fn append_open_position_identity(
    bytes: &mut Vec<u8>,
    strategy_identity: &str,
    runtime_asset: &str,
    position: &OpenPosition,
) {
    append_str(bytes, strategy_identity);
    append_str(bytes, runtime_asset);
    append_side(bytes, position.side);
    append_i64(bytes, position.entry_time);
    append_f64(bytes, position.entry_price);
    append_f64(bytes, position.quantity);
}

fn append_str(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&(value.len() as u64).to_be_bytes());
    bytes.extend_from_slice(value.as_bytes());
}

fn append_i64(bytes: &mut Vec<u8>, value: i64) {
    bytes.extend_from_slice(&value.to_be_bytes());
}

fn append_f64(bytes: &mut Vec<u8>, value: f64) {
    bytes.extend_from_slice(&value.to_bits().to_be_bytes());
}

fn append_side(bytes: &mut Vec<u8>, side: PositionSide) {
    bytes.push(match side {
        PositionSide::Long => b'L',
        PositionSide::Short => b'S',
    });
}

fn append_exit_kind(bytes: &mut Vec<u8>, exit_kind: ExitKind) {
    bytes.push(match exit_kind {
        ExitKind::StrategyExit => b'S',
        ExitKind::RiskExit {
            selected: RiskExitKind::StopLoss,
        } => b'L',
        ExitKind::RiskExit {
            selected: RiskExitKind::TakeProfit,
        } => b'T',
        ExitKind::ForceClose => b'F',
    });
}

fn stable_hash64(bytes: &[u8]) -> u64 {
    // FNV-1a 64-bit: simple, deterministic, and sufficient for stable
    // projection keys without pulling DB or runtime crates into hashing policy.
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use trading_runtime::{RuntimePortfolioSnapshot, StrategyDecision};

    use super::*;

    const STRATEGY_IDENTITY: &str = "mean-reversion-paper";
    const RUNTIME_ASSET: &str = "BTC-USD";

    #[derive(Debug, Clone, PartialEq)]
    enum StoreCall {
        Open {
            projection_key: String,
        },
        Close {
            open_projection_key: String,
            trade_projection_key: String,
        },
    }

    #[derive(Default)]
    struct FakePaperStore {
        open_position: RefCell<Option<PaperOpenPosition>>,
        trades: RefCell<Vec<PaperTrade>>,
        calls: RefCell<Vec<StoreCall>>,
        fail_next_open: RefCell<Option<String>>,
        fail_next_close: RefCell<Option<String>>,
    }

    impl FakePaperStore {
        fn with_open_position(position: PaperOpenPosition) -> Self {
            Self {
                open_position: RefCell::new(Some(position)),
                ..Self::default()
            }
        }

        fn set_trades(&self, trades: Vec<PaperTrade>) {
            *self.trades.borrow_mut() = trades;
        }

        fn fail_next_open(&self, message: impl Into<String>) {
            *self.fail_next_open.borrow_mut() = Some(message.into());
        }
    }

    impl PaperTradingPersistenceStore for FakePaperStore {
        fn load_open_position(
            &self,
            _strategy_identity: &str,
            _runtime_asset: &str,
        ) -> Result<Option<PaperOpenPosition>, PaperTradingPersistenceError> {
            Ok(self.open_position.borrow().clone())
        }

        fn load_trades(
            &self,
            _strategy_identity: &str,
            _runtime_asset: &str,
        ) -> Result<Vec<PaperTrade>, PaperTradingPersistenceError> {
            Ok(self.trades.borrow().clone())
        }

        fn open_position(
            &self,
            position: &PaperOpenPosition,
        ) -> Result<(), PaperTradingPersistenceError> {
            self.calls.borrow_mut().push(StoreCall::Open {
                projection_key: position.projection_key.clone(),
            });

            if let Some(message) = self.fail_next_open.borrow_mut().take() {
                return Err(PaperTradingPersistenceError::Store(message));
            }

            let mut open_position = self.open_position.borrow_mut();
            match open_position.as_ref() {
                Some(existing) if existing == position => Ok(()),
                Some(existing) => Err(PaperTradingPersistenceError::Store(format!(
                    "conflicting open position '{}'",
                    existing.projection_key
                ))),
                None => {
                    *open_position = Some(position.clone());
                    Ok(())
                }
            }
        }

        fn record_position_closed(
            &self,
            open_projection_key: &str,
            trade: &PaperTrade,
        ) -> Result<(), PaperTradingPersistenceError> {
            self.calls.borrow_mut().push(StoreCall::Close {
                open_projection_key: open_projection_key.to_string(),
                trade_projection_key: trade.projection_key.clone(),
            });

            if let Some(message) = self.fail_next_close.borrow_mut().take() {
                return Err(PaperTradingPersistenceError::Store(message));
            }

            let mut trades = self.trades.borrow_mut();
            if let Some(existing_trade) = trades
                .iter()
                .find(|existing| existing.projection_key == trade.projection_key)
            {
                if existing_trade == trade {
                    remove_matching_open_position(
                        &mut self.open_position.borrow_mut(),
                        open_projection_key,
                    );
                    return Ok(());
                }

                return Err(PaperTradingPersistenceError::Store(format!(
                    "conflicting completed trade '{}'",
                    existing_trade.projection_key
                )));
            }

            let mut open_position = self.open_position.borrow_mut();
            let Some(existing_open) = open_position.as_ref() else {
                return Err(PaperTradingPersistenceError::Store(format!(
                    "no matching open paper position for '{open_projection_key}'"
                )));
            };
            if existing_open.projection_key != open_projection_key {
                return Err(PaperTradingPersistenceError::Store(format!(
                    "open paper position '{}' does not match '{open_projection_key}'",
                    existing_open.projection_key
                )));
            }

            *open_position = None;
            trades.push(trade.clone());
            Ok(())
        }
    }

    fn remove_matching_open_position(
        open_position: &mut Option<PaperOpenPosition>,
        open_projection_key: &str,
    ) {
        if open_position
            .as_ref()
            .is_some_and(|open| open.projection_key == open_projection_key)
        {
            *open_position = None;
        }
    }

    fn runtime_position(side: PositionSide) -> OpenPosition {
        OpenPosition {
            symbol: RUNTIME_ASSET.into(),
            side,
            entry_price: 100.0,
            quantity: 2.0,
            entry_time: 1_700_000_000_000,
            risk_boundaries: PositionRiskBoundaries {
                stop_loss: Some(95.0),
                take_profit: Some(120.0),
            },
        }
    }

    fn closed_position(position: OpenPosition, exit_kind: ExitKind) -> (ClosedPosition, ExitKind) {
        let exit_price = match exit_kind {
            ExitKind::RiskExit {
                selected: RiskExitKind::StopLoss,
            } => 95.0,
            _ => 110.0,
        };
        (
            ClosedPosition {
                position,
                exit_price,
                exit_time: 1_700_000_060_000,
                realized_pnl: 20.0,
            },
            exit_kind,
        )
    }

    fn snapshot(open_position: Option<OpenPosition>) -> RuntimePortfolioSnapshot {
        RuntimePortfolioSnapshot {
            realized_cash_balance: 1_000.0,
            open_position,
            completed_trade_count: 0,
            current_equity: 1_000.0,
        }
    }

    fn step(events: Vec<RuntimeEvent>) -> RuntimeStep {
        RuntimeStep::new(events, snapshot(None))
    }

    fn adapter(store: &FakePaperStore) -> PaperTradingPersistenceAdapter<&FakePaperStore> {
        PaperTradingPersistenceAdapter::new(store, STRATEGY_IDENTITY, RUNTIME_ASSET)
    }

    #[test]
    fn restore_builds_portfolio_state_from_paper_trades_and_open_position() {
        let open_position = paper_open_position_from_runtime(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &runtime_position(PositionSide::Short),
        );
        let store = FakePaperStore::with_open_position(open_position);

        let (first_close, _) =
            closed_position(runtime_position(PositionSide::Long), ExitKind::StrategyExit);
        let mut first_trade = paper_trade_from_runtime(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &first_close,
            ExitKind::StrategyExit,
        );
        first_trade.realized_pnl = 12.5;
        let (second_close, _) =
            closed_position(runtime_position(PositionSide::Short), ExitKind::ForceClose);
        let mut second_trade = paper_trade_from_runtime(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &second_close,
            ExitKind::ForceClose,
        );
        second_trade.realized_pnl = -2.5;
        store.set_trades(vec![first_trade, second_trade]);

        let restored = adapter(&store)
            .restore_portfolio_state(1_000.0)
            .expect("paper portfolio restore should succeed");

        assert_eq!(restored.realized_cash_balance, 1_010.0);
        assert_eq!(restored.completed_trade_count, 2);
        let position = restored
            .open_position
            .expect("persisted paper open position should restore");
        assert_eq!(position.symbol, RUNTIME_ASSET);
        assert_eq!(position.side, PositionSide::Short);
        assert_eq!(position.entry_price, 100.0);
        assert_eq!(position.quantity, 2.0);
        assert_eq!(position.risk_boundaries.stop_loss, Some(95.0));
        assert_eq!(position.risk_boundaries.take_profit, Some(120.0));
    }

    #[test]
    fn projection_keys_are_deterministic_and_exclude_risk_boundaries_from_identity() {
        let position = runtime_position(PositionSide::Long);

        let first_key = open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);
        let second_key = open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);
        assert_eq!(first_key, second_key);

        let mut risk_changed = position.clone();
        risk_changed.risk_boundaries.stop_loss = Some(90.0);
        risk_changed.risk_boundaries.take_profit = None;
        assert_eq!(
            first_key,
            open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &risk_changed),
            "Position Risk Boundaries are persisted data, not identity fields"
        );

        let mut quantity_changed = position.clone();
        quantity_changed.quantity = 3.0;
        assert_ne!(
            first_key,
            open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &quantity_changed)
        );
        assert_ne!(
            first_key,
            open_position_projection_key("other-strategy", RUNTIME_ASSET, &position)
        );
        assert_ne!(
            first_key,
            open_position_projection_key(STRATEGY_IDENTITY, "ETH-USD", &position)
        );

        let (closed, _) = closed_position(position, ExitKind::StrategyExit);
        let strategy_exit_key = completed_trade_projection_key(
            STRATEGY_IDENTITY,
            RUNTIME_ASSET,
            &closed,
            ExitKind::StrategyExit,
        );
        assert_eq!(
            strategy_exit_key,
            completed_trade_projection_key(
                STRATEGY_IDENTITY,
                RUNTIME_ASSET,
                &closed,
                ExitKind::StrategyExit,
            )
        );
        assert_ne!(
            strategy_exit_key,
            completed_trade_projection_key(
                STRATEGY_IDENTITY,
                RUNTIME_ASSET,
                &closed,
                ExitKind::ForceClose,
            )
        );
    }

    #[test]
    fn project_step_processes_persistable_events_in_runtime_order() {
        let store = FakePaperStore::default();
        let adapter = adapter(&store);
        let position = runtime_position(PositionSide::Long);
        let (closed, exit_kind) = closed_position(position.clone(), ExitKind::StrategyExit);
        let open_key = open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);
        let trade_key =
            completed_trade_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &closed, exit_kind);

        let report = adapter
            .project_step(&step(vec![
                RuntimeEvent::PortfolioUpdated {
                    snapshot: snapshot(Some(position.clone())),
                },
                RuntimeEvent::PositionOpened {
                    position: position.clone(),
                },
                RuntimeEvent::PositionClosed {
                    closed_position: closed,
                    exit_kind,
                },
                RuntimeEvent::PortfolioUpdated {
                    snapshot: snapshot(None),
                },
            ]))
            .expect("ordered projection should succeed");

        assert_eq!(
            report,
            PaperProjectionReport {
                persisted_transition_count: 2
            }
        );
        assert_eq!(
            *store.calls.borrow(),
            vec![
                StoreCall::Open {
                    projection_key: open_key.clone()
                },
                StoreCall::Close {
                    open_projection_key: open_key,
                    trade_projection_key: trade_key,
                },
            ]
        );
    }

    #[test]
    fn project_step_ignores_non_transition_runtime_events() {
        let store = FakePaperStore::default();
        let report = adapter(&store)
            .project_step(&step(vec![
                RuntimeEvent::StrategyDecisionProduced {
                    decision: StrategyDecision::hold(),
                },
                RuntimeEvent::PortfolioUpdated {
                    snapshot: snapshot(None),
                },
                RuntimeEvent::StrategyTickCompleted,
                RuntimeEvent::TradableCandleCompleted,
            ]))
            .expect("non-transition events should be ignored");

        assert_eq!(
            report,
            PaperProjectionReport {
                persisted_transition_count: 0
            }
        );
        assert!(store.calls.borrow().is_empty());
        assert!(store.open_position.borrow().is_none());
        assert!(store.trades.borrow().is_empty());
    }

    #[test]
    fn duplicate_projection_is_idempotent_when_store_confirms_same_data() {
        let store = FakePaperStore::default();
        let adapter = adapter(&store);
        let position = runtime_position(PositionSide::Long);
        let open_step = step(vec![RuntimeEvent::PositionOpened {
            position: position.clone(),
        }]);

        adapter.project_step(&open_step).unwrap();
        adapter.project_step(&open_step).unwrap();

        let open_key = open_position_projection_key(STRATEGY_IDENTITY, RUNTIME_ASSET, &position);
        assert_eq!(
            store
                .open_position
                .borrow()
                .as_ref()
                .map(|p| &p.projection_key),
            Some(&open_key)
        );

        let (closed, exit_kind) = closed_position(position, ExitKind::ForceClose);
        let close_step = step(vec![RuntimeEvent::PositionClosed {
            closed_position: closed,
            exit_kind,
        }]);
        adapter.project_step(&close_step).unwrap();
        adapter.project_step(&close_step).unwrap();

        assert!(store.open_position.borrow().is_none());
        assert_eq!(store.trades.borrow().len(), 1);
        assert_eq!(store.calls.borrow().len(), 4);
    }

    #[test]
    fn projection_errors_surface_and_stop_processing_following_events() {
        let store = FakePaperStore::default();
        store.fail_next_open("unconfirmed open projection");
        let adapter = adapter(&store);
        let position = runtime_position(PositionSide::Long);
        let (closed, exit_kind) = closed_position(position.clone(), ExitKind::StrategyExit);

        let error = adapter
            .project_step(&step(vec![
                RuntimeEvent::PositionOpened { position },
                RuntimeEvent::PositionClosed {
                    closed_position: closed,
                    exit_kind,
                },
            ]))
            .expect_err("unconfirmed projection should be an adapter error");

        assert!(error.to_string().contains("unconfirmed open projection"));
        assert_eq!(store.calls.borrow().len(), 1);
        assert!(store.trades.borrow().is_empty());
    }

    #[test]
    fn close_projection_inconsistency_is_returned_to_caller() {
        let store = FakePaperStore::default();
        let position = runtime_position(PositionSide::Long);
        let (closed, exit_kind) = closed_position(position, ExitKind::StrategyExit);

        let error = adapter(&store)
            .project_step(&step(vec![RuntimeEvent::PositionClosed {
                closed_position: closed,
                exit_kind,
            }]))
            .expect_err("missing matching open position should be inconsistent");

        assert!(error
            .to_string()
            .contains("no matching open paper position"));
        assert_eq!(store.calls.borrow().len(), 1);
        assert!(store.trades.borrow().is_empty());
    }
}
