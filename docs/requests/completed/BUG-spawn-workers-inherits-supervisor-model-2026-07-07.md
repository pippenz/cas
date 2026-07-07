# BUG: spawn_workers with no model/effort silently inherits the supervisor's model — cost footgun

**From:** petra-stella-cloud team (supervisor session, 2026-07-07)
**Severity:** High (cost), Low (correctness)
**Component:** factory / `mcp__cas__coordination action=spawn_workers`

## What happened

A supervisor session running **claude-fable-5 at high effort** issued:

```
mcp__cas__coordination action=spawn_workers count=3 isolate=true cli=claude
```

No `model` or `effort` was passed. All three workers spawned inheriting the supervisor's session model — **three Fable 5 / high-effort workers**, one of which was assigned a `tier:light` task (delete a duplicate `<h2>`, add a dark-theme CSS override). The operator spotted the fleet and killed it before instructions were delivered; only boot tokens were burned, but at Mythos-tier pricing a full wave of this would have been very expensive.

- cas 2.27.0 (9ebc844 2026-07-07)
- Workers spawned: zen-puma-94, steady-marten-47, gentle-gopher-35 (all `cli=claude`, no model override → fable-5 inherited)
- Repo: petra-stella-cloud, epic cas-83ec

## Why this is a footgun

1. **The default is the most expensive possible choice whenever the supervisor runs a frontier model.** Inheritance is a sensible default for `cli`, but for `model` it means "supervisor tier = worker tier" — the exact opposite of the model-selection rubric (`cas-supervisor/references/model-selection.md`), which says the stock floor is codex/gpt-5.5/medium and frontier workers should map to named tasks only.
2. **It's silent.** The spawn result ("Queued spawn request for 3 worker(s)") and `worker_status` output show branch/heartbeat/clone but **not the model or effort** each worker is running. The operator only found out from billing-side observation. There is no confirmation, no warning, nothing to review.
3. Supervisor skill docs say "omit = session default" for the *standard* tier, which reads as "a sane default" — but the session default is whatever the supervisor happens to be, which for Fable/Opus supervisors is frontier.

## Suggested fixes (any subset helps)

1. **Decouple the worker default from the supervisor's model.** When `model`/`effort` are omitted, resolve from `[factory.defaults]` in `.cas/config.toml` (or a hardcoded sane floor, e.g. sonnet/medium for `cli=claude`), NOT from the supervisor session.
2. **Warn or require confirmation when the resolved worker model is frontier-tier** (opus/fable/mythos) and no explicit `model=` was passed — e.g. `⚠️ workers will inherit claude-fable-5 (frontier). Pass model= explicitly or set [factory.defaults].`
3. **Show model + effort per worker in `worker_status` and in the spawn ack.** Right now fleet composition is invisible, so a mis-tiered fleet can't be caught by inspection.

## Workaround in the meantime

Supervisors must always pass explicit `model=` and `effort=` on every `spawn_workers` call (we've stored this as a team memory). But defaults that require discipline to avoid a 10x+ cost multiplier are still bugs.
