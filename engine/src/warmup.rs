use crate::{error::EngineError, vm::Engine};
use shared::Candle;

/// Feed `warmup_candles` into the engine without calling `on_tick`.
///
/// Used by the daemon on startup: fetch the last N historical candles from
/// SpacetimeDB and push them here so the indicator cache is warm before
/// the first live candle arrives. No signals are produced, no trades are
/// recorded.
///
/// `warmup_candles` must be in chronological order (oldest first).
pub fn warmup(engine: &mut Engine, warmup_candles: Vec<Candle>) -> Result<(), EngineError> {
    for candle in warmup_candles {
        engine.push_candle(candle);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_candle(close: f64, ts: i64) -> Candle {
        Candle {
            timestamp: ts,
            symbol: "TEST".into(),
            open: close - 0.5,
            high: close + 1.0,
            low: close - 1.0,
            close,
            volume: 1000.0,
            timeframe: "1d".parse().unwrap(),
        }
    }

    const STRATEGY: &str = r#"
fn on_tick(candles, context) {
    #{ signal: "HOLD" }
}
"#;

    const ICHIMOKU_READY_STRATEGY: &str = r#"
fn on_tick(candles, context) {
    let ichi = indicators::ichimoku(candles);
    if ichi == () {
        return #{ signal: "HOLD" };
    }
    #{ signal: "BUY" }
}
"#;

    #[test]
    fn warmup_pre_loads_candles() {
        let mut engine = Engine::new(STRATEGY).unwrap();

        let historical: Vec<Candle> = (1..=20).map(|i| make_candle(i as f64, i)).collect();
        let count = historical.len();

        warmup(&mut engine, historical).unwrap();

        assert_eq!(engine.candle_count(), count);
    }

    #[test]
    fn warmup_followed_by_tick_works() {
        let mut engine = Engine::new(STRATEGY).unwrap();

        let historical: Vec<Candle> = (1..=20).map(|i| make_candle(i as f64, i)).collect();
        warmup(&mut engine, historical).unwrap();

        let ctx = shared::Context::new(10_000.0);
        let decision = engine.tick(make_candle(21.0, 21), ctx).unwrap();
        assert_eq!(engine.candle_count(), 21);
        assert_eq!(decision.signal, shared::Signal::Hold);
    }

    #[test]
    fn ichimoku_is_ready_on_first_live_tick_after_51_warmup_bars() {
        let mut engine = Engine::new(ICHIMOKU_READY_STRATEGY).unwrap();

        let historical: Vec<Candle> = (1..=51).map(|i| make_candle(i as f64, i)).collect();
        warmup(&mut engine, historical).unwrap();

        let ctx = shared::Context::new(10_000.0);
        let decision = engine.tick(make_candle(52.0, 52), ctx).unwrap();

        assert_eq!(decision.signal, shared::Signal::Buy);
    }

    #[test]
    fn ichimoku_is_not_ready_on_first_live_tick_after_only_50_warmup_bars() {
        let mut engine = Engine::new(ICHIMOKU_READY_STRATEGY).unwrap();

        let historical: Vec<Candle> = (1..=50).map(|i| make_candle(i as f64, i)).collect();
        warmup(&mut engine, historical).unwrap();

        let ctx = shared::Context::new(10_000.0);
        let decision = engine.tick(make_candle(51.0, 51), ctx).unwrap();

        assert_eq!(decision.signal, shared::Signal::Hold);
    }
}
