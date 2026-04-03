pub mod momentum;
pub mod slope;
pub mod support_resistance;
pub mod trend;
pub mod volatility;
pub mod volume;

// Re-export result types so callers only need `indicators::*`
pub use trend::{
    adx::AdxResult,
    ichimoku::IchimokuResult,
    macd::MacdResult,
};
pub use volatility::{
    bollinger::BbResult,
    keltner::KeltnerResult,
};
pub use momentum::stochastic::StochasticResult;
pub use support_resistance::{
    fibonacci::fibonacci_retracements,
    pivot_points::PivotResult,
};
pub use volume::volume_profile::VolumeProfileBucket;
