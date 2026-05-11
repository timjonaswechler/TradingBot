use shared::Candle;

/// Classic Pivot Point levels (floor pivots).
#[derive(Debug, Clone, PartialEq)]
pub struct PivotResult {
    pub pp: f64,
    pub r1: f64,
    pub r2: f64,
    pub r3: f64,
    pub s1: f64,
    pub s2: f64,
    pub s3: f64,
}

/// Classic Pivot Points from the **previous** completed candle (typically daily).
///
/// Pass the single previous-period candle. The result gives levels for the current period.
pub fn pivot_points(prev: &Candle) -> PivotResult {
    let pp = (prev.high + prev.low + prev.close) / 3.0;
    let r1 = 2.0 * pp - prev.low;
    let s1 = 2.0 * pp - prev.high;
    let r2 = pp + (prev.high - prev.low);
    let s2 = pp - (prev.high - prev.low);
    let r3 = prev.high + 2.0 * (pp - prev.low);
    let s3 = prev.low - 2.0 * (prev.high - pp);
    PivotResult {
        pp,
        r1,
        r2,
        r3,
        s1,
        s2,
        s3,
    }
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
            timeframe: "1d".into(),
        }
    }

    #[test]
    fn known_values() {
        // high=110, low=90, close=105 → PP=(110+90+105)/3 = 101.6667
        let c = candle(110.0, 90.0, 105.0);
        let p = pivot_points(&c);
        assert!((p.pp - 101.666_666_666_666_67).abs() < 1e-6);
        assert!((p.r1 - (2.0 * p.pp - 90.0)).abs() < 1e-10);
        assert!((p.s1 - (2.0 * p.pp - 110.0)).abs() < 1e-10);
    }

    #[test]
    fn resistance_levels_ordered() {
        let c = candle(110.0, 90.0, 100.0);
        let p = pivot_points(&c);
        assert!(p.r3 > p.r2 && p.r2 > p.r1 && p.r1 > p.pp);
    }

    #[test]
    fn support_levels_ordered() {
        let c = candle(110.0, 90.0, 100.0);
        let p = pivot_points(&c);
        assert!(p.pp > p.s1 && p.s1 > p.s2 && p.s2 > p.s3);
    }
}
