# To-Issues-Entwurf: Tokio-basierte In-Memory Trading-Runtime mit Rhai, Live/Paper/Backtest

Dieses Dokument zerlegt die geplante Architektur in unabhängig greifbare Vertical Slices für ein zunächst rein **in-memory** laufendes System auf Basis von Tokio, einer Rhai-basierten Strategy-Runtime und drei Betriebsmodi: Live, Paper und Backtest. [github](https://github.com/tokio-rs/tokio)

## Zielbild

Die Runtime soll Rhai-Strategiedateien verarbeiten und pro neuem Datenpunkt einen Tick ausführen. Live und Paper teilen sich denselben Kernpfad; der Unterschied liegt primär im Execution- und Portfolio-Profil, während Backtest historische Daten aus der Datenbank lädt, optional mutiert, mehrere Runtime-Instanzen parallel startet und die Ergebnisse am Ende als Report persistiert. [tokio](https://tokio.rs)

Die Architektur muss deshalb weniger wie ein frei fließender Event-Bus und mehr wie ein **deterministischer Tick-Workflow** funktionieren. Tokio liefert dafür Runtime, Tasks, Timer und Scheduling-Bausteine, aber die eigentliche Domänen-Semantik sollte als klarer Ablauf pro Tick modelliert werden. [github](https://github.com/tokio-rs/tokio)

## Empfohlene Architekturform

Für deinen Fall passt am besten eine **EDA-inspirierte Tick-Pipeline mit explizitem Scheduling** statt ein komplett offener Event-Bus. Tokio eignet sich für asynchrone I/O, Timer, Task-Ausführung und Koordination, während die eigentliche Strategieausführung pro Tick in einer festen Reihenfolge laufen sollte. [github](https://github.com/tokio-rs/tokio)

Ein sinnvoller Tick pro Datenpunkt sieht so aus:

1. Neue Marktdaten werden verfügbar.
2. Ein Tick-Trigger für genau diesen Datenpunkt wird erzeugt.
3. Die Runtime lädt den benötigten Kontextzustand.
4. Die Rhai-Strategie wird mit Tick-Kontext ausgeführt.
5. Orders, Signals und State-Änderungen werden gesammelt.
6. Das passende Execution-Profil (Paper oder Live) verarbeitet die Outputs.
7. Ergebnisse, Logs und Metriken werden persistiert.

Das ist im Kern ein Scheduler mit Checkliste pro Tick, nicht primär ein chaotischer Publish/Subscribe-Bus. Das passt auch zu deinem Wunsch, dass „alles klar nacheinander ablaufen wird“. [github](https://github.com/tokio-rs/tokio)

## Vorgeschlagene Vertical Slices

1. **Title**: In-memory tick bus and deterministic runtime loop
   **Type**: AFK
   **Blocked by**: None
   **User stories covered**: Als Entwickler möchte pro neuem Datenpunkt genau einen Tick durch eine feste Runtime-Pipeline schicken, damit die Ausführung deterministisch und testbar bleibt.

2. **Title**: Rhai strategy host with file-based script loading
   **Type**: AFK
   **Blocked by**: Slice 1
   **User stories covered**: Als Entwickler möchte Strategien als Rhai-Dateien laden und in der Runtime ausführen, damit Strategielogik vom Engine-Core getrennt bleibt. [rhai](https://rhai.rs)

3. **Title**: Tick context contract for market data, indicators, portfolio and clock
   **Type**: AFK
   **Blocked by**: Slice 2
   **User stories covered**: Als Strategieautor möchte ich innerhalb eines Ticks einen stabilen, wohldefinierten Kontext sehen, damit Rhai-Skripte unabhängig vom Betriebsmodus bleiben. [perplexity](https://www.perplexity.ai/search/c37da4e2-23cb-49d8-839d-f9f1b96d68cc)

4. **Title**: Live data ingress to database to tick trigger flow
   **Type**: AFK
   **Blocked by**: Slice 1
   **User stories covered**: Als Betreiber möchte ich zyklisch Datenquellen abrufen und nach erfolgreicher Speicherung pro neuem Datenpunkt einen Tick anstoßen, damit das Live-System datengetrieben reagiert.

5. **Title**: Unified execution adapter with paper and live profiles
   **Type**: AFK
   **Blocked by**: Slice 3
   **User stories covered**: Als Entwickler möchte denselben Runtime-Kern sowohl mit fiktivem als auch mit echtem Execution-Profil nutzen, damit Paper und Live nur an den Außenkanten variieren.

6. **Title**: End-to-end paper trading slice on the shared runtime
   **Type**: AFK
   **Blocked by**: Slice 4, Slice 5
   **User stories covered**: Als Nutzer möchte ich ein vollständiges Paper-System haben, das echte Daten verarbeitet, Strategien ausführt und simulierte Orders sowie PnL speichert.

7. **Title**: End-to-end live trading slice with provider-backed execution
   **Type**: HITL
   **Blocked by**: Slice 4, Slice 5
   **User stories covered**: Als Nutzer möchte ich denselben Tick-Pfad gegen einen echten Provider ausführen, damit Orders live platziert und Kontostände synchronisiert werden können.

8. **Title**: Historical replay runner for backtest mode
   **Type**: AFK
   **Blocked by**: Slice 3
   **User stories covered**: Als Nutzer möchte ich historische Daten aus der Datenbank sequenziell als Tick-Strom abspielen, damit Strategien offline mit demselben Kernpfad geprüft werden können.

9. **Title**: Dataset mutation hooks for scenario generation
   **Type**: AFK
   **Blocked by**: Slice 8
   **User stories covered**: Als Forscher möchte ich historische Daten vor oder während eines Backtests kontrolliert mutieren können, damit Szenario- und Robustheitstests möglich werden.

10. **Title**: Parallel backtest runtime pool for Monte Carlo experiments
    **Type**: AFK
    **Blocked by**: Slice 8, Slice 9
    **User stories covered**: Als Forscher möchte ich mehrere Backtest-Runtimes parallel ausführen, damit Monte-Carlo-Experimente schneller und reproduzierbar laufen. [cjwebb](https://cjwebb.com/parallel-monte-carlo-rust/)

11. **Title**: Result aggregation and persisted backtest report
    **Type**: AFK
    **Blocked by**: Slice 10
    **User stories covered**: Als Nutzer möchte ich nach einem Backtest- oder Monte-Carlo-Lauf einen gespeicherten Report mit Kennzahlen und Vergleichswerten erhalten.

12. **Title**: Tick phase scheduler with explicit step checklist
    **Type**: AFK
    **Blocked by**: Slice 1, Slice 3
    **User stories covered**: Als Entwickler möchte jeder Tick über feste Phasen wie `load -> evaluate -> execute -> persist -> publish metrics` laufen, damit Reihenfolge und Fehlergrenzen explizit bleiben. [tokio](https://tokio.rs)

13. **Title**: Failure policy for tick isolation and retry semantics
    **Type**: HITL
    **Blocked by**: Slice 12
   **User stories covered**: Als Architekt möchte ich festlegen, wie Tick-Fehler behandelt werden, damit klar ist, ob ein Fehler einen einzelnen Tick, eine Runtime oder einen ganzen Lauf stoppt.

## Warum diese Form besser passt als ein freier Event-Bus

Dein beschriebenes System hat zwar Ereignisse, aber der fachliche Kern ist **Tick-orchestriert**: Ein Datenpunkt wird verfügbar, daraus wird genau ein Tick, und dieser Tick läuft in einer festen Reihenfolge durch klar definierte Schritte. Das ähnelt eher einem Scheduler oder einer Run-Checklist als einem vollständig entkoppelten Pub/Sub-System. [github](https://github.com/tokio-rs/tokio)

Ein kleiner in-memory Event-Bus ist trotzdem sinnvoll, aber eher an den Rändern:

- Daten-Ingress meldet „new data persisted“.
- Runtime meldet „tick completed“ oder „tick failed“.
- Reporting meldet „run finished“.

Die Tick-Logik selbst sollte dagegen **nicht** aus 20 frei verketteten Subscribern bestehen, sondern als feste Phase-Pipeline modelliert sein. Das hält die Semantik für Trading, Backtesting und Auditing deutlich sauberer. [tokio](https://tokio.rs)

## Empfohlene Modulgrenzen

Für deine vorhandene Richtung mit Domain-, Engine- und App-Layer passt diese Aufteilung besonders gut: [perplexity](https://www.perplexity.ai/search/c37da4e2-23cb-49d8-839d-f9f1b96d68cc)

| Modul | Verantwortung |
|---|---|
| `domain` | Candle/DataPoint, Tick, Signal, OrderIntent, Fill, PortfolioSnapshot, Report |
| `runtime` | Tick-Loop, Phase-Scheduler, in-memory event queue, runtime state |
| `strategy-rhai` | Laden, Kompilieren und Ausführen von Rhai-Dateien; Host-API für Indikatoren und Kontext  [rhai](https://rhai.rs/book/lib/rhai-fs.html) |
| `ingest-live` | Zyklisches Laden externer Daten und Persistenz-Trigger |
| `execution-paper` | Simulierte Ausführung, fiktives Portfolio, Slippage-/Fee-Modell |
| `execution-live` | Provider-Anbindung, Order-Mapping, Kontosynchronisierung |
| `backtest` | Historical replay, Mutation-Hooks, Run-Setup |
| `reporting` | Aggregation von Läufen und Speichern der Ergebnisreports |

## Mögliche Tick-Phasen

Eine erste, sehr brauchbare Checkliste pro Tick wäre:

1. `load_tick_input` — den neuen Datenpunkt und benötigte Historie laden.
2. `build_context` — Indikatoren, Portfolio, Clock und Metadaten zusammensetzen.
3. `run_strategy` — Rhai-Skript gegen den Tick-Kontext ausführen. [rhai](https://rhai.rs)
4. `collect_outputs` — Signale, OrderIntents, Diagnostik einsammeln.
5. `execute_orders` — Paper- oder Live-Adapter ausführen.
6. `persist_results` — Positionen, Fills, Logs, Tick-Outcome speichern.
7. `emit_metrics` — optionale Runtime-Metriken und Run-Status veröffentlichen.

Damit bekommst du EDA als internes Kommunikationsmuster, aber Scheduling als dominantes Ordnungsprinzip. Genau das scheint zu deinem Use-Case zu passen. [github](https://github.com/tokio-rs/tokio)

## Beispiel-Issues im Template-Stil

### Slice 1 — In-memory tick bus and deterministic runtime loop

## What to build

Build the first runnable runtime slice for in-memory tick processing. A new tick input enters the runtime, passes through a deterministic processing loop, and produces a verifiable outcome without external broker integration. [tokio](https://tokio.rs)

## Acceptance criteria

- [ ] A tick can be submitted into the runtime through an in-memory entry point.
- [ ] The runtime processes the tick through a fixed sequence of steps.
- [ ] A minimal example verifies deterministic ordering and output for one tick.

## Blocked by

None - can start immediately.

### Slice 2 — Rhai strategy host with file-based script loading

## What to build

Build a Rhai-backed strategy host that loads a strategy from a file and evaluates it inside the runtime. The strategy remains outside the engine core and uses a controlled host API for access to runtime data. [rhai](https://rhai.rs/book/lib/rhai-fs.html)

## Acceptance criteria

- [ ] A strategy file can be loaded from disk and compiled for runtime use.
- [ ] A runtime tick can invoke the loaded Rhai strategy.
- [ ] Errors in script loading or evaluation are surfaced as structured runtime failures.

## Blocked by

- Slice 1

### Slice 6 — End-to-end paper trading slice on the shared runtime

## What to build

Build the first end-to-end trading mode on top of the shared runtime using paper execution. Real or replayed market data should trigger ticks, strategies should produce order intents, and the paper execution profile should update a simulated portfolio and persist results.

## Acceptance criteria

- [ ] A persisted data point triggers a runtime tick.
- [ ] Strategy outputs are executed through the paper profile.
- [ ] Portfolio updates and run results are stored for later analysis.

## Blocked by

- Slice 4
- Slice 5

### Slice 10 — Parallel backtest runtime pool for Monte Carlo experiments

## What to build

Build a backtest runner that launches multiple independent runtime instances in parallel for Monte Carlo experiments. Each run should operate on its own scenario input and return structured results for aggregation. [docs](https://docs.rs/tokio/latest/tokio/task/)

## Acceptance criteria

- [ ] Multiple backtest runtime instances can run concurrently.
- [ ] Each run is isolated in state and output.
- [ ] Results from all runs can be collected for later reporting.

## Blocked by

- Slice 8
- Slice 9

## Fragen zur Freigabe

1. Willst du die Slices eher nach **Betriebsmodi** gliedern, also Paper zuerst vollständig, dann Live, dann Backtest, oder lieber nach **gemeinsamen Kernfähigkeiten** wie Runtime, Strategy Host, Scheduler und Execution-Profil?
2. Soll Monte Carlo wirklich auf Tokio-Tasks basieren, oder willst du für CPU-lastige Backtests eher bewusst Worker/Thread-Pools trennen, obwohl Tokio als Orchestrator bleibt? [cjwebb](https://cjwebb.com/parallel-monte-carlo-rust/)
