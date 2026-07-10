---
name: model-selection
description: Supervisor model/effort routing — match worker model tier to task complexity at breakdown, spawn, and escalation time.
managed_by: cas
---

# Model Selection — Matching Workers to Tasks

Pay for reasoning only where reasoning is the bottleneck. Every worker slot has three knobs — `cli`, `model`, `effort` — and the supervisor owns them: decide per task at breakdown, spawn the mix the backlog needs, escalate deliberately. Spawning everything at the session default wastes budget on chores and starves hard tasks of capability.

Routing is two stages. **Stage 1 — tier the task** by complexity; the tier is a stable property of the work. **Stage 2 — pick the lane** that fills that tier:

- **Codex is the default matrix** — every tier resolves to a Codex `gpt-5.5`/`gpt-5.6-sol` spawn.
- **Claude Max is a fit overlay** — route to it immediately when the work benefits (taste, Claude-native tooling, architecture, safety, rescue, independent review), not only after Codex fails.
- **Grok is a capacity overlay** — route to it while its credits/auth/throughput are healthy; fall back to the same Codex tier when they are not.

Evidence rule: change routing guidance only on independent, cross-vendor evidence (e.g. Artificial Analysis II/CAI leaderboards **plus** a live harness check) — never on a single failed spawn. A spawn failure from a mistyped slug tests the slug, not the model. Optional write-up: `docs/reports/2026-07-09-factory-model-matrix-recommendations.html` (rev 2).

## Tiers (Codex-first)

| Tier | Spawn parameters | Use for |
|---|---|---|
| **light** | `cli=codex model=gpt-5.5 effort=low` | Chores, docs, mechanical renames, config bumps, `depth=light` tasks, test backfill that mirrors existing patterns. |
| **standard** | `cli=codex model=gpt-5.5 effort=medium` | Normal feature/bug work with a clear spec and bounded blast radius. The stock floor. |
| **heavy** | `cli=codex model=gpt-5.5 effort=high` | Cross-cutting refactors, concurrency/lifecycle code, migrations, gnarly debugging, P0/P1 critical-path units. |
| **frontier** | `cli=codex model=gpt-5.6-sol effort=high` | Architecture-defining units, high-blast-radius changes, tasks that already bounced twice. Sparingly — every frontier worker should map to named tasks. The exact slug is `gpt-5.6-sol`; a bare `gpt-5.6` is **not** a valid spawn recipe. |

Token-heavy read-only investigation belongs in a `cas-codex-exec` shell-out, not a worker and not your own context window.

### Claude Max lane (fit routing — not a last resort)

The operator has a Claude Code Max subscription; use that prepaid capacity where Claude materially fits the task. Route **immediately**, not only after Codex bounces:

- **`cli=claude model=sonnet effort=high`** — taste-sensitive output (UI/UX, API/SDK shape, naming, prompts, docs, release notes, error wording) and Claude-native work (Claude Code harness / skills / hooks compatibility, public-facing surfaces).
- **`cli=claude model=opus effort=high`** — architecture judgment, safety-critical changes, rescue of a stuck task, and independent / second-opinion review.

Max is still quota-limited capacity: keep `effort=high` as the ceiling on long worker loops (no `xhigh`/`max`), preserve explicit `cli`/`model`/`effort`, and fall back to the equivalent Codex tier when the Claude usage window is constrained.

### Grok lane (capacity routing — health-gated)

Grok is an optional credit/capacity route, not a required rung. Use it while healthy; fall back to the same-tier Codex rung when not.

- **`cli=grok model=grok-composer-2.5-fast effort=low`** — light / flash lane (Composer is a Grok model id, never `cli=cursor`); same-tier Codex fallback is `gpt-5.5 effort=low`.
- **`cli=grok model=grok-4.5 effort=medium|high`** — standard / heavy capacity; same-tier Codex fallback is `gpt-5.5 effort=medium|high`.

Health check before routing to Grok: credits/quota available, auth valid, throughput healthy (`grok models` responds). If any is red, take the same-tier Codex rung instead.

### Model slug table

| `cli=` | Accepted `model=` slugs | Notes |
|---|---|---|
| `codex` | `gpt-5.5`, `gpt-5.6-sol` | Plain slugs only — `-codex`-suffixed slugs are rejected by the API. Frontier is the exact slug `gpt-5.6-sol`; bare `gpt-5.6` is invalid. |
| `claude` | `sonnet`, `opus`, `haiku` (full Anthropic ids also ok) | Aliases preferred in factory docs. |
| `grok` | `grok-4.5`, `grok-composer-2.5-fast` | From live `grok models`. Composer is a **model id on the Grok harness** — never invent `cli=cursor`. |

### Effort vocabulary (CAS-wide)

Accepted values: `minimal` \| `low` \| `medium` \| `high` \| `xhigh` (alias `x-high`).

How each backend receives them:

| Backend | Flag / config |
|---|---|
| Claude | `--effort <level>` |
| Codex | `--config model_reasoning_effort=<level>` |
| Grok | `--reasoning-effort <level>` |

For multi-step workers, `effort=high` is the ceiling — do not reach for `xhigh`/`max` on long agent loops (overthink + cost). Codex tiers move on effort (`low` → `medium` → `high`) then on model (`gpt-5.5` → `gpt-5.6-sol`).

## Spawn cookbook (all three harnesses)

Copy-paste `spawn_workers` recipes. Examples below use this harness's coordination tool prefix. Worker `cli=`/`model=`/`effort=` are independent of which harness the supervisor runs on — `cli=codex` works from Claude, Codex, or Grok supervisors alike.

### Codex workers (default matrix)

```
# light / bulk
mcp__cas__coordination action=spawn_workers count=2 isolate=true cli=codex model=gpt-5.5 effort=low

# standard floor
mcp__cas__coordination action=spawn_workers count=2 isolate=true cli=codex model=gpt-5.5 effort=medium

# heavy
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=codex model=gpt-5.5 effort=high worker_names="hv-ada"

# frontier — exact slug gpt-5.6-sol
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=codex model=gpt-5.6-sol effort=high worker_names="fr-ada"
```

### Claude Max workers (fit: taste / architecture / review)

```
# taste / Claude-native work
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=claude model=sonnet effort=high worker_names="sn-ada"

# architecture / safety / rescue / independent review
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=claude model=opus effort=high worker_names="op-ada"
```

### Grok workers (capacity — use while credits/auth/throughput healthy)

```
# light / flash — Composer model id on cli=grok
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=grok model=grok-composer-2.5-fast effort=low worker_names="lt-ada"

# standard / heavy capacity
mcp__cas__coordination action=spawn_workers count=2 isolate=true cli=grok model=grok-4.5 effort=medium
mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=grok model=grok-4.5 effort=high worker_names="gh-ada"
```

Parameter table and field names: [reference.md](reference.md#spawn_workers-parameters).

## Routing Axes

Use tier labels as defaults, then check four axes before spawning:

| Tier | Cost | Intelligence | Speed | Taste |
|---|---|---|---|---|
| **light** | Lowest agent $/task (Codex gpt-5.5 low; Grok Composer when capacity-routed) | Sufficient for well-bounded mechanical work | Highest — low wall time / flash lane | Low: fine for renames and internal scaffolding; review public surfaces |
| **standard** | Low — Codex gpt-5.5 medium is the cost-efficient floor | High for bounded engineering; default for most factory work | High throughput on sustained agent loops | Low-to-mid: fine for internal code; review user-facing prose |
| **heavy** | Codex gpt-5.5 high; Claude/Grok only when fit/capacity says so | High for messy codebases, lifecycle bugs, multi-module judgment | Strong multi-step agent loops; slower than light on tiny tasks | Mid: good default for critical-path code; route Claude for taste |
| **frontier** | Highest — reserve for quality/risk that justifies it | Highest ceiling (Codex gpt-5.6-sol; Claude Opus for architecture / review) | Slowest / most expensive agent loops | High: taste-sensitive output that must land cleanly |

Glossary:

- **Cost** is budget spent per task (prefer $/task and tokens/task over list $/M tokens alone). Codex is the efficient default lane; Claude Max is prepaid but quota-limited; Grok is credit-gated capacity.
- **Intelligence** is how hard a problem the model can handle unsupervised: ambiguity, hidden coupling, long reasoning chains, and unfamiliar code.
- **Speed** is wall-clock and throughput: decode TPS, agent task wall time, and tokens burned per task.
- **Taste** is the quality of what ships: UI/UX judgment, API and SDK shape, naming, code style, prompts, docs, release notes, and error-message wording.

Taste-sensitive work routes to a high-taste tier even when the task is mechanically simple. Skill wording, supervisor guidance, release notes, public docs, API/SDK surfaces, and user-facing error text are not "light" just because the diff is small — start those on Claude Max (Sonnet → Opus), not the cheapest lane.

## Reading the task signals

Score each task while breaking down the EPIC:

- `task_type=chore`, docs-only, or `depth=light` → **light**
- Spike whose question is architectural ("which design holds at 10x?") → **heavy** or **frontier**; mechanical spikes ("does the API return X?") → **light** or **standard**
- Priority 0–1 AND on the critical path → at least **heavy**
- Touches 3+ modules, shared traits/schemas, or unwind/panic/locking behavior → **heavy**
- You would read the diff twice yourself before merging → **frontier**
- Taste, Claude-native, architecture, safety, rescue, or independent review in play → the **Claude Max lane** at the matching tier
- Everything else → **standard** (the default is the default for a reason)

Effort is the main cost lever on reasoning-capable models. Prefer raising **effort on Codex gpt-5.5** (`low` → `medium` → `high`) before changing lane or model. Move to `gpt-5.6-sol` for frontier reasoning; move to Claude Max for fit (taste / architecture / review); move to Grok for capacity relief.

For every worker, `effort=high` is the ceiling. `xhigh`/`max` increase per-step reasoning, not step count or run length; on hard multi-step work they tend to overthink each move, produce heavier diffs, and multiply cost. Escalate the model tier or split the task before raising effort above `high`.

## Workflow

1. **Tag at breakdown** — tasks default to standard; tag deviations with a label: `labels="tier:light"` / `"tier:heavy"` / `"tier:frontier"`. Note non-obvious tier rationale (and any fit/capacity lane choice) in the task's `design` field.
2. **Spawn the mix** — count tiers in the ready backlog, then issue one `spawn_workers` call per tier (a call's parameters apply to every worker in that call):
   ```
   # two standard workers (floor)
   mcp__cas__coordination action=spawn_workers count=2 isolate=true cli=codex model=gpt-5.5 effort=medium

   # one light worker for chores
   mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=codex model=gpt-5.5 effort=low worker_names="lt-ada"

   # one heavy worker for tier:heavy tasks
   mcp__cas__coordination action=spawn_workers count=1 isolate=true cli=codex model=gpt-5.5 effort=high worker_names="hv-ada"
   ```
   Every `spawn_workers` call MUST include explicit `cli=`, `model=`, and `effort=`.
   Relying on omitted fields is a fallback path that emits an acknowledgement
   warning, not an approved supervisor workflow.
   Tiers change the fleet's composition, not its size — worker-count strategy (3–4 max, sized by independent file groups) still applies.
3. **Route by tier and lane** — assign `tier:*`-labelled tasks to a matching worker; standard tasks go to anyone at that tier. Send taste / architecture / review to the Claude Max lane; send capacity overflow to Grok while healthy. Name heavier workers so routing stays legible (`hv-*`, `fr-*`, `op-*`).
4. **Escalate on failure** — a task rejected or verification-failed twice moves up one tier: raise Codex effort, then Codex model (`gpt-5.5` → `gpt-5.6-sol`), or move to the Claude Max lane. Don't re-run the same task on the same tier hoping for different output.
5. **Escalate on judgment** — the two-rejection rule is a floor, not a permission gate. If a cheaper worker's draft gathers facts but misses the quality bar, escalate before verification fails. Judge the output, not the price tag; use cheap tiers for information and drafts, then pay for what ships. Cost is a tiebreaker only.

### Escalation ladder (Codex-first, with fit/capacity overlays)

```
light     codex  model=gpt-5.5 effort=low
→ standard  codex  model=gpt-5.5 effort=medium
  → heavy     codex  model=gpt-5.5 effort=high
    → frontier  codex  model=gpt-5.6-sol effort=high   # exact slug; +reasoning ceiling
  fit overlay:  claude model=sonnet|opus effort=high   # taste / architecture / rescue / review
  capacity:     grok   model=grok-composer-2.5-fast effort=low | grok-4.5 effort=medium|high (health-gated)
```

- Raise Codex effort before switching model; raise Codex model before assuming you need another vendor.
- Taste, Claude-native, architecture, safety, rescue, or independent review can jump straight to Claude Max even if the diff is small.
- Route to Grok only while its credits/auth/throughput are healthy; otherwise take the same-tier Codex rung.
6. **De-escalate the tail** — when only light tasks remain, don't leave a heavy/frontier worker idle-burning; shut it down and let light workers sweep the tail.

Explicit per-spawn parameters beat `.cas/config.toml` `[factory.defaults]` / `[[factory.workers]]` for that spawn only — check the project config before assuming what the floor actually is.
