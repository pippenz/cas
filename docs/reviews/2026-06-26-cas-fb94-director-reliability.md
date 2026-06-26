# Code Review — cas-fb94 director-reliability epic (assembled)

**Date:** 2026-06-26
**Reviewer:** supervisor (fierce-puma-23), line-by-line pass
**Range:** `b078b72..HEAD` in assembly worktree (8 commits)
**Gate:** `cargo build` exit 0; cas-pty+cas-mux 150 tests, director 47, factory_ops 47, mcp_tools_test 175 — all green on the merged tree.

## Verdict: APPROVE for merge. No P0/P1 blockers; 3 low-severity advisories.

## Scope
| Commit | Task | Facet |
|---|---|---|
| ffe89b2 | cas-55dc | A/B — TaskCompleted edge-trigger + state-guard |
| b5f386b | cas-4038 | C — WorkerIdle fresh-heartbeat gate |
| 7295489 | cas-b67d | D (core) — supervisor exclusion + stale-epic advice removed |
| 484521c | cas-8c5a | E — shutdown kills PTY process tree |
| 7d82f39 | cas-86c5 | observability — checkpoint label, activity-age line, reset guard |
| fcb380e | cas-a7fa | bonus — 8KB supervisor-guidance trim |
| 98285fb | cas-6c0a | bonus — skill_store hash-id collision |

## Correctness highlights (verified, not assumed)
- **State-guard** (events.rs): `task_completed_announced` collected-then-inserted via an intermediate Vec to release the `&self.last_state` borrow before mutating — correct. Never cleared on reappearance (the oscillation defense). TaskAssigned guard keyed `task_id:assignee` so a genuine reassignment to a *different* worker still fires.
- **Supervisor exclusion**: `is_worker_agent_name` (worker_names only, excludes supervisor) gates the idle loop; placed before `seen_factory_agents.insert` and the tick increment. Does not suppress legit workers.
- **killpg**: relies on PGID==PID, valid because portable_pty `setsid()`s the child pre-exec → whole node→codex tree signaled. `mux.kill_worker` guards on `PaneKind::Worker` so the supervisor pane can't be killed.
- **Reset guard**: only triggers when a matching agent record exists AND heartbeat ≤ WORKER_STALE_SECS(30s); orphaned/stale-assignee resets bypass it (dead-session recovery preserved). Returns `Ok(success)` (non-destructive), not an error.
- **Clock-skew**: heartbeat/activity gates use `age_secs >= 0 && age_secs < THRESHOLD`.

## Advisories (low — non-blocking)
1. **cas-4038** — fresh-heartbeat `continue` resets `consecutive_idle_ticks` but not `idle_already_emitted`, unlike the `current_task.is_some()` / `pending_messages>0` branches. Likely intended (anti-spam: a once-flagged worker that only blips active via the heartbeat path won't re-announce), but the asymmetry deserves a one-line comment.
2. **cas-55dc** — `task_completed_announced` / `task_assigned_announced` grow unbounded for the detector's (per-session) lifetime. Bounded by tasks-per-session in practice; no eviction policy.
3. **cas-8c5a** — `force=false` is SIGTERM to the group but the belt-and-suspenders `child.kill()` still SIGKILLs the *direct* child immediately, so force=false isn't purely graceful for the parent. Plus a low-probability PID-reuse hazard if `process_id()` returned a recycled PID after an external reap (mitigated: the child handle is held, so the PID stays reserved as a zombie until waited).

## Recommendation
Merge as-is. The 3 advisories can ride as follow-up polish if desired; none affect correctness of the shipped behavior. Note `cas-405f` (residual D: phantom-sender/identity/count) and `cas-7e7b` (commit-guard) remain open as tracked follow-ons.
