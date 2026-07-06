---
name: model-selection
description: Supervisor model/effort routing — match worker model tier to task complexity at breakdown, spawn, and escalation time.
managed_by: cas
---

# Model Selection — Matching Workers to Tasks

Pay for reasoning only where reasoning is the bottleneck. Every worker slot has three knobs — `cli`, `model`, `effort` — and the supervisor owns them: decide per task at breakdown, spawn the mix the backlog needs, escalate deliberately. Spawning everything at the session default wastes budget on chores and starves hard tasks of capability.

## Tiers

| Tier | Spawn overrides | Use for |
|---|---|---|
| **light** | `cli=codex model=gpt-5.5 effort=low` | Chores, docs, mechanical renames, config bumps, `depth=light` tasks, test backfill that mirrors existing patterns |
| **standard** | *(omit — session default)* | Normal feature/bug work with a clear spec and bounded blast radius. The stock floor is codex / gpt-5.5 / medium. |
| **heavy** | `cli=claude model=sonnet effort=high` | Cross-cutting refactors, concurrency/lifecycle code, migrations, gnarly debugging, P0/P1 critical-path units |
| **frontier** | `cli=claude model=opus effort=high` | Architecture-defining units, high-blast-radius changes, tasks that already bounced twice. Sparingly — every frontier worker should map to named tasks. |

Model slugs: Claude accepts the `sonnet` / `opus` / `haiku` aliases. Codex subscription accounts must use plain `gpt-5.5` — `-codex`-suffixed slugs are rejected by the API.

## Reading the task signals

Score each task while breaking down the EPIC:

- `task_type=chore`, docs-only, or `depth=light` → **light**
- Spike whose question is architectural ("which design holds at 10x?") → **heavy**; mechanical spikes ("does the API return X?") → **light** or **standard**
- Priority 0–1 AND on the critical path → at least **heavy**
- Touches 3+ modules, shared traits/schemas, or unwind/panic/locking behavior → **heavy**
- You would read the diff twice yourself before merging → **frontier**
- Everything else → **standard** (the default is the default for a reason)

Effort is the main cost lever on subscription-metered backends. For long-running heavy work, prefer switching model tier (`cli=claude model=sonnet effort=high`) over pinning codex at `high`/`xhigh` — sustained deep-effort runs on a metered subscription burn budget disproportionately fast.

## Workflow

1. **Tag at breakdown** — tasks default to standard; tag deviations with a label: `labels="tier:light"` / `"tier:heavy"` / `"tier:frontier"`. Note non-obvious tier rationale in the task's `design` field.
2. **Spawn the mix** — count tiers in the ready backlog, then issue one `spawn_workers` call per tier (a call's overrides apply to every worker in that call):
   ```
   # two standard workers on the session default
   mcp__cas__coordination action=spawn_workers count=2 isolate=true

   # one heavy worker for the tier:heavy tasks
   mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=claude model=sonnet effort=high worker_names="hv-ada"
   ```
   Tiers change the fleet's composition, not its size — worker-count strategy (3–4 max, sized by independent file groups) still applies.
3. **Route by tier** — assign `tier:*`-labelled tasks to a matching worker; standard tasks go to anyone. Name heavier workers so routing stays legible (`hv-*`, `fr-*`).
4. **Escalate on failure** — a task rejected or verification-failed twice moves up one tier: reassign to an existing heavier worker or spawn one. Don't re-run the same task on the same tier hoping for different output.
5. **De-escalate the tail** — when only light tasks remain, don't leave a heavy/frontier worker idle-burning; shut it down and let a cheaper worker sweep the tail.

Per-spawn overrides beat `.cas/config.toml` `[factory.defaults]` / `[[factory.workers]]` for that spawn only — check the project config before assuming what the floor actually is.
