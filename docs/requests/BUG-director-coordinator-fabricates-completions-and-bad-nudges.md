---
from: cas-src supervisor (live factory session)
date: 2026-06-26
priority: P1
related: cas-c790 (phantom director nudges lead as idle worker), cas-e98e (liveness signals disagree), cas-b67d/cas-405f (prior director fixes, CLOSED)
---

# BUG — `director` coordinator fabricates "completed" notices, emits wrong assign-by-name guidance, and idle-spams the supervisor

**Reporter:** factory supervisor `sharp-pelican-14`, live session `4faf90cc-...`, 2026-06-26
**Severity:** P1 — the director's messages are actively misleading. A supervisor that trusts them will close/merge non-existent work, assign tasks with a key that doesn't resolve, and chase phantom idle workers. This session is a continuous reproduction.

## Live evidence (this session, all verifiable against git)

Running a 5-worker sprint, the `director` coordinator emitted, repeatedly:

1. **Fabricated completion notices.** "Worker X has completed task Y" for cas-f8e3, cas-7f2c, cas-9db0, cas-61af, cas-c790, cas-c496, cas-6d6d, cas-e2e2, cas-34f7f — while the worker branch was still at the base commit with **0 commits and a clean tree**. At least 6 of these were provably false at emit time (`git -C .cas/worktrees/<w> rev-list --count main..HEAD` == 0). The director declared "completed cas-34f7f" and "completed cas-c496" for tasks that to this minute have no commit.
2. **Wrong assignment guidance.** Every idle nudge says `Assign work: mcp__cas__task action=update id=<task-id> assignee=<worker-NAME>`. But a name-keyed assignee is invisible to the worker's `task mine` (which resolves on the worker's session id). Following the director's own instruction produces a task the worker can't see. Correct key is the worker's CAS session id.
3. **Relentless idle/ready spam.** Multiple "worker is ready and waiting" + "worker is idle with no assigned tasks" per worker per minute, including for workers that are mid-edit (dirty worktree) and for the supervisor itself.

## Relationship to existing tickets

- `cas-c790` covers one slice: director nudging a **lead/supervisor** as an idle worker on a single-session run.
- `cas-e98e` covers liveness-signal disagreement (worker_status / agent_list / FACTORY pane / OS procs).
- **This ticket is the missing slice:** the director's *message content* is wrong independent of liveness — false completions and a wrong assignment key. Fix should land alongside c790/e98e (same subsystem) but the completion + guidance bugs are distinct from "who's alive."

## Architecture map (find the emitter)

The director is a coordinator that synthesizes these strings. Locate it by grepping the exact emitted text:

- `"has completed task"` → the completion-notice path. It must be deriving "completed" from something other than a real terminal signal (likely: worker went idle / active_tasks emptied / a heartbeat-with-no-task). It needs to gate on an actual completion signal (task status transition to closed/verified, or a real commit), not idle.
- `"is idle with no assigned tasks"` / `"is ready and waiting for tasks"` → the idle-nudge path. Two bugs here: (a) the assignee template uses NAME not session id; (b) it fires for busy and for non-worker (lead/supervisor) agents.
- `"Assign work: mcp__cas__task action=update id="` → the literal guidance template to fix (session id, not name).

Grep `cas-cli/src` for those literals to find the module (coordination/director).

## Falsifiable hypotheses

| # | Hypothesis | Falsify |
|---|---|---|
| H1 | "completed" is derived from worker idle/active_tasks==0, not a real task close/commit | Trace the completion-notice trigger; if it fires on idle with the task still `in_progress` and 0 commits, confirmed |
| H2 | the assignee guidance template hard-codes `assignee=<name>` | grep the template string; if it interpolates name not session id, confirmed |
| H3 | idle nudges don't filter by role (lead/supervisor) or by current activity (dirty/active task) | check the nudge predicate; if it lacks a role guard + an activity guard, confirmed (overlaps cas-c790) |

## Acceptance criteria

1. The director NEVER emits "has completed task X" unless task X is actually closed/verified (status transition), or at minimum has a real commit on the worker branch. No idle-derived completions.
2. Assignment guidance, if emitted at all, uses the worker's **session id** as the assignee value (the key `task mine` resolves on) — never the display name.
3. Idle/ready nudges do not fire for: (a) leads/supervisors, (b) workers with an active in_progress task, (c) workers with a dirty worktree / recent activity.
4. Regression coverage for each of the three (fabricated-completion, wrong-key guidance, mis-targeted nudge).
5. Coordinate with cas-c790/cas-e98e so the three director/liveness fixes compose without conflict (same subsystem).

## Diagnostic recipe

```bash
# Reproduce the false-completion: assign a task, do NOT let the worker commit, watch for "has completed task"
# Ground truth at any moment:
for w in <worktrees>; do git -C ".cas/worktrees/$w" rev-list --count main..HEAD; done
# Any "completed" notice while that count is 0 for the named worker = fabricated.
```
