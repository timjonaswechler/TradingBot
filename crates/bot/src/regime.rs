use crate::market_data::Candle;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Regime {
    /// Extreme volatility spike + rapid drawdown. ATR >> historical median.
    Crash,
    /// Downtrend with elevated volatility.
    Bear,
    /// Normal conditions, no strong trend signal.
    Neutral,
    /// Uptrend with moderate volatility.
    Bull,
}

#[derive(Debug, Clone)]
pub struct RegimeParams {
    /// ATR > crash_atr_multiplier * median_atr → Crash. Default: 3.0
    pub crash_atr_multiplier: f64,
    /// ATR > bear_atr_multiplier * median_atr → Bear. Default: 2.0
    pub bear_atr_multiplier: f64,
    /// Long EMA slope > this → Bull. Default: 0.0003 (0.03% per bar)
    pub bull_trend_threshold: f64,
    /// Long EMA slope < this → Bear trend component. Default: -0.0003
    pub bear_trend_threshold: f64,
    /// Period for ATR calculation. Default: 14
    pub atr_period: usize,
    /// Lookback period for computing median ATR. Default: 100
    pub atr_median_period: usize,
    /// Period for long EMA used to determine trend. Default: 200
    pub long_ema_period: usize,
    /// Slope lookback for EMA trend. Default: 5
    pub slope_lookback: usize,
}

impl Default for RegimeParams {
    fn default() -> Self {
        Self {
            crash_atr_multiplier: 3.0,
            bear_atr_multiplier: 2.0,
            bull_trend_threshold: 0.0003,
            bear_trend_threshold: -0.0003,
            atr_period: 14,
            atr_median_period: 100,
            long_ema_period: 200,
            slope_lookback: 5,
        }
    }
}

impl RegimeParams {
    fn min_candles(&self) -> usize {
        (self.long_ema_period + self.slope_lookback).max(self.atr_period + 1)
    }
}

// ---------------------------------------------------------------------------
// Inline helpers (intentionally self-contained; do not depend on other crates)
// ---------------------------------------------------------------------------

/// EMA series (oldest-first output) seeded with SMA of the first `period` bars.
fn compute_ema(prices: &[f64], period: usize) -> Vec<f64> {
    if prices.len() < period {
        return vec![];
    }
    let alpha = 2.0 / (period as f64 + 1.0);
    let seed: f64 = prices[..period].iter().sum::<f64>() / period as f64;
    let mut ema = Vec::with_capacity(prices.len() - period + 1);
    ema.push(seed);
    for &p in &prices[period..] {
        let prev = *ema.last().unwrap();
        ema.push(prev + alpha * (p - prev));
    }
    ema
}

/// ATR series (oldest-first) via EMA of true range. Output starts at bar index 1.
fn compute_atr(high: &[f64], low: &[f64], close: &[f64], period: usize) -> Vec<f64> {
    if high.len() < period + 1 {
        return vec![];
    }
    let tr: Vec<f64> = (1..high.len())
        .map(|i| {
            let hl = high[i] - low[i];
            let hc = (high[i] - close[i - 1]).abs();
            let lc = (low[i] - close[i - 1]).abs();
            hl.max(hc).max(lc)
        })
        .collect();
    compute_ema(&tr, period)
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        (sorted[mid - 1] + sorted[mid]) / 2.0
    } else {
        sorted[mid]
    }
}

fn classify(
    atr_values: &[f64],
    atr_idx: usize,
    ema_values: &[f64],
    ema_idx: usize,
    params: &RegimeParams,
) -> Regime {
    let current_atr = atr_values[atr_idx];

    let window_start = atr_idx.saturating_sub(params.atr_median_period - 1);
    let window = &atr_values[window_start..=atr_idx];
    let med = if window.len() >= params.atr_median_period {
        median(window)
    } else {
        window.iter().sum::<f64>() / window.len() as f64
    };

    let slope = if ema_idx >= params.slope_lookback {
        let prev = ema_values[ema_idx - params.slope_lookback];
        if prev != 0.0 { (ema_values[ema_idx] - prev) / prev } else { 0.0 }
    } else {
        0.0
    };

    if current_atr > params.crash_atr_multiplier * med {
        Regime::Crash
    } else if current_atr > params.bear_atr_multiplier * med && slope < 0.0 {
        Regime::Bear
    } else if slope > params.bull_trend_threshold {
        Regime::Bull
    } else if slope < params.bear_trend_threshold {
        Regime::Bear
    } else {
        Regime::Neutral
    }
}

/// Reverse a newest-first candle slice into three oldest-first f64 arrays (high, low, close).
fn candles_to_oldest_first(candles: &[Candle]) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let n = candles.len();
    let mut high = vec![0.0f64; n];
    let mut low = vec![0.0f64; n];
    let mut close = vec![0.0f64; n];
    for (i, c) in candles.iter().enumerate() {
        let j = n - 1 - i;
        high[j] = c.high as f64 / 100.0;
        low[j] = c.low as f64 / 100.0;
        close[j] = c.close as f64 / 100.0;
    }
    (high, low, close)
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Detect current market regime from candles (newest-first).
/// Uses ATR-based volatility AND long-EMA trend direction.
/// Returns regime for the MOST RECENT bar (candles[0]).
pub fn detect_regime(candles: &[Candle], params: &RegimeParams) -> Regime {
    if candles.len() < params.min_candles() {
        return Regime::Neutral;
    }

    let (high, low, close) = candles_to_oldest_first(candles);
    let atr_values = compute_atr(&high, &low, &close, params.atr_period);
    let ema_values = compute_ema(&close, params.long_ema_period);

    if atr_values.is_empty() || ema_values.len() < params.slope_lookback + 1 {
        return Regime::Neutral;
    }

    classify(
        &atr_values, atr_values.len() - 1,
        &ema_values, ema_values.len() - 1,
        params,
    )
}

/// Returns the regime history for all bars (oldest-first output, matching chronological order).
/// candles input is newest-first; output is oldest-first.
pub fn detect_regime_series(candles: &[Candle], params: &RegimeParams) -> Vec<Regime> {
    let n = candles.len();
    let min_len = params.min_candles();

    let (high, low, close) = candles_to_oldest_first(candles);
    let atr_values = compute_atr(&high, &low, &close, params.atr_period);
    let ema_values = compute_ema(&close, params.long_ema_period);

    if atr_values.is_empty() || ema_values.is_empty() {
        return vec![Regime::Neutral; n];
    }

    // Alignment: atr_values[i] covers close[i+1]; ema_values[i] covers close[i + ema_period - 1].
    let ema_offset = params.long_ema_period - 1;

    let mut result = Vec::with_capacity(n);
    for b in 0..n {
        if b < min_len - 1 {
            result.push(Regime::Neutral);
            continue;
        }
        let atr_idx = match b.checked_sub(1) {
            Some(idx) if idx < atr_values.len() => idx,
            _ => { result.push(Regime::Neutral); continue; }
        };
        let ema_idx = match b.checked_sub(ema_offset) {
            Some(idx) if idx < ema_values.len() => idx,
            _ => { result.push(Regime::Neutral); continue; }
        };
        if ema_idx < params.slope_lookback {
            result.push(Regime::Neutral);
            continue;
        }
        result.push(classify(&atr_values, atr_idx, &ema_values, ema_idx, params));
    }
    result
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn make_candle(ts_secs: i64, open: i64, high: i64, low: i64, close: i64) -> Candle {
        Candle {
            timestamp: Utc.timestamp_opt(ts_secs, 0).unwrap(),
            open,
            high,
            low,
            close,
            volume: 1_000_000,
        }
    }

    /// Synthetic uptrend candles (newest-first). Price rises by `step_cents` per bar.
    fn uptrend_candles(n: usize, start_cents: i64, step_cents: i64) -> Vec<Candle> {
        let base_ts = 1_700_000_000i64;
        // Build oldest-first, then reverse so index 0 is newest.
        (0..n)
            .map(|i| {
                let price = start_cents + (i as i64) * step_cents;
                let spread = (price / 200).max(1);
                make_candle(
                    base_ts + i as i64 * 3600,
                    price - spread,
                    price + spread,
                    price - spread * 2,
                    price,
                )
            })
            .rev()
            .collect()
    }

    /// Flat candles at a fixed price (newest-first).
    fn stable_candles(n: usize, price_cents: i64) -> Vec<Candle> {
        let base_ts = 1_700_000_000i64;
        let spread = (price_cents / 1000).max(1);
        (0..n)
            .map(|i| make_candle(
                base_ts + i as i64 * 3600,
                price_cents - spread,
                price_cents + spread,
                price_cents - spread,
                price_cents,
            ))
            .rev()
            .collect()
    }

    // ------------------------------------------------------------------
    // Test 1: stable low-volatility → Neutral or Bull
    // ------------------------------------------------------------------
    #[test]
    fn test_stable_market_not_crash() {
        let params = RegimeParams::default();
        let candles = stable_candles(300, 10_000); // €100 per share
        let regime = detect_regime(&candles, &params);
        assert!(
            matches!(regime, Regime::Neutral | Regime::Bull),
            "Expected Neutral or Bull for stable market, got {:?}",
            regime
        );
    }

    // ------------------------------------------------------------------
    // Test 2: insert a huge volatility spike → Crash or Bear
    // ------------------------------------------------------------------
    #[test]
    fn test_spike_triggers_crash_or_bear() {
        let params = RegimeParams {
            atr_period: 14,
            atr_median_period: 100,
            long_ema_period: 50,
            slope_lookback: 5,
            crash_atr_multiplier: 3.0,
            bear_atr_multiplier: 2.0,
            ..Default::default()
        };
        let mut candles = stable_candles(200, 10_000);
        // Replace the most-recent candle with a massive spike.
        candles[0] = make_candle(
            1_700_000_000,
            10_000,
            25_000, // very high
            2_000,  // very low
            5_000,  // close well below open → crash candle
        );
        let regime = detect_regime(&candles, &params);
        assert!(
            matches!(regime, Regime::Crash | Regime::Bear),
            "Expected Crash or Bear after spike, got {:?}",
            regime
        );
    }

    // ------------------------------------------------------------------
    // Test 3: strong uptrend → Bull
    // ------------------------------------------------------------------
    #[test]
    fn test_uptrend_gives_bull() {
        let params = RegimeParams {
            long_ema_period: 50,
            slope_lookback: 5,
            atr_period: 14,
            atr_median_period: 50,
            bull_trend_threshold: 0.0001,
            ..Default::default()
        };
        // Start at €100 and rise by €0.50 per bar (0.5% per bar → well above threshold).
        let candles = uptrend_candles(200, 10_000, 50);
        let regime = detect_regime(&candles, &params);
        assert_eq!(
            regime,
            Regime::Bull,
            "Expected Bull for strong uptrend, got {:?}",
            regime
        );
    }

    // ------------------------------------------------------------------
    // Test 4: too few candles → Neutral (graceful degradation)
    // ------------------------------------------------------------------
    #[test]
    fn test_too_few_candles_is_neutral() {
        let params = RegimeParams::default(); // needs 205 bars minimum
        let candles = stable_candles(10, 10_000);
        assert_eq!(detect_regime(&candles, &params), Regime::Neutral);
    }

    // ------------------------------------------------------------------
    // Test 5: detect_regime_series output length == candles.len()
    // ------------------------------------------------------------------
    #[test]
    fn test_series_length_matches_input() {
        let params = RegimeParams {
            long_ema_period: 50,
            slope_lookback: 5,
            atr_period: 14,
            atr_median_period: 50,
            ..Default::default()
        };
        let candles = stable_candles(300, 10_000);
        let series = detect_regime_series(&candles, &params);
        assert_eq!(
            series.len(),
            candles.len(),
            "Series length must equal candle count"
        );
    }
}
