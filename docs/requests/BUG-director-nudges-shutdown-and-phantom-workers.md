---
from: pantheon factory supervisor (live session)
date: 2026-07-01 (updated 2026-07-02 — recurrence + new severe symptom)
priority: P0 (was P1) — see 2026-07-02 recurrence: director directed a cherry-pick to the production branch (`main`)
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

---

## Recurrence 2026-07-02 (same session c5c67e82) — NEW, more severe symptom: **harmful merge guidance** + stale/out-of-order delivery

**Reporter:** factory supervisor `bright-condor-9`, epic `cas-2ebb` (analytics asset-class filter), workers `jolly-otter-79` + `sturdy-condor-11`.

Beyond the stale-roster nudges above, the director this session did three new things. #1 is the serious one — it is not just noise, it is **actively dangerous guidance**:

### 1. Directed a prod-corrupting merge ("cherry-pick to main") — P0-class advice
On epic completion the director emitted:

> 🎉 All subtasks of epic 'Analytics asset-class filter…' (cas-2ebb) are now closed!
> Next steps: **Cherry-pick worker commits to main** · Verify the integrated result · Close the epic …

This is wrong on two counts and a supervisor that trusted it would ship unreviewed code to production:
- The epic was integrated via the correct flow — worker branches → `--no-ff` merges into the epic → epic → `develop` → (PR) → `staging`. There are no loose worker commits to cherry-pick.
- **`main` is the production branch.** "Cherry-pick worker commits to main" bypasses `develop`/`staging`/review entirely and drops raw factory commits onto prod. In this repo that is an explicit never-do (a standing rule the human operator has restated). The director should never emit branch/merge instructions, and certainly not ones that target `main`.

The generic "next steps" template appears to hard-code a `main`-centric cherry-pick workflow that does not match projects using a `develop → staging → main` promotion flow. It should not prescribe a merge strategy at all.

### 2. False "idle" for an actively-InProgress worker
The director sent `"sturdy-condor-11 is idle with no assigned tasks"` while `cas-3009` was **InProgress** and the worker was mid-edit on `analytics.vue` (verified: `task show cas-3009` → `Status: InProgress`, updated timestamp *after* the assignment, and the worker's own progress note landed ~same minute). Acting on this (reassigning) would have collided two workers on one task.

### 3. Stale, out-of-order message delivery (minutes-late echoes)
A batch of director + worker messages with source timestamps `13:00–13:03Z` was delivered to the supervisor much later in the session, **after** the referenced work (`cas-3456`, `cas-3009`, the whole epic) had already been merged and closed and the workers had stood down. The director re-emitted "worker idle / assign work / all subtasks closed → cherry-pick" for an epic that was already fully shipped. A supervisor replaying these at face value would re-open settled work.

### Added acceptance criteria (extend the list above)
5. The director MUST NOT emit branch/merge/promotion instructions. Specifically it must never tell a supervisor to "cherry-pick … to `main`" (or otherwise write to the production branch). Integration strategy is repo-specific and owned by the supervisor/human, not the coordinator. If a "next steps" template exists, strip the merge prescription.
6. Completion / "all subtasks closed" and idle/assign nudges must be gated on **current** authoritative state at emit time (live `worker_status` + live task status), not on queued/append-only events. A worker whose task is `InProgress` must never be reported idle; an epic already closed must not generate "next steps" nudges.
7. Message delivery should carry and respect ordering/freshness so minutes-stale echoes are not replayed as current directives (or are clearly marked stale).
