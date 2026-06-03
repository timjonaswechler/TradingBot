# Agent Instructions

## Runtime refactor work

For any task related to the Trading Runtime refactor, agents must read these before changing code:

- `CONTEXT.md` — domain language only.
- `docs/refactor/trading-runtime-refactor-plan.md` — target plan and implementation phases.
- `docs/refactor/runtime-migration-control.md` — canonical/donor/transitional/removable code map.
- Relevant ADRs in `docs/adr/`.
- The active GitHub issue and any referenced parent/child issues.

Before building on old code, classify the path explicitly:

- **canonical** — build target behavior here.
- **donor** — port behavior from here, but do not extend it as architecture.
- **transitional** — adapter/migration glue only.
- **removable** — can be deleted once protected by target behavior/tests.

Runtime refactor guardrails:

- Do not add new Portfolio/Execution semantics to `backtester` or `trading-daemon`.
- Do not add DB details to `trading-runtime`.
- Do not add new Runtime semantics to `domain` while #36 semantic cleanup is unresolved.
- Do not extend `engine` as a target architecture; it is donor material only.
- If a gap is found, cite the issue that owns it or stop and ask for clarification.
- Delete donor/transitional code only after target behavior exists, tests protect it, and no active issue still depends on the old path.
