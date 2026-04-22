//! In-memory backtester.
//!
//! Pure, synchronous, no I/O, no database. Feeds candles through a live
//! `engine::Engine` and simulates trade execution in RAM. Designed so that
//! the GPUI frontend can embed the exact same code — it just keeps a
//! `BacktestState` around and paints it.
//!
//! # Example
//! ```no_run
//! use backtester::{run_backtest, BacktestConfig};
//! use engine::Engine;
//!
//! # fn go() -> anyhow::Result<()> {
//! let src = std::fs::read_to_string("strategies/sma_cross.rhai")?;
//! let mut engine = Engine::new(&src)?;
//! let candles = vec![];           // load from wherever
//! let result = run_backtest(&mut engine, candles, BacktestConfig::default())?;
//! println!("{} trades, final equity {:.2}", result.trades.len(), result.metrics.final_equity);
//! # Ok(()) }
//! ```
use anyhow::Result;
use engine::Engine;
use shared::{
    plan_action, realized_pnl, Action, Candle, Context, Position, PositionSide, TradeDecision,
};

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single completed (closed) trade in the backtest.
#[derive(Debug, Clone)]
pub struct Trade {
    pub side:         PositionSide,
    pub entry_price:  f64,
    pub exit_price:   f64,
    pub size:         f64,
    pub pnl:          f64,
    pub entry_time:   i64,
    pub exit_time:    i64,
    pub entry_reason: String,
    pub exit_reason:  String,
}

/// Equity measurement at a given candle timestamp.
#[derive(Debug, Clone, Copy)]
pub struct EquityPoint {
    pub timestamp: i64,
    pub equity:    f64,
}

/// Aggregated metrics after a backtest run.
#[derive(Debug, Clone, Copy)]
pub struct BacktestMetrics {
    pub trade_count:         usize,
    pub wins:                usize,
    pub losses:              usize,
    pub win_rate:            f64,
    pub total_pnl:           f64,
    pub max_drawdown:        f64,
    pub max_drawdown_pct:    f64,
    pub final_equity:        f64,
    pub peak_equity:         f64,
    pub cagr:                f64,
    pub sharpe:              f64,
    pub time_in_market_pct:  f64,
    pub years:               f64,
}

/// Buy-and-hold benchmark on the same candle series.
#[derive(Debug, Clone, Copy)]
pub struct Benchmark {
    pub final_equity:     f64,
    pub cagr:             f64,
    pub max_drawdown:     f64,
    pub max_drawdown_pct: f64,
}

/// Full output of a backtest run.
#[derive(Debug, Clone)]
pub struct BacktestResult {
    pub trades:        Vec<Trade>,
    pub equity_curve:  Vec<EquityPoint>,
    pub metrics:       BacktestMetrics,
    pub benchmark:     Benchmark,
    pub final_balance: f64,
}

/// Configuration for a backtest run.
#[derive(Debug, Clone, Copy)]
pub struct BacktestConfig {
    pub initial_balance: f64,
    /// Number of leading candles to push into the engine *without* ticking.
    /// Typically equal to the detected warmup period.
    pub warmup_bars:     usize,
}

impl Default for BacktestConfig {
    fn default() -> Self {
        Self { initial_balance: 10_000.0, warmup_bars: 0 }
    }
}

// ── InMemoryExecutor ──────────────────────────────────────────────────────────

/// Paper-trading state machine with no I/O.  Same decision logic as
/// `trading_daemon::order_executor::PaperExecutor` (via `shared::plan_action`),
/// but all writes go into `trades` / `equity_curve` rather than SpacetimeDB.
///
/// Public so the UI can drive it one candle at a time.
#[derive(Debug, Clone)]
pub struct InMemoryExecutor {
    balance:      f64,
    position:     Option<Position>,
    pub trades:   Vec<Trade>,
    pub equity_curve: Vec<EquityPoint>,
    peak_equity:  f64,
    max_drawdown: f64,
    bars_total:       usize,
    bars_in_position: usize,
}

impl InMemoryExecutor {
    pub fn new(initial_balance: f64) -> Self {
        Self {
            balance:      initial_balance,
            position:     None,
            trades:       Vec::new(),
            equity_curve: Vec::new(),
            peak_equity:  initial_balance,
            max_drawdown: 0.0,
            bars_total:       0,
            bars_in_position: 0,
        }
    }

    pub fn balance(&self)  -> f64            { self.balance }
    pub fn position(&self) -> Option<&Position> { self.position.as_ref() }

    /// Build the `Context` passed to the strategy for the next tick.
    pub fn context(&self, last_close: f64) -> Context {
        let unrealized = self.position.as_ref()
            .map(|p| p.unrealised_pnl(last_close))
            .unwrap_or(0.0);
        Context {
            balance:      self.balance,
            equity:       self.balance + unrealized,
            position:     self.position.clone(),
            trades_count: self.trades.len() as u32,
        }
    }

    /// Advance one candle. Checks stops (on a pre-existing position only),
    /// then applies the strategy's decision, then records an equity point.
    pub fn apply(&mut self, candle: &Candle, decision: &TradeDecision) {
        if self.position.is_some() {
            self.check_stops(candle);
        }

        let action = plan_action(&decision.signal, self.position.as_ref().map(|p| p.side));
        match action {
            Action::OpenLong  => self.open_position(PositionSide::Long,  candle, decision),
            Action::OpenShort => self.open_position(PositionSide::Short, candle, decision),
            Action::Close     => {
                let reason = decision.reason.clone().unwrap_or_else(|| "strategy close".into());
                self.close_position(candle, &reason);
            }
            Action::Nothing   => {}
        }

        self.record_equity(candle);
    }

    // ── internals ────────────────────────────────────────────────────────────

    fn open_position(&mut self, side: PositionSide, candle: &Candle, decision: &TradeDecision) {
        let size = self.balance * decision.size / candle.close;
        self.position = Some(Position {
            symbol:      candle.symbol.clone(),
            side,
            entry_price: candle.close,
            size,
            entry_time:  candle.timestamp,
            stop_loss:   decision.stop_loss,
            take_profit: decision.take_profit,
        });
    }

    fn close_position(&mut self, candle: &Candle, reason: &str) {
        let pos = match self.position.take() {
            Some(p) => p,
            None    => return,
        };
        let exit_price = candle.close;
        let pnl = realized_pnl(pos.side, pos.entry_price, exit_price, pos.size);
        self.balance += pnl;
        self.trades.push(Trade {
            side:         pos.side,
            entry_price:  pos.entry_price,
            exit_price,
            size:         pos.size,
            pnl,
            entry_time:   pos.entry_time,
            exit_time:    candle.timestamp,
            entry_reason: String::new(),
            exit_reason:  reason.to_string(),
        });
    }

    fn check_stops(&mut self, candle: &Candle) {
        let (hit_sl, hit_tp) = match &self.position {
            None => return,
            Some(pos) => match pos.side {
                PositionSide::Long => (
                    pos.stop_loss  .map(|sl| candle.low  <= sl).unwrap_or(false),
                    pos.take_profit.map(|tp| candle.high >= tp).unwrap_or(false),
                ),
                PositionSide::Short => (
                    pos.stop_loss  .map(|sl| candle.high >= sl).unwrap_or(false),
                    pos.take_profit.map(|tp| candle.low  <= tp).unwrap_or(false),
                ),
            },
        };
        if hit_sl {
            self.close_position(candle, "stop-loss triggered");
        } else if hit_tp {
            self.close_position(candle, "take-profit triggered");
        }
    }

    fn record_equity(&mut self, candle: &Candle) {
        let unrealized = self.position.as_ref()
            .map(|p| p.unrealised_pnl(candle.close))
            .unwrap_or(0.0);
        let equity = self.balance + unrealized;

        if equity > self.peak_equity { self.peak_equity = equity; }
        let dd = self.peak_equity - equity;
        if dd > self.max_drawdown { self.max_drawdown = dd; }

        self.bars_total += 1;
        if self.position.is_some() { self.bars_in_position += 1; }

        self.equity_curve.push(EquityPoint {
            timestamp: candle.timestamp,
            equity,
        });
    }

    pub fn metrics(&self, initial_balance: f64) -> BacktestMetrics {
        let trade_count = self.trades.len();
        let wins        = self.trades.iter().filter(|t| t.pnl >  0.0).count();
        let losses      = self.trades.iter().filter(|t| t.pnl <  0.0).count();
        let total_pnl   = self.trades.iter().map(|t| t.pnl).sum::<f64>();
        let final_eq    = self.equity_curve.last().map(|p| p.equity).unwrap_or(initial_balance);
        let win_rate    = if trade_count == 0 { 0.0 } else { wins as f64 / trade_count as f64 };

        let years = span_years(&self.equity_curve);
        let cagr = compute_cagr(initial_balance, final_eq, years);
        let sharpe = compute_sharpe(&self.equity_curve, years);
        let max_dd_pct = if self.peak_equity > 0.0 { self.max_drawdown / self.peak_equity } else { 0.0 };
        let tim_pct = if self.bars_total == 0 { 0.0 } else {
            self.bars_in_position as f64 / self.bars_total as f64
        };

        BacktestMetrics {
            trade_count,
            wins,
            losses,
            win_rate,
            total_pnl,
            max_drawdown:       self.max_drawdown,
            max_drawdown_pct:   max_dd_pct,
            final_equity:       final_eq,
            peak_equity:        self.peak_equity,
            cagr,
            sharpe,
            time_in_market_pct: tim_pct,
            years,
        }
    }
}

// ── Analytics helpers ────────────────────────────────────────────────────────

const MS_PER_YEAR: f64 = 365.25 * 86_400_000.0;

fn span_years(curve: &[EquityPoint]) -> f64 {
    match (curve.first(), curve.last()) {
        (Some(a), Some(b)) if b.timestamp > a.timestamp =>
            (b.timestamp - a.timestamp) as f64 / MS_PER_YEAR,
        _ => 0.0,
    }
}

fn compute_cagr(initial: f64, final_val: f64, years: f64) -> f64 {
    if initial <= 0.0 || final_val <= 0.0 || years <= 0.0 { return 0.0; }
    (final_val / initial).powf(1.0 / years) - 1.0
}

/// Annualised Sharpe ratio (risk-free = 0) from the equity curve.
fn compute_sharpe(curve: &[EquityPoint], years: f64) -> f64 {
    if curve.len() < 2 || years <= 0.0 { return 0.0; }
    let rets: Vec<f64> = curve.windows(2)
        .filter_map(|w| {
            if w[0].equity > 0.0 { Some(w[1].equity / w[0].equity - 1.0) } else { None }
        })
        .collect();
    if rets.len() < 2 { return 0.0; }
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var  = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (rets.len() - 1) as f64;
    let std  = var.sqrt();
    if std == 0.0 { return 0.0; }
    let periods_per_year = rets.len() as f64 / years;
    (mean / std) * periods_per_year.sqrt()
}

/// Compute buy-and-hold benchmark over the tick candles.
/// Starts with `initial_balance`, buys at first close, holds to last close.
fn compute_benchmark(initial_balance: f64, candles: &[Candle]) -> Benchmark {
    if candles.len() < 2 || candles[0].close <= 0.0 {
        return Benchmark { final_equity: initial_balance, cagr: 0.0, max_drawdown: 0.0, max_drawdown_pct: 0.0 };
    }
    let entry = candles[0].close;
    let size  = initial_balance / entry;

    let mut peak = initial_balance;
    let mut max_dd = 0.0;
    for c in candles {
        let eq = size * c.close;
        if eq > peak { peak = eq; }
        let dd = peak - eq;
        if dd > max_dd { max_dd = dd; }
    }
    let final_equity = size * candles.last().unwrap().close;
    let years = (candles.last().unwrap().timestamp - candles[0].timestamp) as f64 / MS_PER_YEAR;
    let cagr = compute_cagr(initial_balance, final_equity, years);
    let max_dd_pct = if peak > 0.0 { max_dd / peak } else { 0.0 };

    Benchmark { final_equity, cagr, max_drawdown: max_dd, max_drawdown_pct: max_dd_pct }
}

// ── Top-level runner ──────────────────────────────────────────────────────────

/// Run an end-to-end backtest: the first `config.warmup_bars` candles are
/// pushed into the engine *without* ticking; the rest are ticked through the
/// `InMemoryExecutor`.
pub fn run_backtest(
    engine:  &mut Engine,
    candles: Vec<Candle>,
    config:  BacktestConfig,
) -> Result<BacktestResult> {
    let mut exec = InMemoryExecutor::new(config.initial_balance);

    let warmup_n = config.warmup_bars.min(candles.len());
    let (warmup, ticks) = candles.split_at(warmup_n);

    for c in warmup {
        engine.push_candle(c.clone());
    }

    for c in ticks {
        let ctx      = exec.context(c.close);
        let decision = engine.tick(c.clone(), ctx)
            .map_err(|e| anyhow::anyhow!("engine tick error: {e}"))?;
        exec.apply(c, &decision);
    }

    let metrics   = exec.metrics(config.initial_balance);
    let benchmark = compute_benchmark(config.initial_balance, ticks);
    Ok(BacktestResult {
        trades:        exec.trades,
        equity_curve:  exec.equity_curve,
        metrics,
        benchmark,
        final_balance: exec.balance,
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shared::{Signal, TradeDecision};

    fn make_candle(ts: i64, close: f64) -> Candle {
        Candle {
            timestamp: ts,
            symbol:    "TEST".into(),
            open:      close - 0.5,
            high:      close + 1.0,
            low:       close - 1.0,
            close,
            volume:    1000.0,
            timeframe: "1d".into(),
        }
    }

    fn buy()  -> TradeDecision { TradeDecision { signal: Signal::Buy,  size: 1.0, stop_loss: None, take_profit: None, reason: None } }
    fn sell() -> TradeDecision { TradeDecision { signal: Signal::Sell, size: 0.0, stop_loss: None, take_profit: None, reason: None } }
    fn short()-> TradeDecision { TradeDecision { signal: Signal::Short,size: 1.0, stop_loss: None, take_profit: None, reason: None } }
    fn cover()-> TradeDecision { TradeDecision { signal: Signal::Cover,size: 0.0, stop_loss: None, take_profit: None, reason: None } }

    #[test]
    fn long_buy_sell_profits() {
        let mut e = InMemoryExecutor::new(1_000.0);
        e.apply(&make_candle(1, 100.0), &buy());
        let size = e.position().unwrap().size;
        e.apply(&make_candle(2, 110.0), &sell());
        assert_eq!(e.trades.len(), 1);
        assert!((e.trades[0].pnl - (110.0 - 100.0) * size).abs() < 1e-9);
        assert!(e.position().is_none());
    }

    #[test]
    fn short_cover_profits_on_drop() {
        let mut e = InMemoryExecutor::new(1_000.0);
        e.apply(&make_candle(1, 100.0), &short());
        let size = e.position().unwrap().size;
        e.apply(&make_candle(2,  90.0), &cover());
        assert_eq!(e.trades.len(), 1);
        assert!((e.trades[0].pnl - (100.0 - 90.0) * size).abs() < 1e-9);
    }

    #[test]
    fn equity_curve_grows_with_each_tick() {
        let mut e = InMemoryExecutor::new(1_000.0);
        e.apply(&make_candle(1, 100.0), &TradeDecision::hold());
        e.apply(&make_candle(2, 101.0), &TradeDecision::hold());
        assert_eq!(e.equity_curve.len(), 2);
    }

    #[test]
    fn max_drawdown_tracks_peak_to_trough() {
        let mut e = InMemoryExecutor::new(1_000.0);
        e.apply(&make_candle(1, 100.0), &buy());
        e.apply(&make_candle(2, 120.0), &TradeDecision::hold()); // peak
        e.apply(&make_candle(3,  80.0), &TradeDecision::hold()); // trough
        let m = e.metrics(1_000.0);
        assert!(m.max_drawdown > 0.0);
        assert!(m.peak_equity > 1_000.0);
    }

    #[test]
    fn stop_loss_closes_long() {
        let mut e = InMemoryExecutor::new(1_000.0);
        let entry = make_candle(1, 100.0);
        let d = TradeDecision { signal: Signal::Buy, size: 1.0, stop_loss: Some(95.0), take_profit: None, reason: None };
        e.apply(&entry, &d);
        // Next candle wick hits SL
        let mut wick = make_candle(2, 96.0);
        wick.low = 90.0;
        e.apply(&wick, &TradeDecision::hold());
        assert_eq!(e.trades.len(), 1);
        assert_eq!(e.trades[0].exit_reason, "stop-loss triggered");
    }
}
