/// SpacetimeDB SDK client wrapper.
///
/// Connects to the `trading-bot` module via WebSocket using the official
/// `spacetimedb-sdk`.  On connect, immediately subscribes to all three tables
/// (`candles`, `live_positions`, `live_trades`) and blocks until the local
/// cache is populated (`on_applied`).
///
/// After `SpacetimeClient::connect()` returns, the cache is guaranteed to be
/// warm — all existing rows are available via `conn.db.candles().iter()` etc.
use std::sync::{mpsc, Arc};

use spacetimedb_sdk::{credentials, DbContext};
use tracing::{info, warn};

use crate::module_bindings::{
    candlesQueryTableAccess, live_positionsQueryTableAccess, live_tradesQueryTableAccess,
    DbConnection, ErrorContext,
};

/// A connected SpacetimeDB client with a warm local cache.
///
/// `conn` is wrapped in `Arc` so it can be shared across Tokio tasks
/// without cloning (SpacetimeDB's `DbConnection` is not `Clone`,
/// but the underlying `DbContextImpl` is reference-counted internally).
pub struct SpacetimeClient {
    pub conn: Arc<DbConnection>,
}

impl SpacetimeClient {
    /// Connect to SpacetimeDB, subscribe to all tables, and wait until the
    /// local cache is populated before returning.
    ///
    /// This call blocks briefly (typically a few milliseconds on localhost)
    /// until `on_applied` fires and the cache is warm.
    pub fn connect(uri: &str, db_name: &str) -> anyhow::Result<Self> {
        let (tx, rx) = mpsc::channel::<()>();
        let db_name = db_name.to_string(); // owned so it can be moved into the closure

        let conn = DbConnection::builder()
            .with_uri(uri)
            .with_database_name(&db_name)
            // Load a saved token if available so we reconnect as the same identity.
            .with_token(
                credentials::File::new(&db_name)
                    .load()
                    .unwrap_or_default(),
            )
            .on_connect(move |ctx, identity, token| {
                info!("Connected to SpacetimeDB as {:?}", identity);

                // Save token for future reconnects.
                if let Err(e) = credentials::File::new(&db_name).save(token) {
                    warn!("Failed to save credentials: {e}");
                }

                // Immediately subscribe to all tables.
                // on_applied fires when the initial rows are loaded into the cache.
                let tx = tx.clone();
                ctx.subscription_builder()
                    .on_applied(move |_ctx| {
                        // Cache is now populated — signal the caller.
                        let _ = tx.send(());
                    })
                    .on_error(|_ctx, err| {
                        warn!("Subscription error: {err}");
                    })
                    .add_query(|q| q.from.candles())
                    .add_query(|q| q.from.live_positions())
                    .add_query(|q| q.from.live_trades())
                    .subscribe();
            })
            .on_connect_error(|_ctx: &ErrorContext, err| {
                warn!("SpacetimeDB connection error: {err}");
            })
            .on_disconnect(|_ctx: &ErrorContext, err| {
                if let Some(e) = err {
                    warn!("Disconnected from SpacetimeDB: {e}");
                } else {
                    info!("Disconnected from SpacetimeDB.");
                }
            })
            .build()?;

        // Spawn the background thread that processes WebSocket messages.
        conn.run_threaded();

        // Block until on_applied fires (cache is warm).
        rx.recv()
            .map_err(|_| anyhow::anyhow!("Subscription failed before on_applied"))?;

        info!("SpacetimeDB cache ready.");
        Ok(Self { conn: Arc::new(conn) })
    }

    /// Build a `SpacetimeClient` from environment variables.
    ///
    /// - `SPACETIMEDB_URL`    (default: `http://localhost:3000`)
    /// - `SPACETIMEDB_MODULE` (default: `trading-bot`)
    pub fn from_env() -> anyhow::Result<Self> {
        let uri = std::env::var("SPACETIMEDB_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3000".into());
        let module = std::env::var("SPACETIMEDB_MODULE")
            .unwrap_or_else(|_| "trading-bot".into());
        Self::connect(&uri, &module)
    }
}
