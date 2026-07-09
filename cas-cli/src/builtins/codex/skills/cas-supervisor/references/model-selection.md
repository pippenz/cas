---
name: model-selection
description: Supervisor model/effort routing — match worker model tier to task complexity at breakdown, spawn, and escalation time.
managed_by: cas
---

# Model Selection — Matching Workers to Tasks

Pay for reasoning only where reasoning is the bottleneck. Every worker slot has three knobs — `cli`, `model`, `effort` — and the supervisor owns them: decide per task at breakdown, spawn the mix the backlog needs, escalate deliberately. Spawning everything at the session default wastes budget on chores and starves hard tasks of capability.

Evidence baseline (Jul 2026): Artificial Analysis leaderboards (II / CAI) plus live `grok models` on Grok Build justify Grok/Composer as the factory default lane. Optional full write-up: `docs/reports/2026-07-09-factory-model-matrix-recommendations.html` (rev 2).

## Tiers

| Tier | Spawn parameters | Use for |
|---|---|---|
| **light** | `cli=grok model=grok-composer-2.5-fast` | Chores, docs, mechanical renames, config bumps, `depth=light` tasks, test backfill that mirrors existing patterns. Composer is a **Grok Build model id**, not a separate `cli=` harness — never invent `cli=cursor`. |
| **standard** | `cli=grok model=grok-4.5 effort=medium` | Normal feature/bug work with a clear spec and bounded blast radius. The stock floor. |
| **heavy** | `cli=grok model=grok-4.5 effort=high` | Cross-cutting refactors, concurrency/lifecycle code, migrations, gnarly debugging, P0/P1 critical-path units. Sonnet is escalate-for-taste, not default heavy. |
| **frontier** | `cli=claude model=opus effort=high` | Architecture-defining units, high-blast-radius changes, tasks that already bounced twice. Sparingly — every frontier worker should map to named tasks. |

Capacity backups (not preferred defaults): when Grok quota is tight, Codex works as bulk backup — `cli=codex model=gpt-5.5 effort=low|medium`. Haiku remains optional for Claude-only tiny tasks.

Token-heavy read-only investigation belongs in a `cas-codex-exec` shell-out, not a worker and not your own context window.

Model slugs:

- **Grok** (`cli=grok`): ids from `grok models` — currently `grok-4.5` (default) and `grok-composer-2.5-fast`. Pass Composer only as `model=grok-composer-2.5-fast` on `cli=grok`.
- **Claude**: `sonnet` / `opus` / `haiku` aliases.
- **Codex**: plain `gpt-5.5` — `-codex`-suffixed slugs are rejected by the API.

## Routing Axes

Use tier labels as defaults, then check four axes before spawning:

| Tier | Cost | Intelligence | Speed | Taste |
|---|---|---|---|---|
| **light** | Lowest agent $/task in the Grok lane (Composer Fast) | Sufficient for well-bounded mechanical work | Highest — low wall time / flash lane | Low: fine for renames and internal scaffolding; review public surfaces |
| **standard** | Low — Grok 4.5 medium is the cost-efficient floor | High for bounded engineering; default for most factory work | High throughput (TPS + tokens/task) vs Claude peers on similar agent scores | Low-to-mid: fine for internal code; review user-facing prose |
| **heavy** | Still Grok-priced; escalate to Claude only when needed | High for messy codebases, lifecycle bugs, multi-module judgment | Strong multi-step agent loops; slower than Composer on tiny tasks | Mid: good default for critical-path code; escalate Claude for taste |
| **frontier** | Highest — reserve for quality/risk that justifies it | Highest available ceiling (Opus / Fable-class when present) | Slowest / most expensive agent loops | High: taste-sensitive output that must land cleanly |

Glossary:

- **Cost** is budget spent per task (prefer $/task and tokens/task over list $/M tokens alone). Grok/Composer is the efficient default lane; Claude subscription workers are the scarce pool.
- **Intelligence** is how hard a problem the model can handle unsupervised: ambiguity, hidden coupling, long reasoning chains, and unfamiliar code.
- **Speed** is wall-clock and throughput: decode TPS, agent task wall time, and tokens burned per task. Prefer Composer for flash work; Grok 4.5 for sustained agent loops.
- **Taste** is the quality of what ships: UI/UX judgment, API and SDK shape, naming, code style, prompts, docs, release notes, and error-message wording.

Taste-sensitive work routes to a high-taste tier even when the task is mechanically simple. Skill wording, supervisor guidance, release notes, public docs, API/SDK surfaces, and user-facing error text are not "light" just because the diff is small — start those on Claude (Sonnet → Opus), not Composer.

## Reading the task signals

Score each task while breaking down the EPIC:

- `task_type=chore`, docs-only, or `depth=light` → **light**
- Spike whose question is architectural ("which design holds at 10x?") → **heavy** or **frontier**; mechanical spikes ("does the API return X?") → **light** or **standard**
- Priority 0–1 AND on the critical path → at least **heavy**
- Touches 3+ modules, shared traits/schemas, or unwind/panic/locking behavior → **heavy**
- You would read the diff twice yourself before merging → **frontier**
- Everything else → **standard** (the default is the default for a reason)

Effort is the main cost lever on reasoning-capable models. On a Grok-first factory, prefer raising **effort on Grok 4.5** (`medium` → `high`) before switching to Claude. Escalate to `cli=claude model=sonnet effort=high` only for taste, Claude-lifecycle thrash, or after Grok has already failed twice — watch Sonnet $/task (verbosity can make it more expensive than Opus).

For Claude workers, `effort=high` is the ceiling. `xhigh`/`max` increase per-step reasoning, not step count or run length; on hard multi-step work they tend to overthink each move, produce heavier diffs, and multiply cost. Escalate the model tier or split the task before raising Claude effort above `high`.

## Workflow

1. **Tag at breakdown** — tasks default to standard; tag deviations with a label: `labels="tier:light"` / `"tier:heavy"` / `"tier:frontier"`. Note non-obvious tier rationale in the task's `design` field.
2. **Spawn the mix** — count tiers in the ready backlog, then issue one `spawn_workers` call per tier (a call's parameters apply to every worker in that call):
   ```
   # two standard workers (floor)
   mcp__cs__coordination action=spawn_workers count=2 isolate=true cli=grok model=grok-4.5 effort=medium

   # one light / flash worker for chores
   mcp__cs__coordination action=spawn_workers count=1 isolate=true cli=grok model=grok-composer-2.5-fast worker_names="lt-ada"

   # one heavy worker for tier:heavy tasks
   mcp__cs__coordination action=spawn_workers count=1 isolate=true cli=grok model=grok-4.5 effort=high worker_names="hv-ada"
   ```
   Every `spawn_workers` call MUST include explicit `model=` (and `effort=` for standard/heavy/frontier and any Claude/Codex spawn).
   Light Composer on Grok is model-id only — still pass `cli=grok model=grok-composer-2.5-fast` explicitly.
   Relying on omitted fields is a fallback path that emits an acknowledgement
   warning, not an approved supervisor workflow.
   Tiers change the fleet's composition, not its size — worker-count strategy (3–4 max, sized by independent file groups) still applies.
3. **Route by tier** — assign `tier:*`-labelled tasks to a matching worker; standard tasks go to anyone. Name heavier workers so routing stays legible (`hv-*`, `fr-*`).
4. **Escalate on failure** — a task rejected or verification-failed twice moves up one tier: reassign to an existing heavier worker or spawn one. Don't re-run the same task on the same tier hoping for different output.
5. **Escalate on judgment** — the two-rejection rule is a floor, not a permission gate. If a cheaper worker's draft gathers facts but misses the quality bar, escalate before verification fails. Judge the output, not the price tag; use cheap tiers for information and drafts, then pay for what ships. Cost is a tiebreaker only.

### Escalation ladder (Grok-first)

```
light     grok  model=grok-composer-2.5-fast
→ standard  grok  model=grok-4.5 effort=medium
  → heavy     grok  model=grok-4.5 effort=high
    → heavy+    claude model=sonnet effort=high   # taste / Claude harness
      → frontier  claude model=opus effort=high     # +II ceiling; rare
```

- Composer → Grok 4.5 when the task needs deeper reasoning or multi-module judgment, not just speed.
- Do **not** auto-promote standard → heavy by switching to Sonnet; prefer Grok high first.
- Taste-sensitive work can jump straight to Claude even if the diff is small.
6. **De-escalate the tail** — when only light tasks remain, don't leave a heavy/frontier worker idle-burning; shut it down and let Composer workers sweep the tail.

Explicit per-spawn parameters beat `.cas/config.toml` `[factory.defaults]` / `[[factory.workers]]` for that spawn only — check the project config before assuming what the floor actually is.
