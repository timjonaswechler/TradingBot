# Trading Bot Architecture (V2)

## Vision
Ein professioneller, zustandsbehafteter (stateful) Trading-Bot in Rust. SpacetimeDB dient ausschließlich als extrem schneller Data-Lake (Speicherung von Candles und Live-Trades). Die gesamte Trading-Logik läuft in einer dauerhaft aktiven Rust-Engine (Daemon), die Live-Daten verarbeitet, Indikatoren effizient im Speicher berechnet ($O(1)$) und Positionen verwaltet. 
Backtests laufen zu 100% In-Memory und sind massiv parallelisierbar, perfekt für die Integration mit GPUI.

---

## Ausführbare Dateien (Binaries)

Das System besteht konzeptionell aus **drei laufenden Prozessen**. Zwei davon baust du selbst, der dritte ist die Datenbank.

### 1. `spacetimedb` Server (Third-Party)
- **Zweck:** Der extrem schnelle, RAM-basierte Data-Lake.
- **Aufgabe:** Speichert historische OHLCV-Daten (`candles`), offene Live-Trades (`live_positions`) und eine Historie aller Live-Trades (`live_trades`).
- **Besonderheit:** Enthält **keine Trading-Logik**. Das WASM-Modul (`spacetimedb-module/`) definiert nur das Schema (Tabellen) und minimale CRUD-Reducer (insert/delete). Clients verbinden sich per WebSocket via `spacetimedb-sdk` und halten einen lokalen Cache der subscriebten Tabellen.

### 2. `trading-daemon` (Dein Rust-Backend)
- **Zweck:** Der Headless-Service (ohne UI) für das echte, laufende Trading (Paper oder Live).
- **Start:** Wird beim Systemstart oder manuell über die Konsole gestartet und läuft dauerhaft im Hintergrund (Tokio Async Runtime).
- **Aufgaben:**
  - **Data Fetcher:** Ein asynchroner Task wacht z.B. alle 30 Minuten auf, holt neue Kerzen (Candles) uber die Broker/Provider-API und speichert sie in SpacetimeDB.
  - **Live Trading Engine:** Ein weiterer Task halt den "State" der Indikatoren im Arbeitsspeicher warm. Sobald der Data Fetcher eine neue Kerze in die DB schreibt (oder direkt weitergibt), tickt die Engine *eine* neue Candle weiter ($O(1)$) und fuhrt das Rhai-Skript aus.
  - **Order Execution:** Wenn das Skript "BUY" sagt, kommuniziert der Daemon mit der Broker-API und speichert den Trade in der DB.

### 3. `trading-ui` (Dein GPUI Frontend)
- **Zweck:** Das visuelle Interface fur Charting, Strategie-Entwicklung und Backtesting.
- **Start:** Wird manuell vom Nutzer gestartet wie eine normale Desktop-App.
- **Aufgaben:**
  - **Charting & Visualisierung:** Verbindet sich per `spacetimedb-sdk` (WebSocket) mit SpacetimeDB, subscribed die `candles`-Tabelle und ladt die Kerzen aus dem lokalen Cache in den Chart.
  - **In-Memory Backtester:** Wenn du einen Backtest startest, nutzt die UI *dieselbe* Rust-Engine-Bibliothek wie der Daemon. Sie ladt tausende Kerzen aus der DB in den RAM und jagt sie in Millisekunden durch die Engine.
  - **Keine DB-Schreibzugriffe:** Die UI schreibt beim Backtesten **nichts** in die Datenbank (kein Simulations-Mull). Die Ergebnisse werden direkt im RAM gehalten und als PnL-Kurve oder Trade-Liste in der UI angezeigt.

---

## Projektstruktur & Crates (Workspace)

Um Code-Duplizierung zu vermeiden, teilen sich Daemon und UI denselben Core-Code:

```text
trading-bot/
├── Cargo.toml                          # Workspace Definition
├── ARCHITECTURE.md                     # Dieses Dokument
│
├── shared/                             # Crate: Gemeinsame Typen (OHLCV, Trade, Signal)
├── db-layer/                           # Crate: SpacetimeDB SDK Client, Queries, generierte Bindings
├── indicators/                         # Crate: Alle Indikatoren als pure Rust-Funktionen
├── engine/                             # Crate: Core Logic, Rhai-Scripting, O(1) Indicator Cache
├── spacetimedb-module/                 # WASM-Modul: Schema + CRUD-Reducer (kein Workspace-Member)
│
├── trading-daemon/                     # Binary 1: Das Backend (Tokio, Broker API, Live-Trading)
├── trading-ui/                         # Binary 2: Das Frontend (GPUI, Charting, In-Memory Backtester)
└── strategies/                         # Rhai-Strategiedateien (z.B. sma_cross.rhai)
```

---

## Datenfluss & Interaktion

### A. Der Live-Trading Datenfluss (Im `trading-daemon`)

```text
[Broker API / Yahoo] 
       │
       ▼ (1. Async Fetch alle X Minuten)
[trading-daemon: Data Fetcher Task]
       │
       ├─► (2. Speichern zur Historisierung) ─► [SpacetimeDB: `candles` Tabelle]
       │
       ▼ (3. Ubergabe der exakt EINEN neuen Candle im RAM)
[trading-daemon: Live Engine Task] 
       │
       ├─► O(1) State Update (Indikatoren, EMA, CCI)
       ├─► Lua/Rhai Skript Ausfuhrung
       │
       ▼ (4. Bei Signal: BUY/SELL)
[Broker API] (Order Ausfuhrung)
       │
       ▼ (5. Dokumentation des Trades)
[SpacetimeDB: `live_positions` / `live_trades` Tabelle]
```

**Wie das Problem des "Warmups" (Startdatum) gelost wird:**
Wenn der `trading-daemon` fur eine Strategie gestartet wird (z.B. Startdatum heute, 14:00 Uhr), liest er im Skript aus, was der hochste benotigte Lookback ist (z.B. MACD 26). 
Er fragt die SpacetimeDB: *"Gib mir die 26 Candles VOR heute 14:00 Uhr"*.
Mit diesen 26 Kerzen futtert er die Indikatoren initial. Ab 14:00 Uhr wartet er dann nur noch auf neue, einzelne Kerzen.

### B. Der Backtest Datenfluss (In der `trading-ui`)

```text
[Nutzer klickt "Start Backtest AAPL, 2020 bis 2024"]
       │
       ▼ (1. SDK WebSocket — lokaler Cache)
[SpacetimeDB] ──► Liefert 50.000 Candles aus dem subscriebten Cache
       │
       ▼ (2. Ubergabe an In-Memory Engine)
[trading-ui: Backtest Runner (Rayon / Parallel)]
       │
       ├─► Tickt in einer for-Schleife alle 50.000 Candles durch
       ├─► Sammelt alle simulierten Trades in einem `Vec<Trade>` im RAM
       │
       ▼ (3. Ruckgabe der Ergebnisse)
[GPUI Chart / Dashboard] ──► Zeichnet Drawdown, PnL, Trade-Marker
```

**Zusammenspiel von Daemon und UI:**
Der Daemon und die UI wissen im Idealfall gar nichts voneinander. Sie kommunizieren indirekt uber die SpacetimeDB.
Wenn der Daemon einen Live-Trade macht, landet er in der SpacetimeDB. Wenn du die UI offnest, liest sie die SpacetimeDB aus und zeigt dir: *"Aha, der Daemon hat gerade AAPL gekauft."* 
Du kannst die UI jederzeit schliessen - der Daemon lauft im Hintergrund sicher weiter.

---

## Warum dieses Design die Probleme aus V1 löst

1. **O(1) statt O(N^2):** Weil der `trading-daemon` dauerhaft lauft, bleibt der Indikator-State (z.B. der EMA) im RAM erhalten. Bei einer neuen Kerze wird nur die Formel `(Close - EMA_alt) * Multiplier + EMA_alt` gerechnet. In V1 (ephemere Binaries) musste bei jedem Tick die gesamte Historie neu iteriert werden.
2. **Keine verpassten Trades:** In V1 hat Cron starr alle 5 Minuten getriggert, egal ob der Broker punktlich geliefert hat oder nicht. Im neuen `trading-daemon` lauft ein asynchroner Task, der genau weiss, wann eine Kerze wirklich *geschlossen* und vollstandig ist, bevor er sie an die Engine ubergibt.
3. **Isolierte Backtests:** In V1 hat der Backtester die Ergebnisse in die DB geschrieben. Das verlangsamt extrem und mullt die DB voll. In V2 lauft das rein im RAM der GPUI-App.
4. **PineScript / TradingView Semantik:** Strategien wie der `3-in-1 CCI Trader` lassen sich nur sauber abbilden, wenn Variablen wie `cciPeriod6[1]` (die letzte Kerze) oder `ta.barssince()` stateful im Speicher gehalten werden. Genau das macht das Crate `engine` nun.

---

## Geplante Indikatoren

### Trend

| Indikator     | Input                       | Output         | Beschreibung                                                      |
|---------------|-----------------------------|----------------|-------------------------------------------------------------------|
| SMA           | closes, period              | f64            | Simple Moving Average                                             |
| EMA           | closes, period              | f64            | Exponential Moving Average                                        |
| DEMA          | closes, period              | f64            | Double EMA (weniger Lag als EMA)                                  |
| TEMA          | closes, period              | f64            | Triple EMA (noch weniger Lag)                                     |
| MACD          | closes, fast, slow, signal  | MacdResult     | Trend + Momentum ({line, signal, histogram})                      |
| Parabolic SAR | candles, step, max          | f64            | Stop-and-Reverse Trendfolge                                       |
| ADX           | candles, period             | f64 (0-100)    | Trendstarke (ohne Richtung)                                       |
| Ichimoku      | candles                     | IchimokuResult | Cloud, Spannen, Chikou ({tenkan, kijun, span_a, span_b, chikou}) |

### Momentum

| Indikator   | Input           | Output      | Beschreibung                           |
|-------------|-----------------|-------------|----------------------------------------|
| RSI         | closes, period  | f64 (0-100) | Overbought/Oversold                    |
| Stochastic  | candles, period | {k, d}      | Stochastic Oscillator                  |
| CCI         | candles, period | f64         | Commodity Channel Index                |
| Williams %R | candles, period | f64 (0-100) | Overbought/Oversold (invertiert)       |
| ROC         | closes, period  | f64         | Rate of Change (Preisveranderung %)    |

### Volatilitat

| Indikator        | Input                   | Output                 | Beschreibung                      |
|------------------|-------------------------|------------------------|-----------------------------------|
| Bollinger Bands  | closes, period, std_dev | BbResult               | Volatility Bands ({upper, middle, lower}) |
| ATR              | candles, period         | f64                    | Average True Range                |
| Keltner Channels | candles, period, mult   | {upper, middle, lower} | ATR-basierte Bander               |

### Volumen

| Indikator      | Input           | Output               | Beschreibung                              |
|----------------|-----------------|----------------------|-------------------------------------------|
| OBV            | candles         | f64                  | On-Balance Volume (Volumen-Trendindikator)|
| VWAP           | candles         | f64                  | Volume Weighted Average Price             |
| Volume Profile | candles         | Vec<{price, volume}> | Volumen je Preiszone                      |
| MFI            | candles, period | f64 (0-100)          | Money Flow Index (Volume-RSI)             |

### Support/Resistance

| Indikator              | Input      | Output                         | Beschreibung                              |
|------------------------|------------|--------------------------------|-------------------------------------------|
| Pivot Points           | candles    | {pp, r1, r2, r3, s1, s2, s3}  | Klassische Pivot-Level                    |
| Fibonacci Retracements | high, low  | Vec<f64>                       | 23.6%, 38.2%, 50%, 61.8%, 78.6%          |

### Sonstige

| Indikator | Input          | Output | Beschreibung                                     |
|-----------|----------------|--------|--------------------------------------------------|
| Slope     | closes, period | f64    | Lineare Regressions-Steigung uber N Perioden     |

---

## Dependencies (Cargo.toml Workspace)

```toml
[workspace]
members = [
    "shared",
    "indicators",
    "db-layer",
    "engine",
    "trading-daemon",
    "trading-ui",
]
# spacetimedb-module ist kein Workspace-Member (wasm32-unknown-unknown Target)
exclude = ["spacetimedb-module"]

[workspace.dependencies]
# Core
serde      = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
thiserror  = "2.0"
anyhow     = "1.0"

# SpacetimeDB client SDK (WebSocket, generierte Bindings)
spacetimedb-sdk = { version = "2" }
# spacetimedb = "2"  # nur in spacetimedb-module (server-side WASM)

# Rhai Scripting
rhai = { version = "1", features = ["sync"] }

# HTTP / Broker APIs
reqwest = { version = "0.12", features = ["json"] }
tokio   = { version = "1",    features = ["full"] }

# Parallelisierung (Backtesting)
rayon = "1.10"

# CLI
clap = { version = "4", features = ["derive"] }

# Logging
tracing            = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
```
