use std::collections::HashMap;

/// Lazy, incremental indicator cache.
///
/// Stores running state for indicators that support O(1) Wilder-style updates
/// (EMA, RSI, ATR). Everything else recomputes from the candle window when called.
///
/// Entries are keyed by `period` and track how many candles were present
/// when the state was last computed so the binding can decide:
/// - same count  → return cached value
/// - count + 1   → O(1) incremental update
/// - anything else → full recompute (cold start or offset access)
#[derive(Debug, Default)]
pub struct IndicatorCache {
    /// EMA: period → (value, candle_count_at_computation)
    pub ema: HashMap<usize, (f64, usize)>,

    /// RSI Wilder state: period → (avg_gain, avg_loss, prev_close, candle_count)
    pub rsi: HashMap<usize, (f64, f64, f64, usize)>,

    /// ATR Wilder state: period → (atr_value, prev_close, candle_count)
    pub atr: HashMap<usize, (f64, f64, usize)>,
}

impl IndicatorCache {
    pub fn new() -> Self {
        Self::default()
    }
}

// ── EMA helpers ─────────────────────────────────────────────────────────────

/// Try to return an EMA value using cached state.
///
/// Returns `Some(value)` if the cache had a valid entry and the update succeeded.
/// Returns `None` if a full recompute is needed (cold start).
pub fn ema_from_cache(cache: &mut IndicatorCache, closes: &[f64], period: usize) -> Option<f64> {
    let k = 2.0 / (period as f64 + 1.0);
    let n = closes.len();

    match cache.ema.get(&period).copied() {
        Some((val, last_n)) if last_n == n => {
            // Same data — nothing changed, return as-is.
            Some(val)
        }
        Some((val, last_n)) if last_n + 1 == n => {
            // One new candle appended — O(1) update.
            let new_val = closes[n - 1] * k + val * (1.0 - k);
            cache.ema.insert(period, (new_val, n));
            Some(new_val)
        }
        _ => None, // cold start or unexpected gap → caller does full recompute
    }
}

/// Store a freshly computed EMA value in the cache.
pub fn ema_store(cache: &mut IndicatorCache, period: usize, value: f64, n: usize) {
    cache.ema.insert(period, (value, n));
}

// ── RSI helpers ──────────────────────────────────────────────────────────────

pub fn rsi_from_cache(cache: &mut IndicatorCache, closes: &[f64], period: usize) -> Option<f64> {
    let n = closes.len();

    match cache.rsi.get(&period).copied() {
        Some((ag, al, _pc, last_n)) if last_n == n => {
            // Same data.
            if al < 1e-12 {
                return Some(100.0);
            }
            Some(100.0 - 100.0 / (1.0 + ag / al))
        }
        Some((ag, al, pc, last_n)) if last_n + 1 == n => {
            // One new candle.
            let change = closes[n - 1] - pc;
            let gain = if change > 0.0 { change } else { 0.0 };
            let loss = if change < 0.0 { change.abs() } else { 0.0 };
            let p = period as f64;
            let new_ag = (ag * (p - 1.0) + gain) / p;
            let new_al = (al * (p - 1.0) + loss) / p;
            cache.rsi.insert(period, (new_ag, new_al, closes[n - 1], n));
            if new_al < 1e-12 {
                return Some(100.0);
            }
            Some(100.0 - 100.0 / (1.0 + new_ag / new_al))
        }
        _ => None,
    }
}

pub fn rsi_store(
    cache: &mut IndicatorCache,
    period: usize,
    avg_gain: f64,
    avg_loss: f64,
    prev_close: f64,
    n: usize,
) {
    cache
        .rsi
        .insert(period, (avg_gain, avg_loss, prev_close, n));
}

// ── ATR helpers ──────────────────────────────────────────────────────────────

pub fn atr_from_cache(
    cache: &mut IndicatorCache,
    candles: &[shared::Candle],
    period: usize,
) -> Option<f64> {
    let n = candles.len();

    match cache.atr.get(&period).copied() {
        Some((val, _pc, last_n)) if last_n == n => Some(val),
        Some((val, pc, last_n)) if last_n + 1 == n => {
            let cur = &candles[n - 1];
            let tr = (cur.high - cur.low)
                .max((cur.high - pc).abs())
                .max((cur.low - pc).abs());
            let p = period as f64;
            let new_val = (val * (p - 1.0) + tr) / p;
            cache.atr.insert(period, (new_val, cur.close, n));
            Some(new_val)
        }
        _ => None,
    }
}

pub fn atr_store(cache: &mut IndicatorCache, period: usize, value: f64, prev_close: f64, n: usize) {
    cache.atr.insert(period, (value, prev_close, n));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ema_cache_hit_same_data() {
        let mut cache = IndicatorCache::new();
        cache.ema.insert(3, (10.0, 5));
        let closes = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(ema_from_cache(&mut cache, &closes, 3), Some(10.0));
    }

    #[test]
    fn ema_cache_incremental_update() {
        // Seed: EMA(3) of [1,2,3] = 2.0 at n=3; k=0.5
        // New close = 4.0 → new EMA = 4*0.5 + 2*0.5 = 3.0
        let mut cache = IndicatorCache::new();
        cache.ema.insert(3, (2.0, 3));
        let closes = vec![1.0, 2.0, 3.0, 4.0];
        let result = ema_from_cache(&mut cache, &closes, 3).unwrap();
        assert!((result - 3.0).abs() < 1e-10);
        // Cache should now be updated to n=4
        assert_eq!(cache.ema[&3].1, 4);
    }

    #[test]
    fn ema_cache_cold_start_returns_none() {
        let mut cache = IndicatorCache::new();
        let closes = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert_eq!(ema_from_cache(&mut cache, &closes, 3), None);
    }

    #[test]
    fn rsi_cache_incremental_update() {
        let mut cache = IndicatorCache::new();
        // Seed with some state (avg_gain=1.0, avg_loss=0.5, prev_close=10.0, n=15)
        cache.rsi.insert(14, (1.0, 0.5, 10.0, 15));
        // New close = 11.0 → change = +1.0
        let mut closes = vec![0.0f64; 15];
        closes.push(11.0);
        let result = rsi_from_cache(&mut cache, &closes, 14).unwrap();
        assert!(result > 50.0); // net gain → RSI above 50
        assert!(result >= 0.0 && result <= 100.0);
    }
}
