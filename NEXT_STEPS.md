# Next Steps & Implementation Plan

Dieses Dokument dient als Einstiegspunkt für die nächste Session. Wir haben die Architektur von einem ephemeren Cron-Setup (V1) zu einem professionellen, zustandsbehafteten Daemon-Setup (V2) umgebaut. 

Hier ist der genaue Fahrplan, wie wir die `ARCHITECTURE.md` nun Schritt für Schritt in Code umsetzen.

---

## Aktueller Stand (Zusammenfassung)
- **Die Entscheidung:** Wir nutzen SpacetimeDB **nur** als blitzschnellen Data-Lake. Keine Logik in der DB (kein WebAssembly).
- **Das Backend (`trading-daemon`):** Ein dauerhaft laufendes Rust-Programm (Tokio), das den Zustand (Indicator-Cache) im RAM behält ($O(1)$ Performance) und Live-Trading übernimmt.
- **Das Frontend (`trading-ui`):** Eine GPUI-Desktop-App, die In-Memory Backtests über historische Daten aus der SpacetimeDB ausführt, ohne die DB vollzuschreiben.
- **Die Engine (`engine`):** Ein geteiltes Crate für Daemon und UI, das die Lua/Rhai-Skripte ausführt und State-Management im TradingView/PineScript-Stil (`close[1]`, `ta.barssince`) ermöglicht.

---

## Implementierungs-Fahrplan

### Phase 1: Foundation & SpacetimeDB (Data Layer)
*Ziel: Ein sauberes Fundament und eine laufende Datenbank.*
1. **Cargo Workspace aufsetzen:** Die Ordnerstruktur aus der `ARCHITECTURE.md` anlegen (`shared/`, `engine/`, `db-layer/`, `trading-daemon/`, `trading-ui/`).
2. **SpacetimeDB Schema definieren:** 
   - Tabellen für `candles`, `live_positions` und `live_trades` in Rust definieren und auf den lokalen SpacetimeDB-Server deployen.
3. **DB-Layer Crate:** Die SpacetimeDB SDK/HTTP-Aufrufe (Lesen & Schreiben von Kerzen/Trades) in Rust kapseln.

### Phase 2: Die Trading Engine (Das Herzstück)
*Ziel: Die zustandsbehaftete Logik, die Indikatoren und Skripte berechnet.*
1. **Indicator State Machine:** Ein Trait/Struct-System für Indikatoren (EMA, SMA, CCI) bauen, das den letzten Wert speichert und bei einem `tick(new_candle)` ein $O(1)$ Update durchführt.
2. **Scripting Integration (Lua/Rhai):** Die gewählte Skriptsprache in Rust einbetten.
3. **PineScript-Semantik nachbauen:** Rust-Bindings schreiben, die dem Skript Zugriff auf historische Werte (`candles[1]`) und den In-Memory-State geben.
4. **Die "Warmup"-Logik:** Implementieren, wie die Engine beim Start das Skript liest (z.B. "braucht 60 Candles Vorlauf"), diese aus der DB zieht und den initialen State aufbaut.

### Phase 3: Der Trading-Daemon (Live Trading)
*Ziel: Der Hintergrund-Service, der Marktdaten holt und handelt.*
1. **Tokio Async Setup:** Eine Dauerschleife bauen, die z.B. alle 5, 15 oder 30 Minuten triggert.
2. **Data Fetcher:** Anbindung an eine API (z.B. Yahoo Finance, Twelve Data, oder Broker), um die neueste, *geschlossene* Kerze zu holen.
3. **Pipeline verknüpfen:** Neue Kerze -> SpacetimeDB Speicherung -> Übergabe an die Engine -> Skriptausführung -> Orderausführung bei Signal (Paper-Trading Dummy für den Anfang).

### Phase 4: GPUI & In-Memory Backtester
*Ziel: Das visuelle Frontend für schnelle Simulationen.*
1. **GPUI Setup:** Ein simples Fenster mit Chart-Canvas aufsetzen.
2. **Data Fetching:** Einen Button "Lade AAPL", der alle historischen Kerzen per HTTP aus SpacetimeDB in den UI-RAM lädt.
3. **Parallel Backtest Runner:** Eine Schleife, die die geladenen Kerzen durch eine frische Instanz der Engine (aus Phase 2) tickt und die Ergebnisse (PnL, Drawdown) in einem `Vec` sammelt.
4. **Visualisierung:** Die Trades und Signale auf dem Chart zeichnen.

---

## Startpunkt für morgen:
Wenn wir morgen anfangen, sollten wir exakt bei **Phase 1, Schritt 1 & 2** beginnen:
1. Den Cargo Workspace (Ordnerstruktur) initialisieren.
2. Das SpacetimeDB-Modul (Tabellen-Definitionen) schreiben, da dies die Datenstruktur (`shared` Crate) für alle anderen Komponenten diktiert.
