//! Trading-daemon internals exposed as a library so the `trading-daemon`
//! integration test suite can drive the executor against a real SpacetimeDB.
//!
//! The actual daemon entrypoint lives in `bin/main.rs` (or `src/main.rs` —
//! Cargo auto-discovers the binary target).
pub mod cli;
pub mod config;
pub mod live_engine;
pub mod order_executor;
pub mod protective_shutdown;
pub mod seed;
