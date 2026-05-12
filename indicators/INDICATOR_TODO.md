# Indicator integration TODO

Audit-Stand: 2026-05-11

Diese Liste meint mit **"sauber integriert"**:
- im `indicators`-Crate implementiert
- bei Bedarf in Rhai über `indicators::...` exponiert
- in der Strategy-Doku dokumentiert
- konsistente API an der Rhai-Grenze
- sinnvolle Warmup-/Test-/Performance-Anbindung

## 1) Noch **nicht** in Rhai exponiert

Diese Funktionen existieren in Rust, sind aber für `.rhai`-Strategien aktuell nicht direkt nutzbar:

- `trend::ema::ema_series`
- `volatility::atr::atr_series`
- `trend::ichimoku::ichimoku_custom`

### Offene Fragen
- Wollen wir diese überhaupt als öffentliche Strategy-API?
- Falls ja: direkte Exponierung oder über eine bewusst andere, author-friendly API?

### Beschlossene Langfrist-Richtung für Ichimoku
Siehe auch Issue `#24`.

- `indicators::ichimoku(candles)` bleibt als kleine, direkte Strategy-API erhalten.
- Diese Form liefert die **current-known**, unmittelbar nutzbaren Top-Level-Werte:
  `tenkan`, `kijun`, `span_a`, `span_b`, `chikou`.
- Langfristig soll eine reichere Overload-Form hinzukommen:
  `indicators::ichimoku(candles, radius)`.
- Diese Overload soll **dieselben Top-Level-Werte** behalten und zusätzlich ein
  Feld `window` exponieren.
- Ziel-Shape für `window`:
  - `current`
  - `past`
  - `future_cloud`
  - `meta`
- Die reichere Fenster-Sicht soll additive Macht liefern, ohne die einfache
  bestehende API zu verschlechtern.
- Warmup-Semantik soll vor Implementierung explizit festgelegt werden; das
  Top-Level-Modell bleibt unabhängig davon weiterhin mit 52 Bars nutzbar.

---

## 2) In Rhai exponiert, aber API **noch mit Rest-Inkonsistenz**

Die meisten history-basierten Indikatoren unterstützen jetzt Offset-Overloads.

Bewusste Ausnahme:

- `fibonacci`

### Begründung
`fibonacci(candles, low, high)` hängt nicht von Candle-Historie ab, sondern nur
von den explizit übergebenen `low`/`high`-Werten. Ein Offset-Parameter wäre
formal möglich, aber semantisch irreführend.

### Offene Entscheidung
- so belassen
- oder `fibonacci` in der öffentlichen API irgendwann anders formen

### Langfristiges Zielbild für Fibonacci
`fibonacci` sollte langfristig **nicht nur** ein nackter Preislevel-Helfer bleiben,
sondern echte Swing-/Struktur-Logik hinter sich haben.

Festgehalten auch in:
- Code-Kommentaren bei `indicators/src/support_resistance/fibonacci.rs`
- `docs/adr/0001-fibonacci-as-anchored-structure-indicator.md`

Wahrscheinliche sinnvolle Zielrichtungen:

1. **Pivot-basiert**
   - Swing Low / Swing High aus bestätigten Pivots ableiten
   - erstes Modell: letztes bestätigtes gegensinniges Pivot-Paar

2. **Anchored / segment-basiert**
   - als evaluator `kind: "fibonacci_retracement"`
   - Fib-Levels für klar definierte Segmente exponieren

3. **Öffentliche API bereinigen**
   - die heutige Utility bleibt als Low-Level-Helfer bestehen
   - langfristig wäre ein klarerer Name wie `fibonacci_levels` sauberer als
     `fibonacci(candles, low, high)`

### Beschlossene Richtung
- Die heutige Funktion bleibt als **Low-Level-Utility** bestehen.
- Die primäre strategische Fibonacci-Nutzung soll ein **echter
  strukturbezogener anchored Indicator** werden.
- Output v1 soll minimal strukturiert sein und bewusst ausbaufähig bleiben.

---

## 3) Exponiert, aber **ohne O(1)-Cache** / nur Voll-Recompute

Aktuell haben nur diese drei Indikatoren eine explizite inkrementelle Cache-Anbindung:

- `ema`
- `rsi`
- `atr`

Folgende exponierte Indikatoren werden derzeit pro Aufruf voll neu berechnet:

### Trend
- `sma`
- `dema`
- `tema`
- `macd`
- `sar`
- `adx`
- `ichimoku`

### Momentum
- `cci`
- `stochastic`
- `williams_r`
- `roc`

### Volatility
- `bollinger`
- `keltner`

### Volume
- `obv`
- `vwap`
- `mfi`
- `volume_profile`

### Support / resistance / geometry
- `pivot_points`
- `fibonacci`
- `slope`

### Hinweis
Das ist nicht automatisch falsch, aber für Live-/Backtest-Parität und Performance ist das der nächste Integrationshebel.

---

## 4) Warmup-Erkennung

Die Warmup-Erkennung wurde bereits verbessert und behandelt jetzt zentrale
Sonderfälle indikator-spezifisch statt blind alle numerischen Parameter als
"Perioden" zu lesen.

Bereits gezielt behandelt:

- `ichimoku(candles)`
- `sar(candles, step, max)`
- `obv(candles)`
- `vwap(candles)`
- `volume_profile(candles, buckets)`
- `pivot_points(candles)`
- `fibonacci(candles, low, high)`

### Noch offen
- das Modell weiter in explizite, zentrale Warmup-Metadaten überführen
- zusätzliche Semantik-Tests für mehr Indicators ergänzen
- langfristig prüfen, ob die `+1`-Heuristik überall die gewünschte Produktsemantik abbildet

---

## 5) Binding-/Strategy-Tests

Stand jetzt gibt es für die öffentliche Rhai-Indicator-API grundlegende
Binding-Smoke-Tests auf Engine-Ebene.

### Noch offen
- tiefere Edge-Case-Tests statt nur Smoke-Tests
- weitere gezielte Offset-Semantik-Tests, nicht nur "Funktion existiert"
- Warmup-spezifische Tests für Sonderfälle
- spätere Tests für eine stärkere Fibonacci-Integration, falls wir Swing-/Pivot-Logik hinzufügen

### Bereits ergänzt
- Offset-Semantik-Tests für `obv`
- Offset-Semantik-Tests für `pivot_points`
- Offset-Semantik-Tests für `bollinger`
- Offset-Semantik-Tests für `stochastic`
- Offset-Semantik-Tests für `adx`

---

## 6) Priorisierte nächste Schritte

### P1 — API-Grenze sauber machen
- [x] Offset-Verhalten für fast alle history-basierten Rhai-Indikatoren vereinheitlicht
- [x] `engine/src/bindings.rs`-Kommentar an die Wahrheit angepasst
- [ ] bewusste Sonderrolle von `fibonacci` langfristig produktseitig neu entscheiden

### P2 — Nicht exponierte Rust-Helfer bewerten
- [ ] `ema_series` öffentlich für Strategien sinnvoll?
- [ ] `atr_series` öffentlich für Strategien sinnvoll?
- [ ] `ichimoku_custom` als author-facing API sinnvoll?

### P3 — Warmup robuster machen
- [x] Spezialfall für `ichimoku` ergänzt
- [x] Null-/Default-Parameter-Indikatoren bewusst behandelt
- [ ] Warmup-Hinweise langfristig zentraler modellieren

### P4 — Testlücken schließen
- [x] Pro exponiertem Rhai-Indicator mindestens ein Binding-Smoke-Test
- [ ] Semantik-/Edge-Case-Tests für ausgewählte kritische Indicators ergänzen
- [x] Erste Offset-Semantik-Suite für komplexere Map-/Array-Indikatoren ergänzt (`obv`, `pivot_points`, `bollinger`, `stochastic`, `adx`)

### P5 — Performance verbessern
- [ ] Prüfen, welche Voll-Recompute-Indikatoren echte O(1)- oder inkrementelle Pfade bekommen sollten

---

## Kurzfazit

**Nicht sauber integriert im engeren Sinn** sind aktuell vor allem:

1. **Rust-only, nicht in Rhai verfügbar**
   - `ema_series`
   - `atr_series`
   - `ichimoku_custom`

2. **Fibonacci ist noch eher Utility als voll integrierter Struktur-Indicator**
   - aktuell nur Preislevel-Berechnung aus `low/high`
   - langfristige Swing-/Pivot-/Anchored-Integration offen

3. **Exponiert, aber noch ohne stärkere Integrationsqualität**
   - fast alle außer `ema`, `rsi`, `atr` wegen fehlender Cache-/Smoke-Test-/teils Warmup-Anbindung
