/// Registers the `indicators` global table in the Lua VM.
///
/// Every function follows the Lua contract from DESIGN.md:
///   `indicators.xxx(candles, period [, offset])  →  value | nil`
///
/// `candles` is the `LuaCandles` token; actual data is read from `EngineData`
/// stored as Lua app-data. `offset = 0` means current bar, `offset = 1` means
/// one bar back (uses a candle slice ending `offset` bars before the last).
use std::cell::RefCell;

use crate::{
    indicator_cache::{
        atr_from_cache, atr_store,
        ema_from_cache, ema_store,
        rsi_from_cache, rsi_store,
    },
    vm::EngineData,
};

use indicators::{
    momentum::{cci::cci, roc::roc, rsi::rsi, stochastic::stochastic, williams_r::williams_r},
    slope::slope,
    support_resistance::{fibonacci::fibonacci_retracements, pivot_points::pivot_points},
    trend::{adx::adx, dema::dema, ema::ema_at, ichimoku::ichimoku, macd::macd, sar::sar, sma::sma, tema::tema},
    volatility::{atr::atr, bollinger::bollinger, keltner::keltner},
    volume::{mfi::mfi, obv::obv, volume_profile::volume_profile, vwap::vwap},
};

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Convert `Option<f64>` to a Lua value (`nil` when `None`).
fn opt_f64(v: Option<f64>) -> mlua::Value {
    match v {
        Some(n) => mlua::Value::Number(n),
        None    => mlua::Value::Nil,
    }
}

/// Borrow the candle slice, optionally trimmed by `offset` bars from the end.
macro_rules! with_candles {
    ($lua:expr, $offset:expr, $body:expr) => {{
        let data_ref = $lua
            .app_data_ref::<RefCell<EngineData>>()
            .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
        let data = data_ref.borrow();
        let n = data.candles.len();
        let end = n.saturating_sub($offset);
        let candles = &data.candles[..end];
        $body(candles)
    }};
}

// Same but also borrows the cache mutably (for incremental updates).
// We need separate access since we can't borrow candles immutably
// and cache mutably from the same RefCell at the same time.
// Solution: clone the closes/candles slice needed, drop the borrow, then mutate cache.

// ── Registration ─────────────────────────────────────────────────────────────

pub fn register_indicators(lua: &mlua::Lua) -> mlua::Result<()> {
    let ind = lua.create_table()?;

    // ── Trend ────────────────────────────────────────────────────────────────

    ind.set("sma", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            sma(&closes, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("ema", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        if offset > 0 {
            // Offset access: recompute from trimmed slice, no cache involvement
            let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
                let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
                ema_at(&closes, period, 0)
            });
            return Ok(opt_f64(result));
        }

        // offset = 0: try incremental cache first
        let (closes, n) = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let data = data_ref.borrow();
            let closes: Vec<f64> = data.candles.iter().map(|c| c.close).collect();
            let n = closes.len();
            (closes, n)
        };

        // Try cache
        let cached = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            ema_from_cache(&mut data.cache, &closes, period)
        };
        if let Some(v) = cached { return Ok(opt_f64(Some(v))); }

        // Full compute
        let result = ema_at(&closes, period, 0);
        if let Some(v) = result {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            ema_store(&mut data.cache, period, v, n);
        }
        Ok(opt_f64(result))
    })?)?;

    ind.set("dema", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            dema(&closes, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("tema", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            tema(&closes, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("macd", lua.create_function(|lua, (_c, fast, slow, signal_p, offset): (mlua::AnyUserData, usize, usize, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            macd(&closes, fast, slow, signal_p)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("line",      r.line)?;
                t.set("signal",    r.signal)?;
                t.set("histogram", r.histogram)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    ind.set("sar", lua.create_function(|lua, (_c, step, max, offset): (mlua::AnyUserData, f64, f64, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            sar(candles, step, max)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("adx", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            adx(candles, period)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("adx",      r.adx)?;
                t.set("plus_di",  r.plus_di)?;
                t.set("minus_di", r.minus_di)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    ind.set("ichimoku", lua.create_function(|lua, (_c, offset): (mlua::AnyUserData, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            ichimoku(candles)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("tenkan",  r.tenkan)?;
                t.set("kijun",   r.kijun)?;
                t.set("span_a",  r.span_a)?;
                t.set("span_b",  r.span_b)?;
                t.set("chikou",  r.chikou)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    // ── Momentum ─────────────────────────────────────────────────────────────

    ind.set("rsi", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        if offset > 0 {
            let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
                let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
                rsi(&closes, period)
            });
            return Ok(opt_f64(result));
        }

        let closes = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let data = data_ref.borrow();
            data.candles.iter().map(|c| c.close).collect::<Vec<f64>>()
        };
        let n = closes.len();

        let cached = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            rsi_from_cache(&mut data.cache, &closes, period)
        };
        if let Some(v) = cached { return Ok(opt_f64(Some(v))); }

        // Full compute — also extract the Wilder state for caching
        if period == 0 || closes.len() <= period { return Ok(mlua::Value::Nil); }
        let mut avg_gain = 0.0f64;
        let mut avg_loss = 0.0f64;
        for i in 1..=period {
            let ch = closes[i] - closes[i - 1];
            if ch > 0.0 { avg_gain += ch; } else { avg_loss += ch.abs(); }
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
        let val = if avg_loss < 1e-12 { 100.0 } else { 100.0 - 100.0 / (1.0 + avg_gain / avg_loss) };

        {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            rsi_store(&mut data.cache, period, avg_gain, avg_loss, *closes.last().unwrap(), n);
        }
        Ok(opt_f64(Some(val)))
    })?)?;

    ind.set("cci", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            cci(candles, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("stochastic", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            stochastic(candles, period)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("k", r.k)?;
                t.set("d", r.d)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    ind.set("williams_r", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            williams_r(candles, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("roc", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            roc(&closes, period)
        });
        Ok(opt_f64(result))
    })?)?;

    // ── Volatility ───────────────────────────────────────────────────────────

    ind.set("bollinger", lua.create_function(|lua, (_c, period, std_dev, offset): (mlua::AnyUserData, usize, f64, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            bollinger(&closes, period, std_dev)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("upper",  r.upper)?;
                t.set("middle", r.middle)?;
                t.set("lower",  r.lower)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    ind.set("atr", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        if offset > 0 {
            let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
                atr(candles, period)
            });
            return Ok(opt_f64(result));
        }

        let (candles_clone, n) = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let data = data_ref.borrow();
            (data.candles.clone(), data.candles.len())
        };

        let cached = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            atr_from_cache(&mut data.cache, &candles_clone, period)
        };
        if let Some(v) = cached { return Ok(opt_f64(Some(v))); }

        let result = atr(&candles_clone, period);
        if let Some(v) = result {
            let prev_close = candles_clone.last().map(|c| c.close).unwrap_or(0.0);
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let mut data = data_ref.borrow_mut();
            atr_store(&mut data.cache, period, v, prev_close, n);
        }
        Ok(opt_f64(result))
    })?)?;

    ind.set("keltner", lua.create_function(|lua, (_c, period, mult, offset): (mlua::AnyUserData, usize, f64, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            keltner(candles, period, mult)
        });
        match result {
            None    => Ok(mlua::Value::Nil),
            Some(r) => {
                let t = lua.create_table()?;
                t.set("upper",  r.upper)?;
                t.set("middle", r.middle)?;
                t.set("lower",  r.lower)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    // ── Volume ───────────────────────────────────────────────────────────────

    ind.set("obv", lua.create_function(|lua, (_c, offset): (mlua::AnyUserData, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            obv(candles)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("vwap", lua.create_function(|lua, (_c, offset): (mlua::AnyUserData, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            vwap(candles)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("mfi", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            mfi(candles, period)
        });
        Ok(opt_f64(result))
    })?)?;

    ind.set("volume_profile", lua.create_function(|lua, (_c, buckets, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            volume_profile(candles, buckets)
        });
        match result {
            None => Ok(mlua::Value::Nil),
            Some(profile) => {
                let t = lua.create_table()?;
                for (i, bucket) in profile.iter().enumerate() {
                    let b = lua.create_table()?;
                    b.set("price",  bucket.price)?;
                    b.set("volume", bucket.volume)?;
                    t.set(i + 1, b)?;
                }
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    // ── Support / Resistance ─────────────────────────────────────────────────

    ind.set("pivot_points", lua.create_function(|lua, (_c, offset): (mlua::AnyUserData, Option<usize>)| {
        // Uses the candle immediately before the current window as the "previous period"
        let offset = offset.unwrap_or(0);
        let prev_candle = {
            let data_ref = lua.app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let data = data_ref.borrow();
            let n = data.candles.len();
            let end = n.saturating_sub(offset);
            if end < 1 { None } else { Some(data.candles[end - 1].clone()) }
        };
        match prev_candle {
            None => Ok(mlua::Value::Nil),
            Some(c) => {
                let r = pivot_points(&c);
                let t = lua.create_table()?;
                t.set("pp", r.pp)?;
                t.set("r1", r.r1)?; t.set("r2", r.r2)?; t.set("r3", r.r3)?;
                t.set("s1", r.s1)?; t.set("s2", r.s2)?; t.set("s3", r.s3)?;
                Ok(mlua::Value::Table(t))
            }
        }
    })?)?;

    ind.set("fibonacci", lua.create_function(|_, (_c, low, high): (mlua::AnyUserData, f64, f64)| {
        let levels = fibonacci_retracements(low, high);
        Ok(levels)
    })?)?;

    // ── Slope ────────────────────────────────────────────────────────────────

    ind.set("slope", lua.create_function(|lua, (_c, period, offset): (mlua::AnyUserData, usize, Option<usize>)| {
        let offset = offset.unwrap_or(0);
        let result = with_candles!(lua, offset, |candles: &[shared::Candle]| {
            let closes: Vec<f64> = candles.iter().map(|c| c.close).collect();
            slope(&closes, period)
        });
        Ok(opt_f64(result))
    })?)?;

    lua.globals().set("indicators", ind)?;
    Ok(())
}
