---
from: Ozer supervisor (pippenz @ /home/pippenz/Petrastella/ozer)
date: 2026-07-14
priority: P1
---

# BUG: Worker died mid-task; task stayed `InProgress` indefinitely with no orphan/death signal to the supervisor

## Summary

Worker on `cas-5127` (P0 hotfix child of epic `cas-ea3e`) committed its finished fix to its worktree branch (`factory/hv-food-qa` @ `a90f98a6`, clean tree) and then its session died before `task close`. Afterwards:

- `task show cas-5127` → `Status: InProgress`, no staleness indicator, last note 13:59.
- `coordination worker_status` → "Workers: None active" — the dead worker simply vanished from the list; nothing marked it as died-while-holding-a-lease.
- No `worker_died` / `task_blocked` notification reached the supervisor (nothing surfaced in-session).
- The default 600s lease had long expired, but lease expiry did not flip the task to open/orphaned or annotate it.

I only discovered the orphan because a (stale, incorrect) director message claimed "all subtasks closed" and I went to verify. The completed work sat invisible in a worktree; the P0 epic would have stalled indefinitely.

## Environment

- `cas 2.27.0 (dd8bcbd-dirty 2026-07-11)`, factory mode, supervisor `fast-kestrel-14`, session `07275a32-c0d5-4695-abbb-5c04663df721`, project `/home/pippenz/Petrastella/ozer`
- Worker had been spawned by a different (director) session; worktree `.cas/worktrees/hv-food-qa`

## Expected

Some combination of:
1. Lease expiry on an `InProgress` task whose holder has no heartbeat → automatic transition to an explicit orphaned/open state (or at minimum a visible `⚠ lease expired, holder gone` marker in `task show` / `task list` / `epic_status`).
2. A `worker_died` queue notification to the supervising session(s) when a registered worker's heartbeat disappears while holding a lease.
3. `worker_status` listing recently-died workers (with last heartbeat + held task) instead of only live ones — "None active" hides the difference between "all done and shut down" and "one crashed mid-P0".

## Repro

1. Spawn a worker, assign a task, let it `task start`.
2. Kill the worker session (or let it crash) after it commits but before close.
3. Wait > lease duration. Observe: task remains `InProgress`, `worker_status` shows no workers, no notification is queued for the supervisor.

## Impact

Factory reliability: a P0 epic silently stalls on completed-but-unclosed work. The recovery path (`task action=reset` exists and is well-designed) is useless if nothing tells the supervisor a reset is needed.
