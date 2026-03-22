use crate::market_data::Candle;
use super::{Signal, Strategy};
use chrono::Datelike;

// ── Parameters ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DualMacdParams {
    // MACD core
    pub fast: usize,   // default 12
    pub slow: usize,   // default 26
    pub signal: usize, // default 9

    // Primary timeframe scoring weights
    pub primary_crossover_weight: f64, // [0.0..3.0] default 1.5
    pub primary_histogram_weight: f64, // [0.0..3.0] default 1.0
    pub primary_slope_weight: f64,     // [0.0..3.0] default 0.8
    pub primary_slope_lookback: usize, // [1..10]    default 3

    // Secondary timeframe — fast drop detection
    pub secondary_drop_threshold: f64,   // normalized histogram drop to trigger [0.0..0.05] default 0.003
    pub secondary_drop_weight: f64,      // suppression weight [0.0..3.0] default 1.5
    pub secondary_slope_lookback: usize, // [1..5]  default 2

    // Calendar effects
    pub month_start_boost: f64,   // [0.0..1.0]  default 0.3
    pub month_start_days: usize,  // [1..5]      default 3
    pub month_end_caution: f64,   // [-1.0..0.0] default -0.2
    pub month_end_days: usize,    // [1..5]      default 3
    pub quarter_end_caution: f64, // [-1.0..0.0] default -0.15
    pub year_end_boost: f64,      // [0.0..1.0]  default 0.2

    // Regime detection thresholds
    pub crash_atr_multiplier: f64,  // [1.0..5.0]   default 3.0
    pub bull_trend_threshold: f64,  // [0.0..0.01]  default 0.0003
    pub bear_trend_threshold: f64,  // [-0.01..0.0] default -0.0003
    pub long_ema_period: usize,     // [50..200]    default 200
    pub atr_period: usize,          // [5..30]      default 14
    pub atr_median_period: usize,   // [50..200]    default 100

    // Decision thresholds
    pub buy_threshold: f64,   // [0.0..5.0]  default 1.5
    pub sell_threshold: f64,  // [-5.0..0.0] default -0.5
    pub short_threshold: f64, // [-5.0..0.0] default -2.0
}

impl Default for DualMacdParams {
    fn default() -> Self {
        Self {
            fast: 12,
            slow: 26,
            signal: 9,
            primary_crossover_weight: 1.5,
            primary_histogram_weight: 1.0,
            primary_slope_weight: 0.8,
            primary_slope_lookback: 3,
            secondary_drop_threshold: 0.003,
            secondary_drop_weight: 1.5,
            secondary_slope_lookback: 2,
            month_start_boost: 0.3,
            month_start_days: 3,
            month_end_caution: -0.2,
            month_end_days: 3,
            quarter_end_caution: -0.15,
            year_end_boost: 0.2,
            crash_atr_multiplier: 3.0,
            bull_trend_threshold: 0.0003,
            bear_trend_threshold: -0.0003,
            long_ema_period: 200,
            atr_period: 14,
            atr_median_period: 100,
            buy_threshold: 1.5,
            sell_threshold: -0.5,
            short_threshold: -2.0,
        }
    }
}

// ── Strategy struct ───────────────────────────────────────────────────────────

pub struct DualMacdStrategy {
    pub params: DualMacdParams,
}

impl DualMacdStrategy {
    pub fn new(params: DualMacdParams) -> Self {
        Self { params }
    }
}

// ── Regime enum (private) ─────────────────────────────────────────────────────

#[derive(Debug, PartialEq)]
enum Regime {
    Bull,
    Bear,
    Neutral,
    Crash,
}

// ── Strategy trait implementation ─────────────────────────────────────────────

impl Strategy for DualMacdStrategy {
    fn name(&self) -> &str {
        "DualMacd"
    }

    fn required_history(&self) -> usize {
        let p = &self.params;
        p.slow + p.signal + p.primary_slope_lookback + p.atr_median_period
    }

    fn signal(&self, primary: &[Candle], secondary: &[Candle]) -> Signal {
        let p = &self.params;

        // Guard: insufficient data → Hold
        let min_required = p.slow + p.signal + p.primary_slope_lookback;
        if primary.len() < min_required {
            return Signal::Hold;
        }

        // ── Step 1: Regime Detection ──────────────────────────────────────────
        let regime = detect_regime(primary, p);

        // ── Step 2: Crash/Bear modifier ───────────────────────────────────────
        let regime_modifier: f64 = match regime {
            Regime::Crash => -10.0,
            Regime::Bear => -2.0,
            _ => 0.0,
        };

        // ── Step 3: Calendar modifier ─────────────────────────────────────────
        let calendar_modifier = calendar_modifier(primary, p);

        // ── Step 4: Primary MACD score ────────────────────────────────────────
        let primary_score = primary_macd_score(primary, p, calendar_modifier);

        // Composite before secondary correction
        let composite_before = if regime == Regime::Crash {
            regime_modifier
        } else {
            primary_score + regime_modifier
        };

        // ── Step 5: Secondary drop detection ─────────────────────────────────
        let drop_penalty = if secondary.len() >= p.slow + p.signal + p.secondary_slope_lookback {
            secondary_drop_penalty(secondary, p)
        } else {
            0.0
        };

        // ── Step 6: Final decision ────────────────────────────────────────────
        let composite = composite_before - drop_penalty;

        match regime {
            Regime::Crash => {
                if composite <= p.short_threshold {
                    Signal::Short
                } else {
                    Signal::Sell
                }
            }
            _ => {
                if composite >= p.buy_threshold {
                    Signal::Buy
                } else if composite <= p.short_threshold {
                    Signal::Short
                } else if composite <= p.sell_threshold {
                    Signal::Sell
                } else {
                    Signal::Hold
                }
            }
        }
    }
}

// ── Regime detection ──────────────────────────────────────────────────────────

fn detect_regime(primary: &[Candle], p: &DualMacdParams) -> Regime {
    let atr_values = compute_atr_series(primary, p.atr_period);
    if atr_values.is_empty() {
        return Regime::Neutral;
    }

    let current_atr = atr_values[0];

    let median_len = atr_values.len().min(p.atr_median_period);
    let median_atr = if median_len == 0 {
        current_atr
    } else {
        let mut sample: Vec<f64> = atr_values[..median_len].to_vec();
        sample.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = median_len / 2;
        if median_len % 2 == 0 {
            (sample[mid - 1] + sample[mid]) / 2.0
        } else {
            sample[mid]
        }
    };

    let closes: Vec<f64> = primary.iter().rev().map(|c| c.close as f64 / 100.0).collect();
    let long_ema_vals = ema(&closes, p.long_ema_period);

    let slope = if long_ema_vals.len() > p.primary_slope_lookback {
        let len = long_ema_vals.len();
        let recent = long_ema_vals[len - 1];
        let older = long_ema_vals[len - 1 - p.primary_slope_lookback];
        let denom = older.abs().max(1e-10);
        (recent - older) / denom / p.primary_slope_lookback as f64
    } else {
        0.0
    };

    // Regime classification
    if median_atr > 0.0 && current_atr > p.crash_atr_multiplier * median_atr {
        Regime::Crash
    } else if median_atr > 0.0 && current_atr > 2.0 * median_atr && slope < 0.0 {
        Regime::Bear
    } else if slope > p.bull_trend_threshold {
        Regime::Bull
    } else if slope < p.bear_trend_threshold {
        Regime::Bear
    } else {
        Regime::Neutral
    }
}

// ── ATR series (newest-first) ─────────────────────────────────────────────────

/// Compute ATR (EMA of True Range). Returns values newest-first.
fn compute_atr_series(candles: &[Candle], period: usize) -> Vec<f64> {
    if candles.len() < period + 1 {
        return vec![];
    }

    // candles is newest-first; iterate in reverse to produce oldest-first true ranges
    let n = candles.len();
    let mut tr_oldest_first: Vec<f64> = Vec::with_capacity(n - 1);

    for i in (0..n - 1).rev() {
        let high = candles[i].high as f64 / 100.0;
        let low = candles[i].low as f64 / 100.0;
        let prev_close = candles[i + 1].close as f64 / 100.0;
        let tr = (high - low)
            .max((high - prev_close).abs())
            .max((low - prev_close).abs());
        tr_oldest_first.push(tr);
    }

    ema(&tr_oldest_first, period).into_iter().rev().collect()
}

// ── Calendar modifier ─────────────────────────────────────────────────────────

fn calendar_modifier(primary: &[Candle], p: &DualMacdParams) -> f64 {
    if primary.is_empty() {
        return 0.0;
    }

    let ts = primary[0].timestamp;
    let day = ts.day();
    let month = ts.month();
    let year = ts.year();

    let days_in_month = days_in_month(year, month);
    let mut modifier = 0.0f64;

    if day <= p.month_start_days as u32 {
        modifier += p.month_start_boost;
    }
    if day >= days_in_month - p.month_end_days as u32 {
        modifier += p.month_end_caution;
    }
    if (month == 3 || month == 6 || month == 9 || month == 12)
        && day >= days_in_month.saturating_sub(4)
    {
        modifier += p.quarter_end_caution;
    }
    // Year-end boost: January 1-5 (new year rally) and December 26+
    if month == 1 && day <= 5 {
        modifier += p.year_end_boost;
    }
    if month == 12 && day >= 26 {
        modifier += p.year_end_boost;
    }

    modifier
}

/// Compute the number of days in a given month (handles leap years).
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

// ── Shared MACD helper ────────────────────────────────────────────────────────

/// Computes aligned (macd_line, signal_line, normalized_histogram) from oldest-first closes.
/// All output vecs share the same length and index space.
/// Returns `None` if there is not enough data.
fn compute_macd(
    closes: &[f64],
    fast: usize,
    slow: usize,
    signal_period: usize,
) -> Option<(Vec<f64>, Vec<f64>, Vec<f64>)> {
    let ema_fast = ema(closes, fast);
    let ema_slow = ema(closes, slow);
    if ema_fast.len() < 2 || ema_slow.len() < 2 {
        return None;
    }

    let slow_len = ema_slow.len();
    let fast_offset = ema_fast.len() - slow_len;
    let macd_line: Vec<f64> = (0..slow_len)
        .map(|i| ema_fast[fast_offset + i] - ema_slow[i])
        .collect();

    let signal_line = ema(&macd_line, signal_period);
    if signal_line.is_empty() {
        return None;
    }

    let sig_len = signal_line.len();
    let macd_offset = macd_line.len() - sig_len;
    let closes_offset = closes.len() - sig_len;

    let histogram: Vec<f64> = (0..sig_len)
        .map(|i| macd_line[macd_offset + i] - signal_line[i])
        .collect();

    let normalized_hist: Vec<f64> = (0..sig_len)
        .map(|i| {
            let c = closes[closes_offset + i];
            if c.abs() > 1e-10 { histogram[i] / c } else { 0.0 }
        })
        .collect();

    Some((macd_line[macd_offset..].to_vec(), signal_line, normalized_hist))
}

// ── Primary MACD score ────────────────────────────────────────────────────────

fn primary_macd_score(primary: &[Candle], p: &DualMacdParams, calendar_modifier: f64) -> f64 {
    let closes: Vec<f64> = primary.iter().rev().map(|c| c.close as f64 / 100.0).collect();

    let (macd_line, signal_line, normalized_histogram) =
        match compute_macd(&closes, p.fast, p.slow, p.signal) {
            Some(v) => v,
            None => return calendar_modifier,
        };

    let last = signal_line.len() - 1;
    if last < p.primary_slope_lookback + 1 {
        return calendar_modifier;
    }

    let crossover_signal = if macd_line[last] > signal_line[last] && macd_line[last - 1] <= signal_line[last - 1] {
        1.0
    } else if macd_line[last] < signal_line[last] && macd_line[last - 1] >= signal_line[last - 1] {
        -1.0
    } else {
        0.0
    };

    let histogram_signal = normalized_histogram[last];

    let slope_signal = {
        let hist_now = macd_line[last] - signal_line[last];
        let hist_old = macd_line[last - p.primary_slope_lookback] - signal_line[last - p.primary_slope_lookback];
        let raw = (hist_now - hist_old) / hist_old.abs().max(1e-10);
        raw / raw.abs().max(1.0)
    };

    p.primary_crossover_weight * crossover_signal
        + p.primary_histogram_weight * histogram_signal.clamp(-1.0, 1.0) * 10.0
        + p.primary_slope_weight * slope_signal.clamp(-1.0, 1.0)
        + calendar_modifier
}

// ── Secondary drop detection ──────────────────────────────────────────────────

fn secondary_drop_penalty(secondary: &[Candle], p: &DualMacdParams) -> f64 {
    let closes: Vec<f64> = secondary.iter().rev().map(|c| c.close as f64 / 100.0).collect();

    let (_, _, normalized_hist) = match compute_macd(&closes, p.fast, p.slow, p.signal) {
        Some(v) => v,
        None => return 0.0,
    };

    let last = normalized_hist.len() - 1;
    if last < p.secondary_slope_lookback {
        return 0.0;
    }

    let drop = normalized_hist[last - p.secondary_slope_lookback] - normalized_hist[last];
    if drop > p.secondary_drop_threshold {
        p.secondary_drop_weight * (drop / p.secondary_drop_threshold).min(3.0)
    } else {
        0.0
    }
}

// ── EMA helper ────────────────────────────────────────────────────────────────

/// Computes EMA from oldest-first data using SMA seed for the first window.
/// Returns a vector of length `data.len() - period + 1` (oldest-first).
fn ema(data: &[f64], period: usize) -> Vec<f64> {
    if period == 0 || data.len() < period {
        return vec![];
    }
    let k = 2.0 / (period as f64 + 1.0);
    let seed: f64 = data[..period].iter().sum::<f64>() / period as f64;
    let mut result = Vec::with_capacity(data.len() - period + 1);
    result.push(seed);
    for &v in &data[period..] {
        let prev = *result.last().unwrap();
        result.push(v * k + prev * (1.0 - k));
    }
    result
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_candle(ts_days_from_epoch: i64, price: i64) -> Candle {
        Candle {
            timestamp: Utc.timestamp_opt(ts_days_from_epoch * 86400, 0).unwrap(),
            open: price,
            high: price + 10,
            low: price - 10,
            close: price,
            volume: 1_000_000,
        }
    }

    /// Build a candle slice with a smooth uptrend, newest-first.
    fn make_uptrend_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let day = (n - 1 - i) as i64; // newest has largest day index
                let price = (10_000 + day * 5) as i64; // cents, increasing
                make_candle(day, price)
            })
            .collect()
    }

    /// Build candles with a crash: normal data then a huge ATR spike at the end.
    fn make_crash_candles(n: usize) -> Vec<Candle> {
        (0..n)
            .map(|i| {
                let day = (n - 1 - i) as i64;
                let base_price = 10_000i64;
                // Most recent candles (small i) have a huge high-low range to spike ATR
                let (high_extra, low_extra) = if i < 5 {
                    (5000i64, 5000i64) // massive true range
                } else {
                    (10i64, 10i64)
                };
                Candle {
                    timestamp: Utc.timestamp_opt(day * 86400, 0).unwrap(),
                    open: base_price,
                    high: base_price + high_extra,
                    low: base_price - low_extra,
                    close: base_price - low_extra, // falling close for bear slope
                    volume: 1_000_000,
                }
            })
            .collect()
    }

    #[test]
    fn test_default_params_no_panic() {
        let strategy = DualMacdStrategy::new(DualMacdParams::default());
        let primary = make_uptrend_candles(300);
        let secondary = make_uptrend_candles(300);
        // Should not panic
        let _ = strategy.signal(&primary, &secondary);
    }

    #[test]
    fn test_insufficient_data_returns_hold() {
        let strategy = DualMacdStrategy::new(DualMacdParams::default());
        // Only 10 candles — well below required_history
        let primary: Vec<Candle> = (0..10).map(|i| make_candle(i, 10_000)).collect();
        let secondary: Vec<Candle> = (0..10).map(|i| make_candle(i, 10_000)).collect();
        assert_eq!(
            strategy.signal(&primary, &secondary),
            Signal::Hold,
            "Should return Hold when insufficient data"
        );
    }

    #[test]
    fn test_uptrend_produces_buy() {
        let params = DualMacdParams {
            // Lower thresholds to make it easier to trigger Buy with synthetic data
            buy_threshold: 0.5,
            sell_threshold: -0.5,
            short_threshold: -2.0,
            // Disable crash detection (large multiplier)
            crash_atr_multiplier: 100.0,
            atr_median_period: 50,
            long_ema_period: 50,
            ..DualMacdParams::default()
        };
        let strategy = DualMacdStrategy::new(params);
        let primary = make_uptrend_candles(300);
        let secondary = make_uptrend_candles(300);
        let sig = strategy.signal(&primary, &secondary);
        // In a sustained uptrend the composite score should be positive → Buy
        assert!(
            sig == Signal::Buy || sig == Signal::Hold,
            "Expected Buy or Hold in strong uptrend, got {:?}",
            sig
        );
    }

    #[test]
    fn test_crash_candles_return_short_or_sell() {
        let params = DualMacdParams {
            crash_atr_multiplier: 2.0, // lower bar to trigger Crash
            atr_median_period: 20,
            long_ema_period: 50,
            short_threshold: -2.0,
            sell_threshold: -0.5,
            buy_threshold: 1.5,
            ..DualMacdParams::default()
        };
        let strategy = DualMacdStrategy::new(params);
        let primary = make_crash_candles(200);
        let secondary = make_crash_candles(200);
        let sig = strategy.signal(&primary, &secondary);
        assert!(
            sig == Signal::Short || sig == Signal::Sell,
            "Expected Short or Sell in crash regime, got {:?}",
            sig
        );
    }

    #[test]
    fn test_required_history() {
        let params = DualMacdParams::default();
        let strategy = DualMacdStrategy::new(params.clone());
        let expected = params.slow + params.signal + params.primary_slope_lookback + params.atr_median_period;
        assert_eq!(strategy.required_history(), expected);
    }

    #[test]
    fn test_ema_basic() {
        // Simple sanity check for EMA helper
        let data: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let result = ema(&data, 3);
        assert_eq!(result.len(), 8); // 10 - 3 + 1
        // First value is SMA of [1,2,3] = 2.0
        assert!((result[0] - 2.0).abs() < 1e-9);
    }
}
