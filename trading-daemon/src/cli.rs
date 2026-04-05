/// CLI argument definitions for the trading daemon.
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name    = "trading-daemon",
    about   = "Stateful trading daemon — paper trading & live strategy execution",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Start the live trading daemon.
    ///
    /// Connects to SpacetimeDB, warms up engines for all configured
    /// assets/intervals, then reacts to new candles via on_insert callbacks.
    Run {
        /// Path to the TOML configuration file.
        #[arg(short, long, default_value = "trading-bot.toml")]
        config: String,
    },

    /// Seed SpacetimeDB with historical candles from Yahoo Finance.
    ///
    /// Loads all asset/interval combinations defined in the config file.
    /// Already-existing candles are skipped (idempotent).
    Seed {
        /// Path to the TOML configuration file.
        #[arg(short, long, default_value = "trading-bot.toml")]
        config: String,

        /// Override the start date (ISO 8601: "2024-01-01").
        #[arg(long)]
        from: Option<String>,
    },
}
