# Runtime-managed risk exits are opt-in hard exits

- Status: accepted
- Date: 2026-05-26

Entry Risk Parameters on opening Strategy Decisions opt a position into runtime-managed hard exits; strategies that want full control over exits omit stop-loss and take-profit and later produce Strategy Exits themselves. Risk Exits are checked only on Tradable Candles before a Strategy Tick, never on Warmup Input, and live restart does not retroactively execute missed past candles as if the market could be replayed. Risk Exit pricing is gap-aware, Stop-Loss is chosen when both Stop-Loss and Take-Profit are hit intrabar, and Runtime Events should expose typed selected/triggered Risk Exit information so reporting can distinguish Strategy Exits, Risk Exits, and Force Closes.
