use shared::Candle;

/// Output of the Ichimoku Cloud calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct IchimokuResult {
    /// Tenkan-sen (Conversion Line) = (9-period high + 9-period low) / 2
    pub tenkan: f64,
    /// Kijun-sen (Base Line) = (26-period high + 26-period low) / 2
    pub kijun: f64,
    /// Senkou Span A = (tenkan + kijun) / 2  (plotted 26 bars ahead)
    pub span_a: f64,
    /// Senkou Span B = (52-period high + 52-period low) / 2  (plotted 26 bars ahead)
    pub span_b: f64,
    /// Chikou Span = current close shifted 26 bars back
    pub chikou: f64,
}

fn donchian_mid(candles: &[Candle], period: usize, end: usize) -> Option<f64> {
    let start = end.checked_sub(period)?;
    let slice = &candles[start..end];
    let high = slice
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = slice.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);
    Some((high + low) / 2.0)
}

/// Standard Ichimoku Cloud with default periods (9, 26, 52).
///
/// This returns the current-known, immediately usable Ichimoku values for the
/// latest bar, not a fully chart-shifted/projected window model.
///
/// TODO(#24)[https://github.com/timjonaswechler/TradingBot/issues/24]: Keep
/// this small top-level API, but consider a richer overload like
/// `ichimoku(candles, radius)` that adds a `.window` payload with
/// `current`/`past`/`future_cloud`/`meta`.
///
/// Input: candles in chronological order (oldest first).
/// Needs at least 52 bars.
pub fn ichimoku(candles: &[Candle]) -> Option<IchimokuResult> {
    ichimoku_custom(candles, 9, 26, 52)
}

/// Ichimoku with custom periods.
pub fn ichimoku_custom(
    candles: &[Candle],
    tenkan_period: usize,
    kijun_period: usize,
    senkou_b_period: usize,
) -> Option<IchimokuResult> {
    let n = candles.len();
    if n < senkou_b_period {
        return None;
    }

    let tenkan = donchian_mid(candles, tenkan_period, n)?;
    let kijun = donchian_mid(candles, kijun_period, n)?;
    let span_a = (tenkan + kijun) / 2.0;
    let span_b = donchian_mid(candles, senkou_b_period, n)?;

    // Chikou = close of current bar (would be plotted 26 bars back)
    let chikou = candles.last()?.close;

    Some(IchimokuResult {
        tenkan,
        kijun,
        span_a,
        span_b,
        chikou,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, c: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: c,
            volume: 1.0,
            timeframe: "1d".parse().unwrap(),
        }
    }

    #[test]
    fn insufficient_data() {
        let c: Vec<Candle> = (1..=51)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        assert_eq!(ichimoku(&c), None);
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        assert!(ichimoku(&c).is_some());
    }

    #[test]
    fn span_a_is_average_of_tenkan_and_kijun() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        let r = ichimoku(&c).unwrap();
        let expected = (r.tenkan + r.kijun) / 2.0;
        assert!((r.span_a - expected).abs() < 1e-10);
    }

    #[test]
    fn chikou_is_last_close() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        let r = ichimoku(&c).unwrap();
        assert_eq!(r.chikou, 60.0);
    }
}
