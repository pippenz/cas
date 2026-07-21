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

## Resolution (cas-2e81, 2026-07-14)

Fixed in factory worker branch `factory/hv-orphan`.

### Root cause
- `mark_stale` revoked leases but left task rows as `InProgress` with assignee intact.
- `worker_status` prune used that path and then showed only the Active roster ("None active"), hiding died-while-leased.
- Daemon maintenance only annotated notes on InProgress tasks — no status flip, no `worker_died` queue event.
- Lease reclaim (`reclaim_expired_leases`) similarly cleared leases without parking tasks.

### Fix
- New module `cas-cli/src/mcp/tools/service/orphan_recovery.rs`:
  - Parks eligible InProgress/Blocked tasks → Open + clears assignee + audit note
  - Skips PSR / AwaitingMerge / Closed / Open (cas-6e4c invariant)
  - Records `EventType::WorkerDied` and queues critical `worker_died` supervisor notifications
- Wired into: `factory_worker_status` prune, `cas_agent_cleanup`, daemon maintenance, embedded daemon maintenance
- `worker_status` now appends **Recently died while leased (N)** with last heartbeat + held task ids

### Evidence (focused tests)
```
cargo test --test factory_mcp_ops_test test_2e81 -- --test-threads=1
# test_2e81_worker_status_parks_orphaned_inprogress_on_stale_prune ... ok
# test_2e81_worker_status_distinguishes_empty_fleet_vs_died_while_leased ... ok
# test_2e81_agent_cleanup_parks_orphan_and_emits_worker_died ... ok
# test_2e81_orphan_recovery_skips_psr_tasks ... ok
# 4 passed
```

Also: `cargo test --test factory_mcp_ops_test test_worker_status -- --test-threads=1` → 7 passed (existing liveness suite still green).
