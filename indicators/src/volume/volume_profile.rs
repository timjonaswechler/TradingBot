use shared::Candle;

/// A single bucket in the Volume Profile histogram.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeProfileBucket {
    /// Mid-price of this bucket.
    pub price: f64,
    /// Total volume traded within this price bucket.
    pub volume: f64,
}

/// Volume Profile — distributes traded volume across `num_buckets` price levels.
///
/// Returns buckets sorted from lowest to highest price.
/// Needs at least 1 candle.
pub fn volume_profile(candles: &[Candle], num_buckets: usize) -> Option<Vec<VolumeProfileBucket>> {
    if candles.is_empty() || num_buckets == 0 {
        return None;
    }

    let high = candles
        .iter()
        .map(|c| c.high)
        .fold(f64::NEG_INFINITY, f64::max);
    let low = candles.iter().map(|c| c.low).fold(f64::INFINITY, f64::min);

    if (high - low).abs() < 1e-12 {
        // Flat market — all volume in one bucket
        return Some(vec![VolumeProfileBucket {
            price: high,
            volume: candles.iter().map(|c| c.volume).sum(),
        }]);
    }

    let bucket_size = (high - low) / num_buckets as f64;
    let mut buckets = vec![0.0f64; num_buckets];

    for c in candles {
        // Distribute candle volume proportionally across the buckets it spans
        let c_low_idx = ((c.low - low) / bucket_size).floor() as usize;
        let c_high_idx = ((c.high - low) / bucket_size).floor() as usize;
        let c_low_idx = c_low_idx.min(num_buckets - 1);
        let c_high_idx = c_high_idx.min(num_buckets - 1);

        let span = (c_high_idx - c_low_idx + 1) as f64;
        let vol_per_bucket = c.volume / span;
        for b in c_low_idx..=c_high_idx {
            buckets[b] += vol_per_bucket;
        }
    }

    let result = buckets
        .iter()
        .enumerate()
        .map(|(i, &vol)| VolumeProfileBucket {
            price: low + (i as f64 + 0.5) * bucket_size,
            volume: vol,
        })
        .collect();

    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candle(h: f64, l: f64, v: f64) -> Candle {
        Candle {
            timestamp: 0,
            symbol: "T".into(),
            open: l,
            high: h,
            low: l,
            close: (h + l) / 2.0,
            volume: v,
            timeframe: "1d".parse().unwrap(),
        }
    }

    #[test]
    fn empty_returns_none() {
        assert_eq!(volume_profile(&[], 10), None);
    }

    #[test]
    fn zero_buckets_returns_none() {
        assert_eq!(volume_profile(&[candle(2.0, 1.0, 100.0)], 0), None);
    }

    #[test]
    fn total_volume_is_preserved() {
        let c = vec![candle(5.0, 1.0, 100.0), candle(8.0, 4.0, 200.0)];
        let profile = volume_profile(&c, 10).unwrap();
        let total: f64 = profile.iter().map(|b| b.volume).sum();
        assert!((total - 300.0).abs() < 1e-6);
    }

    #[test]
    fn returns_correct_bucket_count() {
        let c = vec![candle(10.0, 1.0, 100.0)];
        let profile = volume_profile(&c, 5).unwrap();
        assert_eq!(profile.len(), 5);
    }
}
