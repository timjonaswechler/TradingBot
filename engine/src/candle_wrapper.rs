use mlua::{MetaMethod, UserData, UserDataFields, UserDataMethods};
use shared::{Context, Position};
use std::cell::RefCell;
use crate::vm::EngineData;

// ── LuaCandle ────────────────────────────────────────────────────────────────

/// A single OHLCV candle exposed to Lua strategies.
pub struct LuaCandle(pub shared::Candle);

impl UserData for LuaCandle {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("open",      |_, this| Ok(this.0.open));
        fields.add_field_method_get("high",      |_, this| Ok(this.0.high));
        fields.add_field_method_get("low",       |_, this| Ok(this.0.low));
        fields.add_field_method_get("close",     |_, this| Ok(this.0.close));
        fields.add_field_method_get("volume",    |_, this| Ok(this.0.volume));
        fields.add_field_method_get("timestamp", |_, this| Ok(this.0.timestamp));
        fields.add_field_method_get("symbol",    |_, this| Ok(this.0.symbol.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("body",  |_, this, ()| Ok(this.0.body()));
        methods.add_method("range", |_, this, ()| Ok(this.0.range()));
    }
}

// ── LuaCandles ───────────────────────────────────────────────────────────────

/// Token passed to Lua as the `candles` argument in `on_tick`.
///
/// Actual candle data lives in `EngineData` (stored as Lua app-data).
///
/// Lua-side contract:
/// - `candles[1]`         → current (newest) candle
/// - `candles[n]`         → nth candle back (nil if out of range)
/// - `#candles`           → number of candles visible
/// - `candles:closes()`   → table of closes, index 1 = newest
/// - `candles:opens()`    → same for opens
/// - `candles:highs()`    → same for highs
/// - `candles:lows()`     → same for lows
/// - `candles:volumes()`  → same for volumes
pub struct LuaCandles;

impl UserData for LuaCandles {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // ── candles[n]  ──────────────────────────────────────────────────────
        methods.add_meta_method(MetaMethod::Index, |lua, _, key: mlua::Value| {
            match key {
                // Integer key: candles[1] = newest, candles[n] = nth back
                mlua::Value::Integer(n) if n >= 1 => {
                    let idx = n as usize;
                    let candle_clone = {
                        let data = lua
                            .app_data_ref::<RefCell<EngineData>>()
                            .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
                        let data = data.borrow();
                        let len = data.candles.len();
                        len.checked_sub(idx).map(|i| data.candles[i].clone())
                    };
                    match candle_clone {
                        Some(c) => Ok(mlua::Value::UserData(lua.create_userdata(LuaCandle(c))?)),
                        None    => Ok(mlua::Value::Nil),
                    }
                }

                // String key: helper methods returned as functions
                mlua::Value::String(ref s) => {
                    let method = s.to_str()?.to_owned();
                    match method.as_str() {
                        "closes" | "opens" | "highs" | "lows" | "volumes" => {
                            let field = method.clone();
                            let f = lua.create_function(move |lua, _: mlua::AnyUserData| {
                                let data = lua
                                    .app_data_ref::<RefCell<EngineData>>()
                                    .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
                                let data = data.borrow();
                                let v: Vec<f64> = data.candles.iter().rev().map(|c| match field.as_str() {
                                    "closes"  => c.close,
                                    "opens"   => c.open,
                                    "highs"   => c.high,
                                    "lows"    => c.low,
                                    "volumes" => c.volume,
                                    _         => unreachable!(),
                                }).collect();
                                Ok(v)
                            })?;
                            Ok(mlua::Value::Function(f))
                        }
                        _ => Ok(mlua::Value::Nil),
                    }
                }

                _ => Ok(mlua::Value::Nil),
            }
        });

        // ── #candles  ────────────────────────────────────────────────────────
        methods.add_meta_method(MetaMethod::Len, |lua, _, ()| {
            let data = lua
                .app_data_ref::<RefCell<EngineData>>()
                .ok_or_else(|| mlua::Error::runtime("EngineData not initialised"))?;
            let len = data.borrow().candles.len();
            Ok(len)
        });
    }
}

// ── LuaPosition ──────────────────────────────────────────────────────────────

/// An open trading position exposed to Lua.
pub struct LuaPosition(pub Position);

impl UserData for LuaPosition {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("side",        |_, this| Ok(this.0.side.to_string()));
        fields.add_field_method_get("entry_price", |_, this| Ok(this.0.entry_price));
        fields.add_field_method_get("size",        |_, this| Ok(this.0.size));
        fields.add_field_method_get("entry_time",  |_, this| Ok(this.0.entry_time));
        fields.add_field_method_get("stop_loss",   |_, this| Ok(this.0.stop_loss));
        fields.add_field_method_get("take_profit", |_, this| Ok(this.0.take_profit));
    }
}

// ── LuaContext ───────────────────────────────────────────────────────────────

/// Portfolio context passed to `on_tick` every candle.
pub struct LuaContext(pub Context);

impl UserData for LuaContext {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("balance",      |_, this| Ok(this.0.balance));
        fields.add_field_method_get("equity",       |_, this| Ok(this.0.equity));
        fields.add_field_method_get("trades_count", |_, this| Ok(this.0.trades_count as i64));

        fields.add_field_method_get("position", |lua, this| {
            match &this.0.position {
                None      => Ok(mlua::Value::Nil),
                Some(pos) => {
                    let ud = lua.create_userdata(LuaPosition(pos.clone()))?;
                    Ok(mlua::Value::UserData(ud))
                }
            }
        });
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("has_position", |_, this, ()| Ok(this.0.has_position()));
    }
}
