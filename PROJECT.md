# TradingBot – Projektdokument

## Vision

Ein automatisierter Trading Bot in **Rust**, der zunächst mit fiktivem Kapital (Paper Trading) handelt, reale Kosten und Steuern simuliert, und über eine **Terminal UI (TUI)** beobachtbar ist. Wenn Paper Trading nachweislich funktioniert, folgt die Anbindung an einen echten Broker.

---

## Technologie-Stack

| Komponente | Technologie | Begründung |
|---|---|---|
| Sprache | Rust | Performance, SpacetimeDB-native, Typsicherheit |
| Datenbank | SpacetimeDB | Reaktive Subscriptions, Module in Rust, Real-Time TUI-Updates |
| TUI | Ratatui + crossterm | De-facto Standard für Rust TUIs, battle-tested |
| Async Runtime | Tokio | Standard in der Rust-Async-Welt |
| HTTP Client | reqwest | Marktdaten-Fetching |
| Marktdaten | yfinance (via HTTP) | Historische + aktuelle Kursdaten |
| Geldbeträge | rust_decimal | Niemals f64 für Geldbeträge |
| Datum/Zeit | chrono | |
| Fehlerbehandlung | anyhow / thiserror | |

---

## Architektur

```
┌─────────────────────────────────────────────────┐
│  Cron Job (system cron oder eigener Scheduler)  │
│  ruft SpacetimeDB-Reducer auf                   │
└──────────────────────┬──────────────────────────┘
                       │
┌──────────────────────▼──────────────────────────┐
│  SpacetimeDB Module (Rust)                      │
│                                                 │
│  Tabellen:                                      │
│  ├── Eine Tabelle pro Asset (z.B. aapl_candles) │
│  ├── portfolio                                  │
│  ├── trades                                     │
│  └── tax_events                                 │
│                                                 │
│  Reducer:                                       │
│  ├── update_market_data(asset)                  │
│  ├── run_strategy(asset)                        │
│  └── execute_paper_trade(order)                 │
└──────────────────────┬──────────────────────────┘
                       │  WebSocket Subscription
┌──────────────────────▼──────────────────────────┐
│  TUI (Rust + Ratatui)                           │
│  ├── Portfolio Übersicht                        │
│  ├── Trade History                              │
│  ├── P&L Dashboard (brutto / netto / Steuer)    │
│  ├── Asset-Charts (Kursverlauf)                 │
│  └── Live-Updates ohne Polling                  │
└─────────────────────────────────────────────────┘
```

---

## Datenbankdesign

### Konzept: Eine Tabelle pro Asset

Jedes beobachtete oder gehandelte Asset bekommt eine eigene Tabelle mit OHLCV-Daten (Open, High, Low, Close, Volume). Das erlaubt einfache Abfragen und klare Trennung der Historien.

```
aapl_candles    → Apple Inc.
msft_candles    → Microsoft
spy_candles     → S&P 500 ETF
rut_candles     → Russell 2000
...
```

### Candle-Tabelle (Schema-Entwurf)

```rust
#[spacetimedb(table)]
pub struct Candle {
    #[primarykey]
    pub timestamp: i64,      // Unix Timestamp (Sekunden)
    pub open:      i64,      // Preis in Cent (rust_decimal intern)
    pub high:      i64,
    pub low:       i64,
    pub close:     i64,
    pub volume:    i64,
    pub interval:  String,   // "1d", "1h", "15m", ...
}
```

### Weitere Tabellen

```rust
// Aktueller Portfolio-Stand
pub struct Portfolio {
    pub asset:        String,
    pub quantity:     i64,       // in kleinster Einheit
    pub avg_buy_price: i64,      // in Cent
    pub last_updated: i64,
}

// Jeder Paper Trade
pub struct Trade {
    pub id:         u64,
    pub asset:      String,
    pub side:       String,      // "buy" | "sell"
    pub quantity:   i64,
    pub price:      i64,         // in Cent
    pub fee:        i64,         // Broker-Kosten in Cent
    pub timestamp:  i64,
    pub strategy:   String,
}

// Steuer-relevante Events
pub struct TaxEvent {
    pub trade_id:       u64,
    pub gain_loss:      i64,     // in Cent, negativ = Verlust
    pub tax_amount:     i64,     // berechnete Steuer
    pub is_exempt:      bool,    // innerhalb Freistellungsauftrag?
    pub timestamp:      i64,
}
```

---

## Paper Trading Engine

### Ziel

Reales Trading-Verhalten simulieren, ohne echtes Geld zu riskieren. Die Simulation soll so realistisch wie möglich sein.

### Simulierte Kosten

| Kostenart | Beschreibung | Typischer Wert |
|---|---|---|
| Spread | Differenz Bid/Ask | variiert je Asset |
| Provision | Pro Order oder prozentual | z.B. 0,1% oder 1€ flat |
| Börsenplatzgebühr | Je nach Handelsplatz | z.B. 0,50€ |

### Steuer-Simulation (Deutschland)

| Steuer | Satz | Hinweis |
|---|---|---|
| Abgeltungssteuer | 25% auf Kursgewinne | |
| Solidaritätszuschlag | 5,5% auf die Steuer | = 26,375% gesamt |
| Kirchensteuer | 8–9% (optional) | konfigurierbar |
| Freistellungsauftrag | 1.000€/Jahr (seit 2023) | Gewinne bis 1.000€ steuerfrei |
| Verlustverrechnung | Verluste werden vorgetragen | und mit späteren Gewinnen verrechnet |

### Kontext-Parameter (konfigurierbar)

```toml
[paper_trading]
starting_capital = 10_000_00   # in Cent (= 10.000 €)
currency = "EUR"

[costs]
commission_type = "flat"       # "flat" | "percent"
commission_amount = 100        # in Cent (= 1,00 €)
spread_simulation = true

[tax]
country = "DE"
freistellungsauftrag = 100_000 # in Cent (= 1.000 €)
kirchensteuer = false
```

---

## Strategie-Interface

### Konzept

`Strategy` ist ein **Rust Trait** — ein Interface, hinter das man beliebige Kalkulationslogik stecken kann. Der Bot kennt nur den Trait, nicht die konkrete Implementierung. Neue Strategien können hinzugefügt werden ohne den restlichen Code zu ändern.

```rust
pub trait Strategy {
    fn name(&self) -> &str;
    fn required_history(&self) -> usize;   // Wie viele Candles werden gebraucht?
    fn signal(&self, candles: &[Candle]) -> Signal;  // <-- hier passiert die Berechnung
}

pub enum Signal {
    Buy,
    Sell,
    Hold,
}
```

### Die entscheidende Funktion: `signal()`

`signal()` bekommt die Candle-Historie als Slice rein und gibt ein Trading-Signal zurück. Hier steckt die gesamte Strategie-Logik. Der Bot ruft sie auf und kümmert sich um die Ausführung.

### Wie der Bot das nutzt

```rust
// Zur Laufzeit: beliebige Strategie einsteckbar
let strategy: Box<dyn Strategy> = Box::new(SmaCrossover {
    short_period: 10,
    long_period: 50,
});

let candles = db.get_candles("aapl", strategy.required_history());
let signal  = strategy.signal(&candles);

match signal {
    Signal::Buy  => execute_paper_trade(Side::Buy, ...),
    Signal::Sell => execute_paper_trade(Side::Sell, ...),
    Signal::Hold => {},
}
```

Später einfach austauschen:
```rust
// Box::new(SmaCrossover { ... })
// Box::new(RsiStrategy { period: 14, oversold: 30, overbought: 70 })
// Box::new(BollingerBands { period: 20, std_dev: 2.0 })
```

### Erste Strategie: SMA Crossover

```rust
pub struct SmaCrossover {
    pub short_period: usize,  // z.B. 10 Tage
    pub long_period:  usize,  // z.B. 50 Tage
}

impl Strategy for SmaCrossover {
    fn name(&self) -> &str { "SMA Crossover" }

    fn required_history(&self) -> usize {
        self.long_period + 1  // +1 für den Vortagsvergleich
    }

    fn signal(&self, candles: &[Candle]) -> Signal {
        let short_sma = average(&candles[..self.short_period]);
        let long_sma  = average(&candles[..self.long_period]);

        // Vortageswerte zum Erkennen ob Crossover gerade passiert ist
        let prev_short = average(&candles[1..=self.short_period]);
        let prev_long  = average(&candles[1..=self.long_period]);

        if prev_short <= prev_long && short_sma > long_sma {
            Signal::Buy   // kurzer SMA kreuzt langem SMA von unten → kaufen
        } else if prev_short >= prev_long && short_sma < long_sma {
            Signal::Sell  // kurzer SMA kreuzt langem SMA von oben → verkaufen
        } else {
            Signal::Hold
        }
    }
}
```

**Logik:** Kurzfristiger Durchschnitt (z.B. 10 Tage) kreuzt langfristigen (z.B. 50 Tage) → Trendwechsel-Signal.

### Geplante weitere Strategien

| Strategie | Beschreibung |
|---|---|
| SMA Crossover | Erster Schritt, einfach & verständlich |
| RSI | Relative Strength Index, Überkauft/Überverkauft |
| Bollinger Bands | Volatilitätsbasiert |
| MACD | Moving Average Convergence Divergence |

---

## TUI – geplante Ansichten

| Tab | Inhalt |
|---|---|
| Portfolio | Aktuelle Positionen, Gesamtwert, fiktives Cash |
| Trades | Trade-Historie, Kosten pro Trade |
| P&L | Gewinn/Verlust brutto, nach Kosten, nach Steuern |
| Assets | Watchlist, letzter Kurs, Candle-Chart (ASCII) |
| Strategie | Aktive Strategie, letzte Signale |
| Steuern | YTD Gewinne, verbrauchter Freistellungsauftrag, Steuerlast |

---

## Meilensteine

### Phase 1 – Fundament
- [ ] Rust-Projekt + Workspace Setup
- [ ] SpacetimeDB Schema (Tabellen & Reducer)
- [ ] Historische Daten via yfinance fetchen und speichern
- [ ] Erste Asset-Tabellen befüllen

### Phase 2 – Bot-Logik
- [ ] Strategy-Trait definieren
- [ ] SMA Crossover Strategie implementieren
- [ ] Paper Trading Engine (inkl. Kosten)
- [ ] Steuer-Simulation

### Phase 3 – TUI
- [ ] Ratatui Setup
- [ ] Portfolio Dashboard
- [ ] Trade History View
- [ ] P&L Ansicht mit Steuerübersicht

### Phase 4 – Broker-Anbindung (offen)
- [ ] Broker auswählen (Interactive Brokers, Alpaca, TradeRepublic?)
- [ ] API-Integration
- [ ] Echtes Order-Management

---

## Offene Entscheidungen

- [ ] **Broker**: Noch nicht entschieden – beeinflusst Märkte und API-Format
- [ ] **Märkte**: Folgt nach Broker-Entscheidung
- [ ] **Candle-Intervall**: Täglich (1d) für den Anfang, später kürzer?
- [ ] **Strategie-Bibliothek**: Mehrere Strategien parallel testen?
- [ ] **Backtesting**: Separate Engine zum Testen auf historischen Daten?

---

## Notizen

- Geldbeträge immer in **Cent als Integer** speichern, niemals als Float
- SpacetimeDB-Module laufen in Rust → kein Sprach-Mismatch
- Cron-Job ruft Reducer auf, TUI subscribed auf Tabellen → keine Polling-Logik nötig
