/// Builds the `indicators` Rhai module.
///
/// Strategies access indicators with:  `indicators::rsi(candles, 14)`
///
/// Most history-dependent functions are overloaded:
///   `indicators::xxx(candles, ...params)`          — offset = 0 (current bar)
///   `indicators::xxx(candles, ...params, offset)`  — value `offset` bars back
///
/// Exception: `indicators::fibonacci(candles, low, high)` is a low-level,
/// stateless helper and has no meaningful offset variant. Long-term, the main
/// Fibonacci workflow should move to an anchored pivot-based evaluator, while
/// this function remains a primitive utility.
///
/// Returns `()` (Rhai unit) when insufficient data, mirroring `Option::None`.
use rhai::{Dynamic, Engine, Module, INT};
use std::sync::Arc;

use crate::candle_wrapper::CandleList;
use crate::indicator_cache::{
    atr_from_cache, atr_store, ema_from_cache, ema_store, rsi_from_cache, rsi_store,
};

use indicators::{
    momentum::{
        cci::cci,
        roc::roc,
        rsi::rsi,
        stochastic::{stochastic_fast, stochastic_full, stochastic_slow},
        williams_r::williams_r,
    },
    slope::slope,
    support_resistance::{fibonacci::fibonacci_retracements, pivot_points::pivot_points},
    trend::{
        adx::adx, dema::dema, ema::ema_at, ichimoku::ichimoku, macd::macd, sar::sar, sma::sma,
        tema::tema,
    },
    volatility::{atr::atr, bollinger::bollinger, keltner::keltner},
    volume::{mfi::mfi, obv::obv, volume_profile::volume_profile, vwap::vwap},
};

// ── Helpers ──────────────────────────────────────────────────────────────────

type RhaiResult = Result<Dynamic, Box<rhai::EvalAltResult>>;

fn opt(v: Option<f64>) -> Dynamic {
    match v {
        Some(n) => Dynamic::from(n),
        None => Dynamic::UNIT,
    }
}

/// Read candles from CandleList, trimmed by `offset` from the end.
/// Returns a clone of the relevant slice.
fn candles_slice(cl: &CandleList, offset: usize) -> Vec<domain::Candle> {
    let candles = cl.candles.read().unwrap();
    let n = candles.len();
    let end = n.saturating_sub(offset);
    candles[..end].to_vec()
}

fn closes_slice(cl: &CandleList, offset: usize) -> Vec<f64> {
    candles_slice(cl, offset).iter().map(|c| c.close).collect()
}

// ── Module builder ────────────────────────────────────────────────────────────

pub fn build_indicators_module() -> Module {
    let mut m = Module::new();

    // ── Trend ────────────────────────────────────────────────────────────────

    m.set_native_fn("sma", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(sma(&closes_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "sma",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(sma(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn("ema", |cl: &mut CandleList, period: INT| -> RhaiResult {
        let closes = closes_slice(cl, 0);
        let n = closes.len();
        // Try incremental cache
        let cached = {
            let mut cache = cl.cache.write().unwrap();
            ema_from_cache(&mut cache, &closes, period as usize)
        };
        if let Some(v) = cached {
            return Ok(Dynamic::from(v));
        }
        // Full compute + store
        let result = ema_at(&closes, period as usize, 0);
        if let Some(v) = result {
            let mut cache = cl.cache.write().unwrap();
            ema_store(&mut cache, period as usize, v, n);
        }
        Ok(opt(result))
    });
    m.set_native_fn(
        "ema",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(ema_at(
                &closes_slice(cl, offset as usize),
                period as usize,
                0,
            )))
        },
    );

    m.set_native_fn("dema", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(dema(&closes_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "dema",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(dema(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn("tema", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(tema(&closes_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "tema",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(tema(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn(
        "macd",
        |cl: &mut CandleList, fast: INT, slow: INT, signal: INT| -> RhaiResult {
            match macd(
                &closes_slice(cl, 0),
                fast as usize,
                slow as usize,
                signal as usize,
            ) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("line".into(), Dynamic::from(r.line));
                    map.insert("signal".into(), Dynamic::from(r.signal));
                    map.insert("histogram".into(), Dynamic::from(r.histogram));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "macd",
        |cl: &mut CandleList, fast: INT, slow: INT, signal: INT, offset: INT| -> RhaiResult {
            match macd(
                &closes_slice(cl, offset as usize),
                fast as usize,
                slow as usize,
                signal as usize,
            ) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("line".into(), Dynamic::from(r.line));
                    map.insert("signal".into(), Dynamic::from(r.signal));
                    map.insert("histogram".into(), Dynamic::from(r.histogram));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn(
        "sar",
        |cl: &mut CandleList, step: f64, max: f64| -> RhaiResult {
            match sar(&candles_slice(cl, 0), step, max) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("value".into(), Dynamic::from(r.value));
                    map.insert(
                        "side".into(),
                        Dynamic::from(match r.side {
                            indicators::trend::sar::SarSide::Long => "long",
                            indicators::trend::sar::SarSide::Short => "short",
                        }),
                    );
                    map.insert("reversed".into(), Dynamic::from(r.reversed));
                    map.insert("ep".into(), Dynamic::from(r.ep));
                    map.insert("af".into(), Dynamic::from(r.af));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "sar",
        |cl: &mut CandleList, step: f64, max: f64, offset: INT| -> RhaiResult {
            match sar(&candles_slice(cl, offset as usize), step, max) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("value".into(), Dynamic::from(r.value));
                    map.insert(
                        "side".into(),
                        Dynamic::from(match r.side {
                            indicators::trend::sar::SarSide::Long => "long",
                            indicators::trend::sar::SarSide::Short => "short",
                        }),
                    );
                    map.insert("reversed".into(), Dynamic::from(r.reversed));
                    map.insert("ep".into(), Dynamic::from(r.ep));
                    map.insert("af".into(), Dynamic::from(r.af));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn("adx", |cl: &mut CandleList, period: INT| -> RhaiResult {
        match adx(&candles_slice(cl, 0), period as usize) {
            None => Ok(Dynamic::UNIT),
            Some(r) => {
                let mut map = rhai::Map::new();
                map.insert("adx".into(), Dynamic::from(r.adx));
                map.insert("plus_di".into(), Dynamic::from(r.plus_di));
                map.insert("minus_di".into(), Dynamic::from(r.minus_di));
                Ok(Dynamic::from_map(map))
            }
        }
    });
    m.set_native_fn(
        "adx",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            match adx(&candles_slice(cl, offset as usize), period as usize) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("adx".into(), Dynamic::from(r.adx));
                    map.insert("plus_di".into(), Dynamic::from(r.plus_di));
                    map.insert("minus_di".into(), Dynamic::from(r.minus_di));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn("ichimoku", |cl: &mut CandleList| -> RhaiResult {
        match ichimoku(&candles_slice(cl, 0)) {
            None => Ok(Dynamic::UNIT),
            Some(r) => {
                let mut map = rhai::Map::new();
                map.insert("tenkan".into(), Dynamic::from(r.tenkan));
                map.insert("kijun".into(), Dynamic::from(r.kijun));
                map.insert("span_a".into(), Dynamic::from(r.span_a));
                map.insert("span_b".into(), Dynamic::from(r.span_b));
                map.insert("chikou".into(), Dynamic::from(r.chikou));
                Ok(Dynamic::from_map(map))
            }
        }
    });
    m.set_native_fn(
        "ichimoku",
        |cl: &mut CandleList, offset: INT| -> RhaiResult {
            match ichimoku(&candles_slice(cl, offset as usize)) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("tenkan".into(), Dynamic::from(r.tenkan));
                    map.insert("kijun".into(), Dynamic::from(r.kijun));
                    map.insert("span_a".into(), Dynamic::from(r.span_a));
                    map.insert("span_b".into(), Dynamic::from(r.span_b));
                    map.insert("chikou".into(), Dynamic::from(r.chikou));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    // ── Momentum ─────────────────────────────────────────────────────────────

    m.set_native_fn("rsi", |cl: &mut CandleList, period: INT| -> RhaiResult {
        let closes = closes_slice(cl, 0);
        let n = closes.len();
        let period = period as usize;

        let cached = {
            let mut cache = cl.cache.write().unwrap();
            rsi_from_cache(&mut cache, &closes, period)
        };
        if let Some(v) = cached {
            return Ok(Dynamic::from(v));
        }

        // Full compute — extract Wilder state for caching
        if period == 0 || closes.len() <= period {
            return Ok(Dynamic::UNIT);
        }
        let mut avg_gain = 0.0f64;
        let mut avg_loss = 0.0f64;
        for i in 1..=period {
            let ch = closes[i] - closes[i - 1];
            if ch > 0.0 {
                avg_gain += ch;
            } else {
                avg_loss += ch.abs();
            }
        }
        avg_gain /= period as f64;
        avg_loss /= period as f64;
        for i in (period + 1)..closes.len() {
            let ch = closes[i] - closes[i - 1];
            let g = if ch > 0.0 { ch } else { 0.0 };
            let l = if ch < 0.0 { ch.abs() } else { 0.0 };
            avg_gain = (avg_gain * (period as f64 - 1.0) + g) / period as f64;
            avg_loss = (avg_loss * (period as f64 - 1.0) + l) / period as f64;
        }
        let val = if avg_loss < 1e-12 {
            100.0
        } else {
            100.0 - 100.0 / (1.0 + avg_gain / avg_loss)
        };

        {
            let mut cache = cl.cache.write().unwrap();
            rsi_store(
                &mut cache,
                period,
                avg_gain,
                avg_loss,
                *closes.last().unwrap(),
                n,
            );
        }
        Ok(Dynamic::from(val))
    });
    m.set_native_fn(
        "rsi",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(rsi(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn("cci", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(cci(&candles_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "cci",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(cci(
                &candles_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    // ── Stochastic (Fast, Slow, Full) ─────────────────────────────────────

    m.set_native_fn(
        "stochastic_fast",
        |cl: &mut CandleList, period: INT| -> RhaiResult {
            match stochastic_fast(&candles_slice(cl, 0), period as usize) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "stochastic_fast",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            match stochastic_fast(&candles_slice(cl, offset as usize), period as usize) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn(
        "stochastic_slow",
        |cl: &mut CandleList, period: INT| -> RhaiResult {
            match stochastic_slow(&candles_slice(cl, 0), period as usize) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "stochastic_slow",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            match stochastic_slow(&candles_slice(cl, offset as usize), period as usize) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn(
        "stochastic_full",
        |cl: &mut CandleList, period: INT| -> RhaiResult {
            match stochastic_full(&candles_slice(cl, 0), period as usize, None, None) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "stochastic_full",
        |cl: &mut CandleList, period: INT, k_smooth: INT| -> RhaiResult {
            match stochastic_full(
                &candles_slice(cl, 0),
                period as usize,
                Some(k_smooth as usize),
                None,
            ) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "stochastic_full",
        |cl: &mut CandleList, period: INT, k_smooth: INT, d_period: INT| -> RhaiResult {
            match stochastic_full(
                &candles_slice(cl, 0),
                period as usize,
                Some(k_smooth as usize),
                Some(d_period as usize),
            ) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "stochastic_full",
        |cl: &mut CandleList,
         period: INT,
         k_smooth: INT,
         d_period: INT,
         offset: INT|
         -> RhaiResult {
            match stochastic_full(
                &candles_slice(cl, offset as usize),
                period as usize,
                Some(k_smooth as usize),
                Some(d_period as usize),
            ) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("k".into(), Dynamic::from(r.k));
                    map.insert("d".into(), Dynamic::from(r.d));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn(
        "williams_r",
        |cl: &mut CandleList, period: INT| -> RhaiResult {
            Ok(opt(williams_r(&candles_slice(cl, 0), period as usize)))
        },
    );
    m.set_native_fn(
        "williams_r",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(williams_r(
                &candles_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn("roc", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(roc(&closes_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "roc",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(roc(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    // ── Volatility ───────────────────────────────────────────────────────────

    m.set_native_fn(
        "bollinger",
        |cl: &mut CandleList, period: INT, std_dev: f64| -> RhaiResult {
            match bollinger(&closes_slice(cl, 0), period as usize, std_dev) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("upper".into(), Dynamic::from(r.upper));
                    map.insert("middle".into(), Dynamic::from(r.middle));
                    map.insert("lower".into(), Dynamic::from(r.lower));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "bollinger",
        |cl: &mut CandleList, period: INT, std_dev: f64, offset: INT| -> RhaiResult {
            match bollinger(&closes_slice(cl, offset as usize), period as usize, std_dev) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("upper".into(), Dynamic::from(r.upper));
                    map.insert("middle".into(), Dynamic::from(r.middle));
                    map.insert("lower".into(), Dynamic::from(r.lower));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    m.set_native_fn("atr", |cl: &mut CandleList, period: INT| -> RhaiResult {
        let period = period as usize;
        let candles = candles_slice(cl, 0);
        let n = candles.len();

        let cached = {
            let mut cache = cl.cache.write().unwrap();
            atr_from_cache(&mut cache, &candles, period)
        };
        if let Some(v) = cached {
            return Ok(Dynamic::from(v));
        }

        let result = atr(&candles, period);
        if let Some(v) = result {
            let prev_close = candles.last().map(|c| c.close).unwrap_or(0.0);
            let mut cache = cl.cache.write().unwrap();
            atr_store(&mut cache, period, v, prev_close, n);
        }
        Ok(opt(result))
    });
    m.set_native_fn(
        "atr",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(atr(
                &candles_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn(
        "keltner",
        |cl: &mut CandleList, period: INT, mult: f64| -> RhaiResult {
            match keltner(&candles_slice(cl, 0), period as usize, mult) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("upper".into(), Dynamic::from(r.upper));
                    map.insert("middle".into(), Dynamic::from(r.middle));
                    map.insert("lower".into(), Dynamic::from(r.lower));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );
    m.set_native_fn(
        "keltner",
        |cl: &mut CandleList, period: INT, mult: f64, offset: INT| -> RhaiResult {
            match keltner(&candles_slice(cl, offset as usize), period as usize, mult) {
                None => Ok(Dynamic::UNIT),
                Some(r) => {
                    let mut map = rhai::Map::new();
                    map.insert("upper".into(), Dynamic::from(r.upper));
                    map.insert("middle".into(), Dynamic::from(r.middle));
                    map.insert("lower".into(), Dynamic::from(r.lower));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    // ── Volume ───────────────────────────────────────────────────────────────

    m.set_native_fn("obv", |cl: &mut CandleList| -> RhaiResult {
        Ok(opt(obv(&candles_slice(cl, 0))))
    });
    m.set_native_fn("obv", |cl: &mut CandleList, offset: INT| -> RhaiResult {
        Ok(opt(obv(&candles_slice(cl, offset as usize))))
    });
    m.set_native_fn("vwap", |cl: &mut CandleList| -> RhaiResult {
        Ok(opt(vwap(&candles_slice(cl, 0))))
    });
    m.set_native_fn("vwap", |cl: &mut CandleList, offset: INT| -> RhaiResult {
        Ok(opt(vwap(&candles_slice(cl, offset as usize))))
    });
    m.set_native_fn("mfi", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(mfi(&candles_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "mfi",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(mfi(
                &candles_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m.set_native_fn(
        "volume_profile",
        |cl: &mut CandleList, buckets: INT| -> RhaiResult {
            match volume_profile(&candles_slice(cl, 0), buckets as usize) {
                None => Ok(Dynamic::UNIT),
                Some(profile) => {
                    let arr: rhai::Array = profile
                        .iter()
                        .map(|b| {
                            let mut map = rhai::Map::new();
                            map.insert("price".into(), Dynamic::from(b.price));
                            map.insert("volume".into(), Dynamic::from(b.volume));
                            Dynamic::from_map(map)
                        })
                        .collect();
                    Ok(Dynamic::from_array(arr))
                }
            }
        },
    );
    m.set_native_fn(
        "volume_profile",
        |cl: &mut CandleList, buckets: INT, offset: INT| -> RhaiResult {
            match volume_profile(&candles_slice(cl, offset as usize), buckets as usize) {
                None => Ok(Dynamic::UNIT),
                Some(profile) => {
                    let arr: rhai::Array = profile
                        .iter()
                        .map(|b| {
                            let mut map = rhai::Map::new();
                            map.insert("price".into(), Dynamic::from(b.price));
                            map.insert("volume".into(), Dynamic::from(b.volume));
                            Dynamic::from_map(map)
                        })
                        .collect();
                    Ok(Dynamic::from_array(arr))
                }
            }
        },
    );

    // ── Support / Resistance ─────────────────────────────────────────────────

    m.set_native_fn("pivot_points", |cl: &mut CandleList| -> RhaiResult {
        let candles = candles_slice(cl, 0);
        match candles.last() {
            None => Ok(Dynamic::UNIT),
            Some(prev) => {
                let r = pivot_points(prev);
                let mut map = rhai::Map::new();
                map.insert("pp".into(), Dynamic::from(r.pp));
                map.insert("r1".into(), Dynamic::from(r.r1));
                map.insert("r2".into(), Dynamic::from(r.r2));
                map.insert("r3".into(), Dynamic::from(r.r3));
                map.insert("s1".into(), Dynamic::from(r.s1));
                map.insert("s2".into(), Dynamic::from(r.s2));
                map.insert("s3".into(), Dynamic::from(r.s3));
                Ok(Dynamic::from_map(map))
            }
        }
    });
    m.set_native_fn(
        "pivot_points",
        |cl: &mut CandleList, offset: INT| -> RhaiResult {
            let candles = candles_slice(cl, offset as usize);
            match candles.last() {
                None => Ok(Dynamic::UNIT),
                Some(prev) => {
                    let r = pivot_points(prev);
                    let mut map = rhai::Map::new();
                    map.insert("pp".into(), Dynamic::from(r.pp));
                    map.insert("r1".into(), Dynamic::from(r.r1));
                    map.insert("r2".into(), Dynamic::from(r.r2));
                    map.insert("r3".into(), Dynamic::from(r.r3));
                    map.insert("s1".into(), Dynamic::from(r.s1));
                    map.insert("s2".into(), Dynamic::from(r.s2));
                    map.insert("s3".into(), Dynamic::from(r.s3));
                    Ok(Dynamic::from_map(map))
                }
            }
        },
    );

    // Low-level helper only: explicit low/high -> levels.
    // Intentionally not offset-aware. The long-term strategy-facing Fibonacci
    // direction is a pivot-based anchored evaluator rather than a rolling call.
    m.set_native_fn(
        "fibonacci",
        |_cl: &mut CandleList, low: f64, high: f64| -> RhaiResult {
            let levels: rhai::Array = fibonacci_retracements(low, high)
                .into_iter()
                .map(Dynamic::from)
                .collect();
            Ok(Dynamic::from_array(levels))
        },
    );

    // ── Slope ────────────────────────────────────────────────────────────────

    m.set_native_fn("slope", |cl: &mut CandleList, period: INT| -> RhaiResult {
        Ok(opt(slope(&closes_slice(cl, 0), period as usize)))
    });
    m.set_native_fn(
        "slope",
        |cl: &mut CandleList, period: INT, offset: INT| -> RhaiResult {
            Ok(opt(slope(
                &closes_slice(cl, offset as usize),
                period as usize,
            )))
        },
    );

    m
}

/// Register the indicators module and all candle/context types on the engine.
pub fn register_all(engine: &mut Engine) {
    let module = Arc::new(build_indicators_module());
    engine.register_static_module("indicators", module);
}
