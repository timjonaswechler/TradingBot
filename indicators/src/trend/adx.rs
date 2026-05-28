use shared::Candle;

/// Result of the ADX calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct AdxResult {
    /// Average Directional Index (0-100) — trend strength, no direction.
    pub adx: f64,
    /// +DI — positive directional indicator.
    pub plus_di: f64,
    /// -DI — negative directional indicator.
    pub minus_di: f64,
}

/// Average Directional Index (ADX).
///
/// Input: candles in chronological order (oldest first).
/// Needs at least `2 * period` bars.
pub fn adx(candles: &[Candle], period: usize) -> Option<AdxResult> {
    if period == 0 || candles.len() < 2 * period {
        return None;
    }

    let n = candles.len();

    // True Range and directional movement for each bar (starting at index 1)
    let mut tr_vals = Vec::with_capacity(n - 1);
    let mut pdm_vals = Vec::with_capacity(n - 1);
    let mut ndm_vals = Vec::with_capacity(n - 1);

    for i in 1..n {
        let cur = &candles[i];
        let prev = &candles[i - 1];

        let tr = (cur.high - cur.low)
            .max((cur.high - prev.close).abs())
            .max((cur.low - prev.close).abs());

        let up = cur.high - prev.high;
        let down = prev.low - cur.low;

        let pdm = if up > down && up > 0.0 { up } else { 0.0 };
        let ndm = if down > up && down > 0.0 { down } else { 0.0 };

        tr_vals.push(tr);
        pdm_vals.push(pdm);
        ndm_vals.push(ndm);
    }

    // Wilder smooth of TR, +DM, -DM
    let smooth = |vals: &[f64]| -> Vec<f64> {
        let mut s = vals[..period].iter().sum::<f64>();
        let mut out = vec![s];
        for &v in &vals[period..] {
            s = s - s / period as f64 + v;
            out.push(s);
        }
        out
    };

    let atr = smooth(&tr_vals);
    let apdm = smooth(&pdm_vals);
    let andm = smooth(&ndm_vals);

    // DI series
    let di_plus: Vec<f64> = apdm
        .iter()
        .zip(atr.iter())
        .map(|(p, t)| 100.0 * p / t)
        .collect();
    let di_minus: Vec<f64> = andm
        .iter()
        .zip(atr.iter())
        .map(|(n, t)| 100.0 * n / t)
        .collect();

    // DX series
    let dx: Vec<f64> = di_plus
        .iter()
        .zip(di_minus.iter())
        .map(|(p, n)| {
            let sum = p + n;
            if sum == 0.0 {
                0.0
            } else {
                100.0 * (p - n).abs() / sum
            }
        })
        .collect();

    // ADX = Wilder smooth of DX over `period`
    if dx.len() < period {
        return None;
    }
    let mut adx_val: f64 = dx[..period].iter().sum::<f64>() / period as f64;
    for &d in &dx[period..] {
        adx_val = (adx_val * (period as f64 - 1.0) + d) / period as f64;
    }

    Some(AdxResult {
        adx: adx_val,
        plus_di: *di_plus.last()?,
        minus_di: *di_minus.last()?,
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
        let c: Vec<Candle> = (1..=27)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        assert_eq!(adx(&c, 14), None); // needs 28
    }

    #[test]
    fn computes_without_panic() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        assert!(adx(&c, 14).is_some());
    }

    #[test]
    fn adx_in_range() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 1.0, i as f64 - 1.0, i as f64))
            .collect();
        let r = adx(&c, 14).unwrap();
        assert!(r.adx >= 0.0 && r.adx <= 100.0);
        assert!(r.plus_di >= 0.0);
        assert!(r.minus_di >= 0.0);
    }

    #[test]
    fn strong_uptrend_high_adx() {
        let c: Vec<Candle> = (1..=60)
            .map(|i| candle(i as f64 + 0.5, i as f64 - 0.5, i as f64))
            .collect();
        let r = adx(&c, 14).unwrap();
        assert!(
            r.adx > 25.0,
            "ADX {:.1} should indicate strong trend",
            r.adx
        );
        assert!(r.plus_di > r.minus_di, "+DI should dominate in uptrend");
    }
}
