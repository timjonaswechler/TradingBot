# Strategy State v2 Complex and Persistent State

Strategy State v2 with complex values or general persistence is not planned for now.

## Why this is out of scope

The current Strategy State model is intentionally primitive and session-local. It gives strategies small scratch memory between Strategy Ticks without turning `context.state` into a second source of Portfolio State, Market State, or Compute State.

The concrete need discussed during triage was that a bot should know after live restart whether it has an open trading position. That is not Strategy State. Open position, entry price, size, stops, take-profits, cash, equity, and completed trade count are Portfolio State. Live restart restoration for those values belongs in the Live Runner / DB persistence seam that initializes the Trading Runtime with the correct Portfolio State, not in strategy-authored key-value memory.

Adding arrays, maps, host objects, or durable Strategy State without a specific strategy-internal use case would risk non-replayable live behavior and live/backtest divergence. If a future strategy needs richer session-local scratch values such as rolling arrays or setup phase objects, that should be raised as a focused issue with examples and without coupling it to Portfolio State restore.

## Prior requests

- #81 — Strategy State: Decide v2 value types and persistence semantics
