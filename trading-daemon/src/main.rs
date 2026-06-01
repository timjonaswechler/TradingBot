/// TradingBot2 — Trading Daemon
///
/// A headless, stateful Tokio service that:
/// - Reacts to new candles in SpacetimeDB via on_insert callbacks
/// - Ticks Rhai strategies with O(1) indicator updates
/// - Executes paper trades and persists them to SpacetimeDB
///
/// Usage:
///   trading-daemon run  --config trading-bot.toml
///   trading-daemon seed --config trading-bot.toml [--from 2020-01-01]
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use tokio_util::sync::CancellationToken;
use tracing::info;

use trading_daemon::{cli, config, live_engine, seed};

use cli::{Cli, Command};
use config::Config;
use db_layer::SpacetimeClient;

#[tokio::main]
async fn main() -> Result<()> {
    // ── Logging ────────────────────────────────────────────────────────────────
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "trading_daemon=info,db_layer=info".into()),
        )
        .init();

    // ── CLI ────────────────────────────────────────────────────────────────────
    let cli = Cli::parse();

    match cli.command {
        Command::Seed {
            config: config_path,
            from,
        } => {
            let config = Config::load(&config_path)?;
            seed::run(&config, from).await?;
        }

        Command::Run {
            config: config_path,
        } => {
            run_daemon(&config_path).await?;
        }
    }

    Ok(())
}

async fn run_daemon(config_path: &str) -> Result<()> {
    let config = Config::load(config_path)?;

    if config.assets.is_empty() {
        anyhow::bail!("No assets configured in '{config_path}'");
    }

    // ── Connect to SpacetimeDB ─────────────────────────────────────────────────
    info!(
        url = config.database.url,
        module = config.database.module,
        "Connecting to SpacetimeDB"
    );
    let client = Arc::new(SpacetimeClient::connect(
        &config.database.url,
        &config.database.module,
    )?);
    info!("Connected — cache ready");

    // ── Spawn one live runtime task per Runtime Asset ─────────────────────────
    let cancel = CancellationToken::new();
    let mut handles = Vec::new();

    for asset in config.assets {
        if asset.strategy.is_empty() {
            tracing::warn!(symbol = asset.symbol, "No strategy configured — skipping");
            continue;
        }
        let client_clone = client.clone();
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            if let Err(e) = live_engine::run(client_clone, asset, cancel_clone).await {
                tracing::error!(error = %e, "Live runtime task failed");
            }
        });

        handles.push(handle);
    }

    // ── Wait for shutdown signal ───────────────────────────────────────────────
    info!("Daemon running — press Ctrl+C to stop");

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            info!("SIGINT received — shutting down gracefully");
        }
        _ = wait_sigterm() => {
            info!("SIGTERM received — shutting down gracefully");
        }
    }

    // Signal all tasks to stop and wait.
    cancel.cancel();
    for handle in handles {
        let _ = handle.await;
    }

    info!("Daemon stopped.");
    Ok(())
}

/// Wait for SIGTERM on Unix systems.
async fn wait_sigterm() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        } else {
            // If we can't register SIGTERM, just pend forever.
            std::future::pending::<()>().await;
        }
    }
    #[cfg(not(unix))]
    std::future::pending::<()>().await;
}
