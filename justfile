# ── TradingBot2 — justfile ─────────────────────────────────────────────────
# Run `just` to see all available recipes.
# Requires: just  (cargo install just)
#           cargo
#           spacetimedb CLI  (for db-* recipes)

# Default: list all recipes
default:
    @just --list

# ── Build ──────────────────────────────────────────────────────────────────

# Build the entire workspace (debug)
build:
    cargo build --workspace

# Build the entire workspace (release)
build-release:
    cargo build --workspace --release

# Build only the trading daemon
build-daemon:
    cargo build -p trading-daemon

# Build only the trading UI
build-ui:
    cargo build -p trading-ui

# ── Test ───────────────────────────────────────────────────────────────────

# Run all tests across the workspace
test:
    cargo test --workspace

# Run tests for a specific crate  (e.g. `just test-crate domain`)
test-crate crate:
    cargo test -p {{crate}}

# Run tests and show output even for passing tests
test-verbose:
    cargo test --workspace -- --nocapture

# ── Lint & Format ──────────────────────────────────────────────────────────

# Check formatting (no writes)
fmt-check:
    cargo fmt --all -- --check

# Apply formatting
fmt:
    cargo fmt --all

# Run clippy on the whole workspace
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# fmt + lint in one shot (run before committing)
check: fmt lint test

# ── Run ────────────────────────────────────────────────────────────────────

# Start the trading daemon  (pass args after `--`, e.g. `just daemon -- --help`)
daemon *args:
    RUST_LOG=${RUST_LOG:-trading_daemon=info,db_layer=info} cargo run -p trading-daemon -- {{args}}

# Seed SpacetimeDB with historical candles from Yahoo Finance
# Uses trading-bot.toml unless you pass your own --config.
seed *args='--config trading-bot.toml':
    RUST_LOG=${RUST_LOG:-trading_daemon=info,db_layer=info} cargo run -p trading-daemon -- seed {{args}}

# Start the live trading daemon
# Uses trading-bot.toml unless you pass your own --config.
run *args='--config trading-bot.toml':
    RUST_LOG=${RUST_LOG:-trading_daemon=info,db_layer=info} cargo run -p trading-daemon -- run {{args}}

# Run a backtest against candles in SpacetimeDB
# Example: just backtest --strategy strategies/sma_cross.rhai --symbol AAPL
backtest *args:
    RUST_LOG=${RUST_LOG:-backtester=info} cargo run -p backtester -- {{args}}

# Start the trading UI
ui *args:
    cargo run -p trading-ui -- {{args}}

# ── SpacetimeDB ────────────────────────────────────────────────────────────

# Start a local SpacetimeDB server (foreground).
# The server listens on http://127.0.0.1:3000
# If you get "spacetime.pid already exists" the server is already running — use `just db-stop` first.
db-start:
    spacetime start

# Stop the running SpacetimeDB server.
db-stop:
    -lsof -ti :3000 | xargs kill 2>/dev/null || true
    @echo "SpacetimeDB stopped (or was not running)."

# Restart the SpacetimeDB server (stop + start).
db-restart: db-stop db-start

# Create a timestamped backup of the SpacetimeDB data directory.
# Stops the server first to guarantee a consistent snapshot.
# Run `just db-start` manually afterwards in your server terminal.
db-backup: db-stop
    #!/usr/bin/env bash
    set -e
    DEST=~/spacetime-backup-$(date +%Y%m%d-%H%M%S)
    cp -r ~/.local/share/spacetime/data/ "$DEST"
    echo "Backup saved to $DEST"
    echo "Run 'just db-start' to bring the server back up."

# Build the SpacetimeDB WASM module (requires wasm32-unknown-unknown target)
db-build:
    cargo build --manifest-path spacetimedb-module/Cargo.toml --target wasm32-unknown-unknown --release

# Publish the WASM module to the local SpacetimeDB server.
# First run: creates the database. Subsequent runs: update the schema (auto-migration).
db-deploy: db-build
    spacetime publish \
        -s local \
        --bin-path spacetimedb-module/target/wasm32-unknown-unknown/release/spacetimedb_module.wasm \
        trading-bot

# Generate Rust client bindings from the module source into db-layer/src/module_bindings/
# Run this after any schema change in spacetimedb-module/src/lib.rs
db-generate:
    mkdir -p db-layer/src/module_bindings
    spacetime generate \
        -l rust \
        --out-dir db-layer/src/module_bindings \
        --module-path spacetimedb-module

# Publish with a full data wipe — DELETES ALL ROWS in every table.
# Only needed to manually reset the DB (e.g. stale data from before teardown existed).
# Integration tests clean up after themselves and don't need this.
db-deploy-clean: db-build
    spacetime publish \
        -s local \
        --delete-data \
        -y \
        --bin-path spacetimedb-module/target/wasm32-unknown-unknown/release/spacetimedb_module.wasm \
        trading-bot

# Full setup from scratch: build WASM, generate bindings, deploy module.
# Requires `just db-start` to already be running in a separate terminal.
db-setup: db-generate db-deploy

# Show status of the SpacetimeDB server and deployed module.
db-status:
    #!/usr/bin/env bash
    echo "── Server ───────────────────────────────────"
    if lsof -ti :3000 >/dev/null 2>&1; then
        echo "  ✅ Running  (PID $(lsof -ti :3000))"
    else
        echo "  ❌ Not running  →  just db-start"
    fi
    echo ""
    echo "── Module ───────────────────────────────────"
    if spacetime sql -s local trading-bot "SELECT COUNT(*) AS n FROM candles" >/dev/null 2>&1; then
        CANDLES=$(spacetime sql -s local trading-bot "SELECT COUNT(*) AS n FROM candles" 2>/dev/null \
            | grep -E '^[[:space:]]+[0-9]' | tr -d ' ' | head -1)
        echo "  ✅ trading-bot deployed"
        echo "  📊 candles in DB: ${CANDLES:-0}"
    else
        echo "  ❌ Module not deployed  →  just db-deploy"
    fi
    echo ""
    echo "── Tables ───────────────────────────────────"
    for TABLE in candles live_positions live_trades; do
        COUNT=$(spacetime sql -s local trading-bot "SELECT COUNT(*) AS n FROM $TABLE" 2>/dev/null \
            | grep -E '^[[:space:]]+[0-9]' | tr -d ' ' | head -1)
        echo "  $TABLE: ${COUNT:-?} rows"
    done

# Run integration tests against a live local SpacetimeDB instance.
# Requires: `just db-start` (Terminal 1) + `just db-setup` (once) before running.
db-test:
    SPACETIMEDB_INTEGRATION=1 cargo test -p db-layer --test integration -- --nocapture --test-threads=1

# Query candles via the CLI (requires running server + deployed module)
db-candles symbol="AAPL" timeframe="1d":
    spacetime sql -s local trading-bot \
        "SELECT timestamp, symbol, open, high, low, close, volume FROM candles \
         WHERE symbol = '{{symbol}}' AND timeframe = '{{timeframe}}' \
         ORDER BY timestamp DESC LIMIT 20"

# Show recent live trades
db-trades strategy="sma_cross":
    spacetime sql -s local trading-bot \
        "SELECT strategy, symbol, side, entry_price, exit_price, pnl FROM live_trades \
         WHERE strategy = '{{strategy}}' ORDER BY exit_time DESC LIMIT 20"

# View module logs (requires running server)
db-logs:
    spacetime logs -s local trading-bot

# ── Utility ────────────────────────────────────────────────────────────────

# Remove build artifacts
clean:
    cargo clean

# Show dependency tree
deps:
    cargo tree --workspace

# Watch and re-run tests on file changes  (requires: cargo-watch)
watch:
    cargo watch -x "test --workspace"
