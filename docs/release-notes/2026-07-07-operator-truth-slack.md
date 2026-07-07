# Slack draft — 2026-07-07 operator-truth release (main 2b7c841 + b58b115)

Channel: #cas-internal (C0B44GUKDK2). Two top-level posts per rubric.

## User post

**Live on production — User**
Your factory dashboard now tells the truth. Was: "finished" alerts while work was actually still waiting to merge, workers grinding away while the task panel showed nothing at all, and scary "worker stalled — consider restarting" alarms about workers that were perfectly fine. Now: a task shows up on the board the moment a worker starts it, "done" only ever means done (work that's waiting on a merge says exactly that), stall alarms only fire when something is genuinely stuck, and the panel always shows you something useful — even before you've picked what to focus on.

## Dev post

**Live on production — Dev**
The whole factory notification pipeline is now state-checked at generation AND delivery. Was: idle events rendered as completions, a stale epic-filtered snapshot fabricated `task_completed` events for concurrently-active epics, and notifications delivered state that was minutes dead. Now: idle payloads carry task status + close-rejection reason; change detection reads a never-filtered snapshot; a delivery-time re-check drops or rewrites anything superseded; and the stall detector requires transcript-mtime staleness (the `is-wedged` signal) plus a task-start grace window before alerting — with the "shutdown + respawn" advice replaced by the triage triad. Also in: `task start` sets the assignee (TUI epic adoption now works unsupervised, with retry until adoption), a new `AwaitingMerge` status parks merge-gated closes without blocking the worker's next task, spawn model/effort resolves from the config cascade instead of inheriting the supervisor session (and is visible per-worker in status), messages to not-yet-registered workers deliver on registration with an honest queued-vs-delivered ack, `spawn_workers task_id` actually pre-assigns now (m206), and the multi-persona code review gained a size-gated sharded mode: subsystem shards + a cross-shard interface-integrator pass + risk-weighted persona routing — which caught 3 verified P1s reviewing this very release.
