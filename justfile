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

# Run tests for a specific crate  (e.g. `just test-crate shared`)
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
    cargo run -p trading-daemon -- {{args}}

# Start the trading UI
ui *args:
    cargo run -p trading-ui -- {{args}}

# ── SpacetimeDB ────────────────────────────────────────────────────────────

# Start a local SpacetimeDB server (foreground)
db-start:
    spacetimedb start

# Deploy the schema to the local SpacetimeDB instance
db-deploy:
    spacetimedb publish --skip-clippy trading-bot

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
