# Trading Bot Architecture Design

## Vision

Ein modularer Trading Bot mit getrennten, ephemeralen Binaries. Rust als performantes Backend für Datenverarbeitung und Indikatoren, Lua als flexible Scripting-Sprache für Strategien. SpacetimeDB als zentrale Datenbank für OHLCV-Daten und Kommunikation.

**Kein dauerhafter Service** (außer dem SpacetimeDB-Server selbst). Jedes Binary wird von Cron oder manuell gestartet, erledigt seine Aufgabe und beendet sich.

---

## Architektur-Übersicht

### Vier unabhängige Binaries

```
┌─────────────────────────────────────────────────────────────────┐
│  runner (Binary 0 – Coordinator)                                │
│  Zweck: Startet data-fetcher, wartet auf Exit, startet runner   │
│  Start: Cron (alle 5 Min)                                       │
│                                                                 │
│  data-fetcher ──> (Exit) ──> strategy-runner ──> Exit           │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  data-fetcher (Binary 1)                                        │
│  Zweck: Daten von Provider holen und in DB schreiben            │
│  Start: Über runner oder manuell                                │
│                                                                 │
│  Provider API ──> SpacetimeDB (candles table) ──> Exit          │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  strategy-runner (Binary 2)                                     │
│  Zweck: Paper Trading – eine Decision pro Run                   │
│  Start: Über runner (nach data-fetcher) oder manuell            │
│                                                                 │
│  DB (candles) ──> Lua (entscheidet) ──> DB (trades) ──> Exit   │
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  backtester (Binary 3)                                          │
│  Zweck: Historische Analyse – Performance-Report                │
│  Start: Manuell wenn man will                                   │
│                                                                 │
│  DB (candles) ──> Replay ──> Lua ──> backtest_* Tables ──> Exit│
└─────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────┐
│  UI (GPUI - später)                                             │
│  Liest Candles aus DB, berechnet Backtest in-memory             │
│  Enthält Strategy Editor (Lua-Dateien bearbeiten/verwalten)     │
└─────────────────────────────────────────────────────────────────┘
```

### Infrastruktur-Hinweis

```
SpacetimeDB Server – der einzige dauerhaft laufende Prozess.
Alle anderen Binaries sind ephemeral (start → work → exit).

SpacetimeDB wurde gewählt wegen:
- Extrem schneller Lese-Performance (tausende Queries/Sekunde)
- WebSocket-Subscriptions ideal für UI (live updates ohne polling)
- Gut geeignet für viele parallele Anfragen beim Backtesting
```

### Cron-Ablauf

```
Cron alle 5 Min:
  │
  └─> runner --strategy sma_cross.lua --symbol AAPL --interval 5m
        │
        ├─> data-fetcher --provider yahoo --symbols AAPL --interval 5m
        │     → Holt Daten vom Provider
        │     → Filtert unfertige Candles (Provider-spezifisch)
        │     → Schreibt in SpacetimeDB
        │     → Exit (runner wartet)
        │
        └─> strategy-runner --strategy sma_cross.lua --symbol AAPL
              → Liest Candles aus DB
              → Lua ruft Indikatoren ON DEMAND
              → Lua trifft EINE Decision
              → Simulierter Trade wird geloggt
              → Exit
```

---

## Projektstruktur

```
trading-bot/
├── Cargo.toml                          # Workspace Definition
├── DESIGN.md                           # Dieses Dokument
├── README.md                           # Projekt-Übersicht
│
├── shared/                             # Shared Types & Traits
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                      # Public Exports
│       ├── candle.rs                   # OHLCV Struct
│       ├── signal.rs                   # Buy/Sell/Hold/Short Enum
│       ├── position.rs                 # Position Tracking
│       ├── context.rs                  # Context für Lua (Balance, Position)
│       └── paper_trader.rs             # Simuliert Trades – genutzt von strategy-runner UND backtester
│
├── db-layer/                           # Crate 1: Datenbank-Zugriff
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── client.rs                   # SpacetimeDB Client
│       └── queries.rs                  # Query-Helper (get_candles, latest, etc.)
│
├── spacetimedb-module/                 # SpacetimeDB Module (Server-Side)
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs                      # Table Definitions + Reducer (deployed auf SpacetimeDB)
│
├── indicators/                         # Crate 2: Technische Indikatoren
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs                      # Public API + Indicator Trait
│       ├── trend/
│       │   ├── sma.rs                  # Simple Moving Average
│       │   ├── ema.rs                  # Exponential Moving Average
│       │   ├── dema.rs                 # Double EMA
│       │   ├── tema.rs                 # Triple EMA
│       │   ├── macd.rs                 # MACD
│       │   ├── sar.rs                  # Parabolic SAR
│       │   ├── adx.rs                  # Average Directional Index
│       │   └── ichimoku.rs             # Ichimoku Cloud
│       ├── momentum/
│       │   ├── rsi.rs                  # Relative Strength Index
│       │   ├── stochastic.rs           # Stochastic Oscillator
│       │   ├── cci.rs                  # Commodity Channel Index
│       │   ├── williams_r.rs           # Williams %R
│       │   └── roc.rs                  # Rate of Change
│       ├── volatility/
│       │   ├── bollinger.rs            # Bollinger Bands
│       │   ├── atr.rs                  # Average True Range
│       │   └── keltner.rs              # Keltner Channels
│       ├── volume/
│       │   ├── obv.rs                  # On-Balance Volume
│       │   ├── vwap.rs                 # Volume Weighted Average Price
│       │   ├── volume_profile.rs       # Volume Profile
│       │   └── mfi.rs                  # Money Flow Index
│       └── support_resistance/
│           ├── pivot_points.rs         # Pivot Points
│           └── fibonacci.rs            # Fibonacci Retracements
│
├── lua-engine/                         # Crate 3: Lua Runtime
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── vm.rs                       # Lua VM Management (mlua)
│       ├── bindings.rs                 # Rust→Lua Bindings + Indicator-Cache
│       ├── indicator_cache.rs          # Stateful Indicator State (incremental updates)
│       ├── strategy_loader.rs          # Lädt & validiert .lua Dateien
│       ├── candle_wrapper.rs           # Candle Access in Lua (1-indexed, neueste zuerst, lazy DB fetch)
│       └── lua/
│           └── api.lua                 # Lua Helper Library (wird automatisch geladen)
│
├── runner/                             # Binary 0: Coordinator
│   ├── Cargo.toml
│   └── src/
│       └── main.rs                     # Startet data-fetcher → wartet → startet strategy-runner
│
├── data-fetcher/                       # Binary 1: Provider → DB
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                     # Entry Point + CLI
│       ├── providers/
│       │   ├── mod.rs                  # DataProvider Trait
│       │   ├── yahoo.rs                # Yahoo Finance (⚠ live candle filtern!)
│       │   ├── twelve_data.rs          # Twelve Data API
│       │   └── broker.rs               # Broker (später, z.B. Alpaca/IBKR)
│       └── db_writer.rs                # Schreibt Candles nach SpacetimeDB
│
├── strategy-runner/                    # Binary 2: Paper Trading
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                     # Entry Point
│       ├── runner.rs                   # Orchestriert: DB → Lua → Trade
│       └── config.rs                   # CLI Argumente
│
├── backtester/                         # Binary 3: Historische Analyse
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                     # Entry Point
│       ├── engine.rs                   # Backtest Engine (Candle-für-Candle Replay)
│       ├── metrics.rs                  # Performance Metriken
│       └── report.rs                   # Report Generator
│
└── strategies/                         # Lua Strategien (kein Rust!)
    ├── sma_cross.lua
    ├── rsi_reversal.lua
    └── macd_divergence.lua
```

---

## Data Providers

### Provider Trait

```rust
// data-fetcher/src/providers/mod.rs

pub trait DataProvider {
    async fn fetch_candles(
        &self,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Candle>>;
}
```

### Yahoo Finance

```rust
// data-fetcher/src/providers/yahoo.rs

// ACHTUNG: Yahoo Finance liefert die aktuelle, noch nicht abgeschlossene Candle mit.
// Diese muss herausgefiltert werden, da sie unvollständige OHLCV-Daten enthält.
//
// Filter: candle.timestamp + interval_duration_ms < now_ms
//
// Außerdem: Yahoo Finance nutzt eine inoffizielle, undokumentierte API.
// Diese kann ohne Vorwarnung brechen. Für Produktiv-Einsatz Twelve Data oder
// Broker-API bevorzugen.
```

### CLI

```bash
data-fetcher --provider yahoo       --symbols AAPL,BTC-USD --interval 5m
data-fetcher --provider twelve_data --symbols AAPL         --interval 5m
data-fetcher --provider broker      --symbols AAPL         --interval 5m  # später
```

---

## Datenfluss

### 1. runner (Coordinator)

```
runner wird von Cron gestartet (--symbols AAPL,BTC-USD --strategy ... --timeout 120)
  │
  ├─> Startet data-fetcher --symbols AAPL,BTC-USD als Subprocess
  │     → Wartet auf Exit (max. --timeout Sekunden)
  │     → Timeout überschritten: Child-Process killen, runner bricht ab
  │     → Exit-Code != 0: runner bricht ab, strategy-runner wird NICHT gestartet
  │       (verhindert Trading auf veralteten Daten)
  │
  └─> Pro Symbol: startet strategy-runner --symbol AAPL (dann --symbol BTC-USD)
        → Ein strategy-runner = ein Symbol (klare Trennung)
        → Wartet auf Exit pro Symbol
        → Exit
```

Implementierung via `std::process::Command` – für sequentielle Ausführung ausreichend.
Bei vielen Symbolen parallel wäre async (tokio) effizienter – aktuell sequentiell pro Symbol.
Exit-Code-Prüfung: `status.success()` vor dem nächsten Schritt.
Timeout: `std::thread::spawn` + Channel oder `wait-timeout` crate.

### 2. data-fetcher (Daten holen)

```
data-fetcher wird gestartet (von runner oder manuell)
  │
  ├─> Wählt Provider (--provider flag)
  │
  ├─> Holt letzte Candles vom Provider
  │     - Symbole: z.B. AAPL, BTC-USD
  │     - Interval: 1m, 5m, 1h, 1d
  │     - Yahoo: filtert unfertige aktuelle Candle heraus
  │
  ├─> Schreibt neue Candles nach SpacetimeDB
  │     - Table: candles
  │     - Dedupliziert über (timestamp, symbol, timeframe) – unique constraint
  │
  └─> Exit
```

### 3. strategy-runner (Paper Trading)

**Lua Engine Nutzung:** Eine Instanz, `on_tick` wird einmal aufgerufen, dann Exit.
Indikatoren fetchen lazy aus DB was sie brauchen und berechnen von scratch (O(period)) – kein persistenter Cache da die Engine danach endet.

```
strategy-runner wird gestartet (von runner oder manuell)
  │
  ├─> Liest offene Position aus DB (falls vorhanden)
  │     - Table: positions
  │
  ├─> Erstellt Lua Engine Instanz
  │     → LuaCandles mit DB-Referenz (lazy loading)
  │
  ├─> on_tick(candles, context) – einmal aufgerufen
  │     │
  │     ├─> Lua ruft: indicators.rsi(candles, 14)
  │     │     → Binding: braucht 14 Candles → fetcht 14 aus DB
  │     ├─> Lua ruft: indicators.sma(candles, 20)
  │     │     → Binding: 14 bereits geladen → fetcht 6 weitere aus DB
  │     └─> Lua trifft Decision
  │           Return: { signal, size?, stop_loss?, take_profit? }
  │
  ├─> paper_trader verarbeitet Decision
  │     - Simuliert Trade
  │     - Updated Position in DB
  │     - Schreibt Trade nach DB (Table: trades)
  │
  └─> Exit
```

### 4. backtester (Historische Analyse)

**Lua Engine Nutzung:** Eine Instanz, `on_tick` wird N-mal aufgerufen (einmal pro Candle).
Die Engine bleibt am Leben – der Indicator-Cache akkumuliert State über alle Candles.

```
Indicator-Cache + Lazy Loading (in lua-engine/src/indicator_cache.rs):

  Cache-Schlüssel: (IndikatorTyp, period, offset)
                    Jeder Offset ist ein eigener Eintrag – kein Rolling Buffer.
  Cache-Wert:      Einzelner berechneter Wert (f64 oder Struct)

  Lua-Sicht (neueste zuerst, 1-indexed):
    rsi(candles, 14, 0) → candles[1] bis candles[14]   (neueste 14)
    rsi(candles, 14, 1) → candles[2] bis candles[15]   (gleiches Fenster, 1 nach hinten)

  Candle 1 (start_date):
    rsi(candles, 14, 0) → Cache miss → candles[1..14]  → berechnen → cached[("rsi",14,0)]
    rsi(candles, 14, 1) → Cache miss → candles[2..15]  → berechnen → cached[("rsi",14,1)]

  Candle 2 (neue Candle available, Lua verschiebt sich):
    rsi(candles, 14, 0) → Cache hit → candles[1] ist neu, rest gleich → O(1) Update
    rsi(candles, 14, 1) → Cache hit → candles[2] ist neu, rest gleich → O(1) Update

  Candle N:
    Alle Calls → O(1), nur +1 neue Candle pro Tick aus DB
```

```
backtester wird mit Parameter gestartet
  │
  ├─> Lädt Strategie (.lua Datei)
  │
  ├─> Generiert session_id (auto_inc in DB)
  │
  ├─> Erstellt Lua Engine Instanz (bleibt für alle Candles am Leben)
  │     → LuaCandles mit DB-Referenz (lazy loading)
  │     → Indicator-Cache lebt in der Engine-Instanz
  │
  ├─> Engine: Iteriert start_date → end_date, Candle für Candle
  │     │
  │     └─> Pro Candle:
  │           - on_tick(candle, context) wird aufgerufen
  │           - Indikator-Call (z.B. rsi(candles, 14)):
  │               Cache miss (erster Aufruf): fetcht N Candles lazy aus DB → berechnet → cached
  │               Cache hit  (Folge-Aufruf):  +1 neue Candle aus DB → O(1) Update
  │               Daten fehlen in DB          → Abbruch: "Nicht genug Daten für RSI(14) ab 2020-01-01"
  │           - Lua trifft Decision
  │           - paper_trader verarbeitet Decision (gleicher paper_trader aus shared/)
  │           - Signal + Trade werden geloggt
  │           - Kein Look-ahead: Lua sieht nur vergangene Candles
  │
  ├─> Schreibt Ergebnisse nach DB
  │     - backtest_sessions (session_id, parameter, ergebnis)
  │     - backtest_signals  (session_id)
  │     - backtest_trades   (session_id)
  │
  └─> metrics: Generiert Report
        - Total Return
        - Sharpe Ratio
        - Max Drawdown
        - Win Rate
        - Trade List
```

### 5. UI Workflow (GPUI - später)

```
UI startet
  │
  ├─> User wählt: Strategie + Symbol + Zeitraum
  │
  ├─> Holt Candles aus DB
  │
  ├─> Berechnet Backtest in-memory
  │     - Nutzt dieselben Crates wie backtester Binary
  │     - Alles im Speicher, kein DB-Schreiben
  │
  ├─> Zeigt Ergebnisse direkt an
  │     - Signale: Warum wurde BUY/SELL entschieden?
  │     - Trades: Entry, Exit, PnL
  │
  ├─> Strategy Editor
  │     - Liste aller .lua Dateien in strategies/
  │     - Eingebauter Text-Editor zum Bearbeiten
  │     - Beim nächsten Run/Backtest wird gespeicherte Datei verwendet
  │
  └─> User wechselt Auswahl
        - Neuer Backtest wird in-memory berechnet
        - Keine DB-Operation nötig
```

---

## Shared Types

### Candle (`shared/src/candle.rs`)

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Candle {
    pub timestamp: i64,       // Unix timestamp (ms)
    pub symbol: String,       // z.B. "AAPL", "BTC-USD"
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,          // f64: deckt Aktien (ganzzahlig) und Crypto (fraktional, z.B. 0.5 BTC)
}

impl Candle {
    pub fn body(&self) -> f64 { (self.close - self.open).abs() }
    pub fn is_bullish(&self) -> bool { self.close > self.open }
    pub fn is_bearish(&self) -> bool { self.close < self.open }
    pub fn range(&self) -> f64 { self.high - self.low }
}
```

### Signal (`shared/src/signal.rs`)

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Buy,
    Sell,
    Hold,
    Short,
    Cover,          // Short-Position schließen
}

#[derive(Debug, Clone)]
pub struct TradeDecision {
    pub signal: Signal,
    pub size: f64,                 // 0.0–1.0 (Portfolio-Anteil, z.B. 0.1 = 10% des Kapitals)
    pub stop_loss: Option<f64>,    // Optionaler Stop-Loss Preis
    pub take_profit: Option<f64>,  // Optionaler Take-Profit Preis
    pub reason: Option<String>,    // Warum diese Decision? (für Logging)
}
// size-Semantik: paper_trader konvertiert Anteil → Shares:
//   shares = (context.balance * decision.size) / entry_price
// Position.size speichert dann die Anzahl Shares/Kontrakte (nicht den Anteil).
```

### Position (`shared/src/position.rs`)

```rust
#[derive(Debug, Clone)]
pub struct Position {
    pub symbol: String,
    pub side: PositionSide,        // Long oder Short
    pub entry_price: f64,
    pub size: f64,                 // Anzahl Shares/Kontrakte
    pub entry_time: i64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PositionSide {
    Long,
    Short,
}
```

### Context (`shared/src/context.rs`)

```rust
#[derive(Debug, Clone)]
pub struct Context {
    pub balance: f64,              // Verfügbares Kapital
    pub position: Option<Position>,
    pub equity: f64,               // balance + position_value
    pub trades_count: u32,
}
```

---

## Lua API Design

### Konzept: Sliding Window + On-Demand Indicators

Jedes Mal wenn `on_tick` aufgerufen wird, erhält die Funktion ein "Fenster" an Candles:

```
candles[1] = aktuelle Candle (neueste, gerade geschlossen)
candles[2] = eine davor
candles[N] = N Candles zurück
```

**Zwei API-Ebenen – wichtig zu verstehen:**

```
Lua-Ebene (strategy-seitig):
  indicators.rsi(candles, 14)        ← nimmt candles-Objekt direkt
  indicators.atr(candles, 14)        ← gleiche API für alle Indikatoren

Rust-Ebene (intern in indicators/):
  pub fn rsi(closes: &[f64], ...) -> Option<f64>   ← reine Mathematik
  pub fn atr(candles: &[Candle], ...) -> Option<f64>

Die Binding-Schicht (lua-engine/src/bindings.rs) übersetzt zwischen beiden:
  LuaCandles → extrahiere closes → rufe rsi(&closes, period) auf
```

Lua muss nicht wissen welche Price-Series ein Indikator braucht – die Bindings regeln das.

**Optionaler `offset`-Parameter:** Jeder Indikator akzeptiert einen dritten Parameter für den Wert N Candles zurück (default = 0 = aktuell). Ermöglicht Crossover-Erkennung:

```lua
local rsi_now  = indicators.rsi(candles, 14)     -- aktueller Wert
local rsi_prev = indicators.rsi(candles, 14, 1)  -- Wert 1 Candle zurück
local crossed_up = rsi_prev < 30 and rsi_now >= 30
```

**Vertrag bei unzureichenden Daten:** Wenn weniger Candles vorhanden sind als der Indikator braucht, gibt Rust `nil` zurück (analog zu TradingViews `na`). Gilt auch bei out-of-bounds `offset`. Strategien müssen nil prüfen:

```lua
local rsi = indicators.rsi(candles, 14)
if rsi == nil then
    return { signal = "HOLD", reason = "insufficient data" }
end
```

### Candle Indexierung

```
Intern (Rust): Vec<Candle> chronologisch, älteste zuerst
               [alt, ..., neu]  Index 0 = älteste

Lua-seitig:    1-indexed, neueste zuerst
               candles[1] = neueste  →  internes Vec[last]
               candles[2] = eine davor →  internes Vec[last-1]
               Out-of-bounds → nil (kein Fehler)

Hilfsmethoden (closes, opens, etc.): neueste zuerst – konsistent mit candles[1].
  candles[1]          = neueste Candle
  candles:closes()[1] = neuester Close-Preis

  Indikatoren nehmen candles direkt – Rust dreht intern um, Lua muss nicht darüber nachdenken.

#candles: __len Metamethod vorhanden → #candles gibt Anzahl verfügbarer Candles zurück.
```

### Start-Datum & Daten-Verfügbarkeit (Backtesting)

**Das Start-Datum ist Candle 1 – kein Vorlauf, kein Warmup.** Indikatoren fetchen bei ihrem ersten Aufruf lazy aus der DB wie viele Candles sie brauchen. Sind die Daten nicht vorhanden, bricht der Backtester mit einer klaren Fehlermeldung ab.

```bash
backtester --strategy sma_cross.lua --start 2020-01-01 --end 2024-01-01
# → Candle 1 = 2020-01-01
# → MACD(26) beim ersten on_tick: fetcht 26 Candles aus DB (bis 2020-01-01)
# → Daten nicht vorhanden → ERROR: "Nicht genug Daten für MACD(26) ab 2020-01-01"
# → Kein min_bars, kein max_candles, keine interne Konfiguration
```

Ablauf:
```
--start 2020-01-01

  Schritt 1: Erstelle Engine mit DB-Referenz, starte bei start_date

  Schritt 2: Iteriere start_date → end_date, Candle für Candle:
               - on_tick(candle, context) wird aufgerufen
               - Indikator-Calls fetchen lazy aus DB (aktuelle + N zurück je nach period)
               - Erste Call: O(period) – fetcht N Candles, baut Cache-State auf
               - Folge-Calls: +1 Candle aus DB, O(1) via Indicator-Cache
               - Daten fehlen → Abbruch: "Nicht genug Daten für <Indikator> ab <Datum>"
               - Signal + Trade werden geloggt
```

**Kein Look-ahead:** Lua sieht auf jeder Candle nur die Vergangenheit, nie die Zukunft.

### Was Lua zur Verfügung steht

```lua
-- strategies/example.lua

local config = {
    name = "example",
}

function on_tick(candles, context)
  -- ============================================================
  -- CANDLE ACCESS (1-indexed, neueste zuerst)
  -- ============================================================

  local current  = candles[1]       -- aktuelle Candle
  local previous = candles[2]       -- eine davor
  local older    = candles[6]       -- 5 Candles zurück

  -- OHLCV Zugriff
  local price = current.close
  local high  = current.high
  local low   = current.low
  local open  = current.open
  local vol   = current.volume
  local ts    = current.timestamp

  -- Candle Properties
  local body    = current:body()       -- |close - open|
  local range   = current:range()      -- high - low
  local bullish = current:is_bullish()
  local bearish = current:is_bearish()

  -- Hilfsmethoden für Custom-Berechnungen (neueste zuerst, wie candles[1])
  local closes  = candles:closes()   -- closes[1] = aktueller Close
  local opens   = candles:opens()
  local highs   = candles:highs()
  local lows    = candles:lows()
  local volumes = candles:volumes()

  -- ============================================================
  -- INDICATORS
  -- Alle Indikatoren nehmen das candles-Objekt direkt.
  -- Rust weiß welche Price-Series intern gebraucht wird.
  -- Nur die aufgerufenen Indikatoren werden berechnet (ON DEMAND).
  -- ============================================================

  -- RSI – gibt f64 (0–100) zurück
  local rsi = indicators.rsi(candles, 14)

  -- SMA / EMA – geben f64 zurück
  local sma_20 = indicators.sma(candles, 20)
  local sma_50 = indicators.sma(candles, 50)
  local ema_12 = indicators.ema(candles, 12)
  local ema_26 = indicators.ema(candles, 26)

  -- MACD – gibt Table zurück
  local macd = indicators.macd(candles, 12, 26, 9)
  -- macd.line, macd.signal, macd.histogram

  -- Bollinger Bands – gibt Table zurück
  local bb = indicators.bollinger(candles, 20, 2.0)
  -- bb.upper, bb.middle, bb.lower

  -- ATR – gibt f64 zurück (nutzt high/low/close intern)
  local atr = indicators.atr(candles, 14)

  -- Slope (lineare Regression) – gibt f64 zurück
  local slope = indicators.slope(candles, 5)
  -- Positiv = uptrend, Negativ = downtrend

  -- Custom Price-Series (z.B. Typical Price für CCI)
  local typical = {}
  for i = 1, #closes do
      typical[i] = (highs[i] + lows[i] + closes[i]) / 3
  end
  -- indicators.cci_series(typical, 20) -- für Custom-Series

  -- ============================================================
  -- CONTEXT (Portfolio-Info)
  -- ============================================================

  local balance = context.balance
  local equity  = context.equity
  local has_pos = context.position ~= nil

  if context.position then
    local pos   = context.position
    local side  = pos.side           -- "long" oder "short"
    local entry = pos.entry_price
    local size  = pos.size
    local pnl   = (current.close - entry) * size
  end

  -- ============================================================
  -- DECISION RETURN
  -- ============================================================

  -- Gültige Signale: "BUY", "SELL", "HOLD", "SHORT", "COVER"
  if rsi < 30 and sma_20 > sma_50 then
    return {
      signal      = "BUY",
      size        = 0.1,
      stop_loss   = current.close * 0.98,
      take_profit = current.close * 1.05,
      reason      = "RSI oversold + EMA cross"
    }
  elseif rsi > 70 then
    return { signal = "SELL", reason = "RSI overbought" }
  else
    return { signal = "HOLD" }
  end
end

return config
```

### Lua Bindings (Rust-Seite)

```rust
// lua-engine/src/bindings.rs

fn register_indicators(lua: &mlua::Lua) -> mlua::Result<()> {
    let globals = lua.globals();
    let indicators = lua.create_table()?;

    // Alle Indikatoren nehmen LuaCandles direkt – Rust extrahiert intern
    // die benötigte Price-Series (closes, high/low/close, etc.)
    indicators.set("rsi",      lua.create_function(lua_rsi)?)?;
    indicators.set("sma",      lua.create_function(lua_sma)?)?;
    indicators.set("ema",      lua.create_function(lua_ema)?)?;
    indicators.set("macd",     lua.create_function(lua_macd)?)?;
    indicators.set("bollinger",lua.create_function(lua_bollinger)?)?;
    indicators.set("atr",      lua.create_function(lua_atr)?)?;
    indicators.set("slope",    lua.create_function(lua_slope)?)?;

    globals.set("indicators", indicators)?;
    Ok(())
}

// lua-engine/src/candle_wrapper.rs

// Intern: Vec<Candle> chronologisch (älteste zuerst, Index 0 = älteste)
// Lua-seitig: 1-indexed, neueste zuerst (candles[1] = Vec[last])
impl mlua::UserData for LuaCandles {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        // 1-indexed Zugriff, neueste zuerst. Out-of-bounds → nil.
        methods.add_meta_method(MetaMethod::Index, |_, this, idx: usize| {
            if idx == 0 {
                return Err(mlua::Error::runtime("candles is 1-indexed (starts at 1)"));
            }
            let len = this.candles.len();
            match len.checked_sub(idx) {
                Some(i) => Ok(this.candles.get(i).cloned()
                    .map(mlua::Value::UserData)
                    .unwrap_or(mlua::Value::Nil)),
                None => Ok(mlua::Value::Nil),   // out-of-bounds → nil, kein Fehler
            }
        });

        // __len: #candles gibt Anzahl verfügbarer Candles zurück
        methods.add_meta_method(MetaMethod::Len, |_, this, ()| {
            Ok(this.candles.len())
        });

        // Hilfsmethoden: neueste zuerst – konsistent mit candles[1]
        // ⚠ Jeder Call allokiert einen neuen Vec<f64>. Ergebnis einmal in lokale Variable
        //   speichern, nicht mehrfach in der gleichen on_tick Ausführung aufrufen.
        methods.add_method("closes",  |_, this, ()| {
            Ok(this.candles.iter().rev().map(|c| c.close).collect::<Vec<f64>>())
        });
        methods.add_method("opens",   |_, this, ()| {
            Ok(this.candles.iter().rev().map(|c| c.open).collect::<Vec<f64>>())
        });
        methods.add_method("highs",   |_, this, ()| {
            Ok(this.candles.iter().rev().map(|c| c.high).collect::<Vec<f64>>())
        });
        methods.add_method("lows",    |_, this, ()| {
            Ok(this.candles.iter().rev().map(|c| c.low).collect::<Vec<f64>>())
        });
        methods.add_method("volumes", |_, this, ()| {
            Ok(this.candles.iter().rev().map(|c| c.volume).collect::<Vec<f64>>())
        });
        // Indikatoren nehmen candles direkt – Rust dreht intern auf älteste-zuerst um
    }
}
```

### Lua Error Handling

Der Lua-Runner (in `lua-engine/src/vm.rs`) fängt alle Fehler aus `on_tick` ab und definiert klares Verhalten:

| Situation | Verhalten |
|---|---|
| `on_tick` wirft Lua-Exception | Fehler wird geloggt, Run wird als Fehler beendet |
| `on_tick` gibt `nil` zurück | Fehler: `"on_tick must return a table"` |
| Return-Table fehlt `signal`-Key | Fehler: `"missing key: signal"` |
| `signal` hat ungültigen Wert | Fehler: `"unknown signal: XYZ"` |
| `size` fehlt bei BUY/SELL | Default: `1.0` (100% des verfügbaren Kapitals) |
| `stop_loss` / `take_profit` fehlen | Default: `nil` (keine automatische Absicherung) |

**Strategie-Fehler stoppen den Run.** Es wird kein Trade ausgeführt, kein HOLD angenommen. Fehler müssen im Lua-Code behoben werden.

---

## Indicator API (Rust)

```rust
// indicators/src/lib.rs

// Indikatoren sind aktuell einfache Funktionen.
// Für Custom-Indikatoren (z.B. eigene Berechnungen aus Lua heraus nutzbar)
// kann ein Indicator-Trait eingeführt werden:
//
//   pub trait Indicator {
//       fn update(&mut self, candles: &[Candle]) -> Option<f64>;
//       fn name(&self) -> &str;
//   }
//
// Damit lassen sich Custom-Indikatoren in Rust implementieren und
// genauso wie built-in Indikatoren in Lua registrieren.
// Input ist immer &[f64] oder &[Candle] (chronologisch, älteste zuerst).

// Alle Indikatoren geben Option<T> zurück:
// None wenn zu wenige Daten vorhanden (closes.len() < period)
// mlua mappt None → nil in Lua
pub fn rsi(closes: &[f64], period: usize) -> Option<f64> { ... }
pub fn sma(closes: &[f64], period: usize) -> Option<f64> { ... }
pub fn ema(closes: &[f64], period: usize) -> Option<f64> { ... }

pub struct MacdResult   { pub line: f64, pub signal: f64, pub histogram: f64 }
pub struct BbResult     { pub upper: f64, pub middle: f64, pub lower: f64 }

pub fn macd(closes: &[f64], fast: usize, slow: usize, signal: usize) -> Option<MacdResult> { ... }
pub fn bollinger(closes: &[f64], period: usize, std_dev: f64) -> Option<BbResult> { ... }
pub fn atr(candles: &[Candle], period: usize) -> Option<f64> { ... }
pub fn slope(closes: &[f64], period: usize) -> Option<f64> { ... }
```

### Performance: Incremental Indicator State

Die Lua API ändert sich nicht. Der Unterschied liegt darin wie lange die Engine-Instanz lebt:

```
strategy-runner:  Engine lebt für einen einzigen on_tick-Aufruf
                  → Indikatoren von scratch (O(period)) – einmalig, kein Problem

backtester:       Engine lebt für alle N Candles
                  → Candle 1: von scratch (O(period)) → State im Cache speichern
                  → Candle 2–N: inkrementeller Update (O(1)) pro Indikator-Aufruf
```

```
Backtest 1000 Candles, RSI(14) + EMA(50) + MACD(26):
  Ohne Cache:  1000 × (14 + 50 + 26) = 90.000 Operationen
  Mit Cache:   (14 + 50 + 26) + 999 × O(1) ≈ 90 + 999 = ~1.100 Operationen
```

Der Cache lebt in `lua-engine/src/indicator_cache.rs` und ist transparent für Lua –
keine Änderung an Strategie-Dateien nötig.

### Geplante Indikatoren

**Trend**

| Indikator     | Input                       | Output      | Beschreibung                          |
|---------------|-----------------------------|-------------|---------------------------------------|
| SMA           | closes, period              | f64         | Simple Moving Average                 |
| EMA           | closes, period              | f64         | Exponential Moving Average            |
| DEMA          | closes, period              | f64         | Double EMA (weniger Lag als EMA)      |
| TEMA          | closes, period              | f64         | Triple EMA (noch weniger Lag)         |
| MACD          | closes, fast, slow, signal  | MacdResult  | Trend + Momentum ({line, signal, histogram}) |
| Parabolic SAR | candles, step, max          | f64         | Stop-and-Reverse Trendfolge           |
| ADX           | candles, period             | f64 (0-100) | Trendstärke (ohne Richtung)           |
| Ichimoku      | candles                     | IchimokuResult | Cloud, Spannen, Chikou ({tenkan, kijun, span_a, span_b, chikou}) |

**Momentum**

| Indikator    | Input               | Output      | Beschreibung                          |
|--------------|---------------------|-------------|---------------------------------------|
| RSI          | closes, period      | f64 (0-100) | Overbought/Oversold                   |
| Stochastic   | candles, period     | {k, d}      | Stochastic Oscillator                 |
| CCI          | candles, period     | f64         | Commodity Channel Index               |
| Williams %R  | candles, period     | f64 (0 bis -100) | Overbought/Oversold (invertiert) |
| ROC          | closes, period      | f64         | Rate of Change (Preisveränderung %)   |

**Volatilität**

| Indikator        | Input                   | Output    | Beschreibung                      |
|------------------|-------------------------|-----------|-----------------------------------|
| Bollinger Bands  | closes, period, std_dev | BbResult  | Volatility Bands ({upper, middle, lower}) |
| ATR              | candles, period         | f64       | Average True Range                |
| Keltner Channels | candles, period, mult   | {upper, middle, lower} | ATR-basierte Bänder  |

**Volumen**

| Indikator      | Input    | Output | Beschreibung                              |
|----------------|----------|--------|-------------------------------------------|
| OBV            | candles  | f64    | On-Balance Volume (Volumen-Trendindikator)|
| VWAP           | candles  | f64    | Volume Weighted Average Price             |
| Volume Profile | candles  | Vec<{price, volume}> | Volumen je Preiszone         |
| MFI            | candles, period | f64 (0-100) | Money Flow Index (Volume-RSI)  |

**Support/Resistance**

| Indikator              | Input           | Output           | Beschreibung                    |
|------------------------|-----------------|------------------|---------------------------------|
| Pivot Points           | candles (last)  | {pp, r1, r2, r3, s1, s2, s3} | Klassische Pivot-Level |
| Fibonacci Retracements | high, low       | Vec<f64>         | 23.6%, 38.2%, 50%, 61.8%, 78.6% |

**Sonstige**

| Indikator | Input          | Output | Beschreibung                       |
|-----------|----------------|--------|------------------------------------|
| Slope     | closes, period | f64    | Lineare Regressions-Steigung über N Perioden |

---

## SpacetimeDB Schema

Das Schema lebt im `spacetimedb-module` Crate und wird auf den SpacetimeDB Server deployed.
Client-Binaries (data-fetcher, strategy-runner, etc.) interagieren über `spacetimedb-sdk` Bindings.

### 6 Tables – Getrennt nach Zweck

#### Core Tables (laufender Bot)

```rust
// spacetimedb-module/src/lib.rs

/// OHLCV Candle Data – Historie vom Provider
#[spacetimedb::table(name = candles, public)]
pub struct CandleRecord {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    #[unique]
    pub canonical_id: String,       // "{symbol}_{timeframe}_{timestamp}" – echte Deduplication
    pub timestamp: i64,             // Unix ms
    pub symbol: String,             // "AAPL", "BTC-USD"
    pub open: f64,
    pub high: f64,
    pub low: f64,
    pub close: f64,
    pub volume: f64,                // f64: Aktien (ganzzahlig) + Crypto (fraktional)
    pub timeframe: String,          // "1m", "5m", "1h", "1d"
    pub provider: String,           // "yahoo", "twelve_data", "broker"
}

/// Offene Positionen – was der Bot aktuell hält
#[spacetimedb::table(name = positions, public)]
pub struct PositionRecord {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub strategy: String,           // "sma_cross"
    pub symbol: String,
    pub side: String,               // "long" oder "short"
    pub entry_price: f64,
    pub size: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub entry_time: i64,
}

/// Prod-Trades (Paper Trading)
#[spacetimedb::table(name = trades, public)]
pub struct TradeRecord {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub timestamp: i64,
    pub strategy: String,
    pub symbol: String,
    pub side: String,               // "long" oder "short"
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub size: f64,
    pub pnl: Option<f64>,
    pub status: String,             // "open", "closed", "stopped"
    pub entry_reason: Option<String>,
    pub exit_reason: Option<String>,
}
```

#### Backtest Tables (historisch, session-basiert)

```rust
/// Backtest Sessions – Index aller Backtest-Läufe
#[spacetimedb::table(name = backtest_sessions, public)]
pub struct BacktestSession {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub strategy: String,
    pub symbol: String,
    pub start_time: i64,
    pub end_time: i64,
    pub created_at: i64,
    pub initial_balance: f64,
    pub final_balance: f64,
    pub total_return: f64,
    pub total_trades: u32,
}

/// Backtest-Signale – warum wurde entschieden?
#[spacetimedb::table(name = backtest_signals, public)]
pub struct BacktestSignalRecord {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub session_id: u64,
    pub timestamp: i64,
    pub strategy: String,
    pub symbol: String,
    pub signal: String,             // "BUY", "SELL", "HOLD", "SHORT", "COVER"
    pub price: f64,
    pub stop_loss: Option<f64>,
    pub take_profit: Option<f64>,
    pub reason: Option<String>,
}

/// Backtest-Trades – simulierte Trades
#[spacetimedb::table(name = backtest_trades, public)]
pub struct BacktestTradeRecord {
    #[primary_key]
    #[auto_inc]
    pub id: u64,
    pub session_id: u64,
    pub strategy: String,
    pub symbol: String,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: Option<f64>,
    pub size: f64,
    pub pnl: Option<f64>,
    pub entry_time: i64,
    pub exit_time: Option<i64>,
    pub exit_reason: Option<String>,
}
```

#### Zwei Backtest-Workflows – explizit getrennt

**CLI-Backtest** (`backtester` Binary):
- Schreibt Ergebnisse persistent in DB (`backtest_sessions`, `backtest_signals`, `backtest_trades`)
- Zweck: langfristige Auswertung, Strategie-Vergleiche über Zeit, Reproduzierbarkeit

**UI-Backtest** (in-memory):
- Liest nur Candles aus DB, berechnet alles im Speicher
- Persistiert Backtest-Ergebnisse nicht in DB
- Zweck: interaktives Arbeiten – Strategie anpassen, sofort Ergebnis sehen, wiederholen
- Nutzt dieselben Crates (`indicators`, `lua-engine`) wie das `backtester` Binary

UI-Backtest und CLI-Backtest sind unabhängige Pfade auf denselben Candle-Daten. Die UI kann CLI-Ergebnisse aus `backtest_sessions` für Vergleiche lesen, führt eigene Berechnungen aber in-memory aus.

---

## Binary-Kommunikation

**Jeder Run ist ein eigenständiger Prozess.** Alle Binaries kommunizieren ausschließlich über SpacetimeDB.

```
┌─────────────────────────────────────────────────────────────┐
│  runner                                                     │
│  Cron ──> data-fetcher (wartet) ──> strategy-runner ──> Exit│
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  data-fetcher                                               │
│  Provider API ──> SpacetimeDB (candles) ──> Exit            │
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  strategy-runner                                            │
│  DB (candles) ──> Lua (ON DEMAND indicators) ──> DB (trades)│
└─────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────┐
│  backtester                                                 │
│  DB (candles) ──> Replay ──> Lua ──> backtest_* Tables      │
└─────────────────────────────────────────────────────────────┘
```

---

## Cron Integration

### runner (alle 5 Minuten)

```bash
# crontab -e
*/5 * * * * /path/to/trading-bot/target/release/runner \
  --strategy strategies/sma_cross.lua \
  --symbols AAPL,BTC-USD \
  --provider yahoo \
  --interval 5m
```

### Manuelle Nutzung

```bash
# Nur Daten holen
data-fetcher --provider yahoo --symbols TSLA,NVDA --interval 1h

# Nur eine Strategie testen
strategy-runner --strategy my_new_strategy.lua --symbol BTC-USD

# Backtest
backtester --strategy sma_cross.lua --symbol AAPL --start 2020-01-01 --end 2024-01-01
```

---

## Backtesting Workflow

```bash
./target/release/backtester \
  --strategy strategies/sma_cross.lua \
  --symbol AAPL \
  --start 2020-01-01 \
  --end 2024-01-01 \
  --initial-balance 10000 \
  --output report.json

# Output:
# {
#   "session_id": 42,
#   "strategy": "sma_cross",
#   "symbol": "AAPL",
#   "period": "2020-01-01 to 2024-01-01",
#   "total_return": "45.2%",
#   "sharpe_ratio": 1.23,
#   "max_drawdown": "-12.5%",
#   "win_rate": "58.3%",
#   "total_trades": 142,
#   "avg_trade_duration": "3.2 days"
# }
```

---

## Zukünftige Erweiterungen

### UI (GPUI)
- Strategy Editor: Lua-Dateien direkt in der UI bearbeiten und verwalten
- Strategie auswählen und direkt backtesten (in-memory)
- Live-Signale sehen
- Backtest-Ergebnisse visualisieren (alle Sessions aus backtest_sessions)
- Portfolio-Übersicht

### Live Trading
- Broker API Anbindung (Interactive Brokers, Alpaca, etc.)
- Order Execution
- Risk Management (Position Sizing, Max Drawdown Limits)
- Broker als Daten-Provider nutzen (data-fetcher --provider broker)

### Erweiterte Features
- Multi-Strategie Support (mehrere Strategien parallel)
- Strategy Ranking (beste Strategie automatisch wählen)
- Alert System (Email, Discord, Telegram)
- Walk-Forward Optimization
- Weitere Provider (Polygon.io, Alpha Vantage, etc.)

---

## Dependencies

### Workspace Dependencies (Cargo.toml)

```toml
[workspace]
members = [
    "shared",
    "db-layer",
    "spacetimedb-module",
    "indicators",
    "lua-engine",
    "runner",
    "data-fetcher",
    "strategy-runner",
    "backtester",
]

[workspace.dependencies]
# Core
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
chrono = { version = "0.4", features = ["serde"] }
thiserror = "2.0"
anyhow = "1.0"

# SpacetimeDB
spacetimedb = "2.0"             # nur in spacetimedb-module
spacetimedb-sdk = "1.12"        # in db-layer und allen Binaries

# Lua
mlua = { version = "0.11", features = ["lua54", "vendored"] }

# HTTP / API
reqwest = { version = "0.13", features = ["json"] }
tokio = { version = "1", features = ["full"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"
```

---

## Design Principles

1. **Single Responsibility:** Jedes Crate hat eine klare, isolierte Aufgabe
2. **Keine Überlappung:** db-layer kennt keine Indikatoren, indicators kennt keine Lua
3. **Shared Types:** Alle gemeinsamen Typen leben in `shared/`
4. **SpacetimeDB als Hub:** Alles läuft über die DB – kein State im Memory
5. **Getrennte Tables:** Core, Backtest – keine Datenvermischung
6. **Session-basierte Backtests:** Jeder Backtest bekommt eine auto-incrementierte `session_id`. Ergebnisse sind reproduzierbar und vergleichbar über Runs.
7. **Lazy Candle Loading, keine Konfiguration:** `candles` fetcht aus DB exakt was ein Indikator braucht – kein min_bars, kein max_candles, kein Buffer. Fehlen Daten → Abbruch mit klarer Meldung welcher Indikator welche Daten vermisst.
8. **Lua ruft Indikatoren ON DEMAND:** Lua entscheidet was sie braucht, Rust berechnet nur das
9. **Candle Access in Lua:** 1-indexed, neueste zuerst – intuitiv für Strategy-Entwickler
10. **Provider-Abstraktion:** Datenquelle austauschbar (Yahoo → Twelve Data → Broker)
11. **Paper Trading First:** Kein echtes Geld bis alles gründlich getestet ist
12. **Ephemeral Runs:** Jedes Binary startet, arbeitet, beendet sich – kein dauerhafter Service (außer SpacetimeDB Server)
13. **Kein paralleler Cron-Run:** runner legt beim Start ein Lock-File an (`/tmp/runner-{strategy}-{symbol}.lock`). Zweiter gleichzeitiger Start → sofortiger Exit. `--timeout` killt den laufenden Child-Prozess wenn er zu lange braucht und gibt das Lock frei.
