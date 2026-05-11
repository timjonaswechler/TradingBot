
 ### 3. Warmup sauber machen

 Danach Warmup-Erkennung verbessern, vor allem für:
 - ichimoku
 - parameterlose/spezielle Indicators
 - Fälle, wo aktueller Fallback zu grob ist

 Skill: tdd

 ────────────────────────────────────────────────────────────────────────────────

 ### 4. Rust-only Helpers bewerten

 Dann prüfen, ob diese wirklich public nach Rhai sollen:
 - ema_series
 - atr_series
 - ichimoku_custom

 Hier erst Produktentscheidung, dann Implementierung.

 Skill: erst normal entscheiden, dann tdd

 ────────────────────────────────────────────────────────────────────────────────

 ### 5. Performance-Integration

 Danach schauen, welche voll recomputenden Indicators echte inkrementelle Pfade bekommen sollen.

 Priorität eher später, nachdem API + Tests stabil sind.

 Skill: eher diagnose oder normales Implementieren, nicht zwingend zuerst tdd

 ────────────────────────────────────────────────────────────────────────────────

 Konkrete Reihenfolge, die ich empfehle

 1. TDD: Smoke tests für alle public indicators
 2. TDD: Offset-Konsistenz fixen
 3. TDD: Warmup-Detection verbessern
 4. entscheiden, welche Rust-only helper public werden
 5. danach Performance/Caching

 Kurz gesagt

 Erst Contract absichern, dann API glätten, dann erweitern.

 Wenn du willst, formuliere ich dir direkt den ersten TDD-Arbeitsslice als genaue Task.
