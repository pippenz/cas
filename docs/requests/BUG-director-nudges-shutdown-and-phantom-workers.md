---
from: pantheon factory supervisor (live session)
date: 2026-07-01
priority: P1
related: BUG-director-coordinator-fabricates-completions-and-bad-nudges.md (idle-spam + wrong-key guidance), BUG-factory-liveness-signals-disagree.md, cas-c790, cas-e98e
---

# BUG — `director` emits ready/idle nudges for SHUT-DOWN and phantom workers (stale roster after `shutdown_workers`)

**Reporter:** factory supervisor `bright-condor-9`, live pantheon session `c5c67e82-...`, 2026-07-01
**Severity:** P1 — the director urged me (repeatedly) to assign real epic tasks to workers that no longer exist. A supervisor that trusts it would assign work into a void (task assigned to a dead session id/name → never executed) while believing the fleet is larger than it is.

This is a **new slice** distinct from the existing director ticket: not "who is alive is ambiguous," but "the director's worker roster is not invalidated on `shutdown_workers`, so it actively nudges about workers that were explicitly killed — plus at least one name that was never in this session's live roster at all."

## Live evidence (this session, verifiable against `worker_status`)

Exact sequence:

1. `spawn_workers count=3 cli=codex isolate=true` → batch A came up: `warm-spider-87`, `wild-wolf-70` (+ a third). They branched off the wrong base, so:
2. `shutdown_workers count=0 force=true` → then `worker_status` returned **"Workers: None active"** (authoritative — batch A is gone).
3. `spawn_workers count=3 cli=codex isolate=true` → batch B came up: `patient-dragon-41`, `silent-cheetah-81`, `ready-badger-29`. `worker_status` shows **exactly these 3**, fresh heartbeats, all assigned + ACK'd their tasks.
4. **The director then flooded me** with `"<w> is ready and waiting for tasks"` + `"<w> is idle with no assigned tasks"` for **`warm-spider-87`, `wild-wolf-70`, and `calm-crow-17`** — plus "There are 8 ready tasks available. Assign work: ... assignee=<name>".

At emit time, `worker_status` (the authoritative live roster) contained **none** of those three names — only batch B. So:
- `warm-spider-87`, `wild-wolf-70` = **shut-down** workers the director still treats as live/idle.
- `calm-crow-17` = a **phantom** — never present in this session's live `worker_status` at all.

The director also repeated the known wrong-key guidance (`assignee=<display-name>`, not session id) — see the related ticket.

## Relationship to existing tickets

- `BUG-director-coordinator-fabricates-completions-and-bad-nudges.md` item #3 covers idle/ready spam for busy workers + the supervisor. **This ticket adds the sharper root cause:** the roster the nudge path iterates is **not invalidated on `shutdown_workers`** (and admits ids never in the live set), so it nudges about dead/phantom workers. Fixing the nudge predicate to gate on "present in authoritative live roster" resolves both.

## Falsifiable hypotheses

| # | Hypothesis | Falsify |
|---|---|---|
| H4 | The director's nudge loop reads a cached/append-only agent registry, not the live `worker_status` set; `shutdown_workers` does not evict entries from it | After `shutdown_workers` + confirmed empty `worker_status`, if the director still emits ready/idle nudges for the shut-down ids, confirmed |
| H5 | A worker id (`calm-crow-17`) can appear in director nudges without ever entering this session's live roster | grep the director's roster source; if it unions across sessions / stale spawn requests rather than the current live workers, confirmed |
| H6 | Nudges are driven by queued spawn/agent-registration events that are not reconciled against subsequent shutdowns | trace the nudge trigger to its event source; if it fires off a stale spawn/registration without a shutdown reconcile, confirmed |

## Acceptance criteria

1. The director MUST NOT emit ready/idle nudges (or "assign work") for a worker absent from the authoritative live roster (`worker_status`). `shutdown_workers` must evict those ids from whatever set the nudge path iterates.
2. No nudge may reference a worker id that was never in the current session's live roster.
3. Regression coverage: spawn N → shutdown all → confirm zero director nudges reference the shut-down ids; spawn M new → confirm nudges only ever reference the M live workers.
4. Compose with the existing director/liveness fixes (same subsystem) — the roster-invalidation fix should also fix the "nudge for busy/supervisor" cases by sourcing from the single authoritative live set.

## Diagnostic recipe

```bash
# 1. spawn workers, capture live roster
#    worker_status  -> record the live names (set L1)
# 2. shutdown_workers count=0 force=true ; worker_status -> expect "None active"
# 3. watch the director stream for N seconds
#    Any "<name> is ready/idle" or "assign ... assignee=<name>" where <name> in L1
#    (or not in the current worker_status) = stale/phantom nudge = confirmed.
```
