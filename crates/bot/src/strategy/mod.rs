use crate::market_data::Candle;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
    Short,
}

pub trait Strategy: Send + Sync {
    fn name(&self) -> &str;
    /// Minimum number of CANDLES required in each slice for signal computation.
    fn required_history(&self) -> usize;
    /// Compute a trading signal.
    /// primary: large-interval candles (e.g. 1d), newest-first
    /// secondary: small-interval candles (e.g. 1h), newest-first
    fn signal(&self, primary: &[Candle], secondary: &[Candle]) -> Signal;
}

pub mod dual_macd;
