# BUG: coordination messages/notifications deliver stale state — no sequencing, invalidation, or delivery-time state check

**From:** petra-stella-cloud team (supervisor session, 2026-07-07)
**Severity:** Medium — every instance costs a supervisor round-trip to disambiguate; some risk wrong actions (duplicate work, acting on dead state)
**Component:** factory/director notification pipeline + coordination message queue (outbox replay, idle notifications, spawn/assign ordering)

## Symptom class

Messages and notifications arrive carrying state that was true when they were *generated*, not when they were *delivered* — with no marker distinguishing fresh from stale. One supervisor session (epic cas-83ec, cas 2.27.0 / 9ebc844) hit this at least four distinct ways in ~30 minutes:

1. **Assignment/registration race (hit twice).** Supervisor set `task update assignee=<worker>` immediately after `spawn_workers`; workers finished registering *after* the write, their first `task mine` came up empty, and all three idle-pinged "ready and waiting for tasks" while the supervisor believed them dispatched. (13:36–13:38, workers hv-query / std-theme / lt-defects; earlier the same morning with the first fleet.)
2. **Replayed kickoff messages.** hv-query completed cas-6c1f and sent its completion report (13:56), then received replays of its original kickoff/assignment messages (13:58) and had to reply "already done — these look like replayed kickoff msgs" to avoid redoing work. std-theme hit the same earlier ("These two kickoff messages crossed with my earlier completion report").
3. **Stale idle-notification summaries.** Idle notifications repeatedly delivered with summaries describing superseded states: lt-defects pinged "ready for assignment" after being assigned; pinged "unable to invoke close" after the task was closed and its shutdown queued; std-theme pinged "standing by for cas-8b4f (blocked on cas-6c1f)" a minute after cas-6c1f closed and cas-8b4f was assigned to it.
4. **Director notifications for dead workers.** Supervisor received a batch of "Worker X is ready/idle" director messages for three workers that had already been shut down minutes earlier (worker_status confirmed `Workers: None active`).

The supervisor-side cost is real: each instance forces a `task show`/`worker_status` round-trip to decide whether a message is actionable (the workflow guide even codifies this: "check the task's current state — the message may be outdated"). Documenting a defect as a consumer-side discipline is a workaround, not a design.

## Suggested fixes (any subset)

1. **Delivery-time state check for state-describing notifications.** Before delivering "worker idle, no tasks" / "worker ready", re-check: does the worker now have an assignment? is it shut down? Drop or re-render the notification if the claim is no longer true.
2. **Sequence/generation stamps.** Give each agent's outbox a monotonically increasing sequence and stamp messages with the task-state generation they were derived from; consumers (and the TUI) can then mark or drop out-of-order/superseded deliveries instead of guessing from prose.
3. **Outbox replay dedup.** Replayed kickoffs should be recognizable as re-deliveries (same message ID) and either suppressed after ack or delivered with an explicit `redelivery: true` marker.
4. **Fix the assign/registration race at the source:** make `spawn_workers` return only after registration, or make assignment writes to a not-yet-registered worker name queue and re-link on registration (the write currently appears to succeed but doesn't bind for `task mine`).

## Related reports (same session)

- BUG-finished-notification-while-close-rejected-2026-07-07.md (one specific instance of the same class: notification content not validated against task state at delivery)
- BUG-worker-cannot-invoke-task-close-2026-07-07.md
- BUG-spawn-workers-inherits-supervisor-model-2026-07-07.md
