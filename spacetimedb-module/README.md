# spacetimedb-module

SpacetimeDB server-side WASM module für TradingBot2.

Definiert Marktdaten, dedizierte Paper-Trading-Persistenz (`paper_open_positions`,
`paper_trades`) und transitional legacy `live_positions` / `live_trades` Tabellen.
**Keine** Trading-Logik — reiner Storage-Adapter.

---

## Voraussetzungen

```bash
# SpacetimeDB CLI installieren
curl -sSf https://install.spacetimedb.com | sh

# wasm32 target hinzufügen
rustup target add wasm32-unknown-unknown
```

---

## Setup (einmalig)

### Option A — All-in-one mit `spacetime dev` (empfohlen für Entwicklung)

```bash
# Startet den lokalen Server, kompiliert das Modul, publiziert es
# UND generiert die Rust-Client-Bindings in db-layer/src/module_bindings/
cd spacetimedb-module
spacetime dev --lang rust --out-dir ../db-layer/src/module_bindings
```

### Option B — Manuell

```bash
# 1. Server starten (in eigenem Terminal lassen)
just db-start
# oder: spacetime start

# 2. WASM bauen
just db-build

# 3. Modul deployen (erstellt "trading-bot" Datenbank)
just db-deploy

# 4. Rust-Bindings generieren
just db-generate
```

---

## Schema-Änderungen deployen

```bash
# Schema update (ohne Datenverlust, soweit SpacetimeDB Auto-Migration erlaubt)
just db-deploy

# Komplett neu (ALLE DATEN LÖSCHEN — nur in Dev!)
just db-deploy-clean
```

---

## Daten inspizieren

```bash
# Letzte 20 Candles für AAPL
just db-candles AAPL 1d

# Letzte Trades
just db-trades sma_cross

# Module Logs
just db-logs

# Direkt via CLI
spacetime sql -s local trading-bot "SELECT COUNT(*) FROM candles"
```

---

## HTTP API (für db-layer)

Der `db-layer` kommuniziert direkt per HTTP REST mit SpacetimeDB:

| Endpoint | Methode | Zweck |
|---|---|---|
| `/v1/{module}/sql` | POST | SQL-Queries (SELECT) |
| `/v1/{module}/call/{reducer}` | POST | Reducer aufrufen (INSERT/UPDATE) |
| `/v1/database/{module}/subscribe` | WebSocket | Live-Updates (für UI, M6) |

Konfiguration via Umgebungsvariablen:
```bash
export SPACETIMEDB_URL=http://localhost:3000
export SPACETIMEDB_MODULE=trading-bot
# export SPACETIMEDB_TOKEN=...  # optional für Auth
```

---

## Tabellen-Schema

### `candles`
| Feld | Typ | Beschreibung |
|---|---|---|
| `id` | `u64` | Auto-increment PK |
| `canonical_id` | `String` | Unique: `{symbol}_{timeframe}_{timestamp_ms}` |
| `timestamp` | `i64` | Candle-Öffnungszeit (Unix ms) |
| `symbol` | `String` | z.B. `"AAPL"`, `"BTC-USD"` |
| `open/high/low/close` | `f64` | OHLC |
| `volume` | `f64` | Fractional-safe |
| `timeframe` | `String` | z.B. `"1m"`, `"1h"`, `"1d"` |
| `provider` | `String` | z.B. `"yahoo"`, `"binance"` |

### `live_positions` (transitional legacy)

Runtime-backed Paper Trading nutzt `paper_open_positions`; `live_positions` bleibt
nur als Legacy-/Admin-Speicher erhalten.

| Feld | Typ | Beschreibung |
|---|---|---|
| `id` | `u64` | Auto-increment PK |
| `strategy` | `String` | Strategie-Name |
| `symbol` | `String` | Ticker |
| `side` | `String` | `"long"` oder `"short"` |
| `entry_price/size` | `f64` | Position-Details |
| `stop_loss/take_profit` | `f64` | Risk-Management (0.0 = nicht gesetzt) |
| `entry_time` | `i64` | Unix ms |
| `entry_reason` | `String` | Log-Nachricht |

### `live_trades` (transitional legacy)

Runtime-backed Paper Trading nutzt `paper_trades`; `live_trades` bleibt nur als
Legacy-/Admin-Speicher erhalten.

| Feld | Typ | Beschreibung |
|---|---|---|
| `id` | `u64` | Auto-increment PK |
| `strategy/symbol/side` | `String` | Trade-Identifikation |
| `entry_price/exit_price` | `f64` | Preise |
| `size/pnl` | `f64` | Größe + Gewinn/Verlust |
| `status` | `String` | `"open"` oder `"closed"` |
| `entry_time/exit_time` | `i64` | Unix ms |
| `entry_reason/exit_reason` | `String` | Log-Nachrichten |

### `paper_open_positions`
| Feld | Typ | Beschreibung |
|---|---|---|
| `projection_key` | `String` | Deterministischer PK für die Runtime-Position |
| `strategy_identity/runtime_asset/side` | `String` | Paper-Trading-Projektionsgrenze |
| `entry_price/quantity` | `f64` | Runtime-Portfolio-Daten |
| `entry_time` | `i64` | Unix ms |
| `stop_loss/take_profit` | `Option<f64>` | Optionale Position Risk Boundaries, keine Sentinel-Werte |
| `entry_metadata` | `Option<String>` | Optionale Adapter-Metadaten |

### `paper_trades`
| Feld | Typ | Beschreibung |
|---|---|---|
| `projection_key` | `String` | Deterministischer PK für den abgeschlossenen Runtime-Trade |
| `strategy_identity/runtime_asset/side` | `String` | Paper-Trading-Projektionsgrenze |
| `entry_price/exit_price/quantity/realized_pnl` | `f64` | Runtime-Portfolio-Daten |
| `entry_time/exit_time` | `i64` | Unix ms |
| `stop_loss/take_profit` | `Option<f64>` | Persistierte Position Risk Boundaries |
| `exit_kind` | `PaperExitKind` | `StrategyExit`, `RiskExitStopLoss`, `RiskExitTakeProfit`, `ForceClose` |
| `entry_metadata/exit_metadata` | `Option<String>` | Optionale Adapter-Metadaten |
