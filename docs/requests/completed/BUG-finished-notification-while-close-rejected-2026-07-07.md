# BUG: operator receives "finished"-style notification while the task close was rejected and the task is still InProgress

**From:** petra-stella-cloud team (supervisor session, 2026-07-07)
**Severity:** Medium — misleads the human operator about delivery state
**Component:** factory / director notifications, worker idle signaling

## What happened

Timeline (2026-07-07, epic cas-83ec, worker `std-theme`, task cas-4425):

1. Worker completed its implementation, committed c088151, pushed `factory/std-theme`, ran `pnpm build && pnpm test` green.
2. Worker attempted `task action=close id=cas-4425` → **rejected** with MERGE REQUIRED (factory branch not yet merged into the local-only epic branch).
3. Worker messaged the supervisor asking for the merge, then went idle. The director/factory surfaced completion-flavored signals to the operator:
   - teammate message with summary "cas-4425 done, ready to merge into epic"
   - `idle_notification` (`idleReason: "available"`) at 13:38:53
4. **The human operator read this as "finished."** Actual task state at that moment: `Status: InProgress`, close rejected, work unmerged. The operator flagged it: "we just got notified finished but that is incorrect."

## The defect

Notifications shown to the operator conflate **"worker turn ended / worker idle"** with **"task finished."** No completion-style signal should reach the operator while:

- the task's close attempt was rejected (verification gate, merge gate, review gate), and/or
- the task status is not `Closed`.

An operator who trusts the notification and walks away believes work shipped when it is sitting unmerged on a factory branch behind a rejected close.

## Suggested fixes

1. **Gate operator-facing completion notifications on actual task state** — emit "finished" only on a successful `close`; otherwise label the event truthfully: "worker idle — task cas-4425 InProgress, close rejected (MERGE REQUIRED), awaiting supervisor merge".
2. **Include task status in idle notifications** — `idleReason: "available"` should carry `task_state: in_progress, close_rejected: merge_required` so downstream renderers can't mislabel it.
3. **Distinguish worker-lifecycle events from task-lifecycle events** in whatever surface notified the operator — they are different state machines and only the task one means "done".

## Reproduction

Any isolated-mode factory run where the epic branch is local-only: worker finishes, close gate returns MERGE REQUIRED, worker idles → operator gets a completion-flavored notification while the task is InProgress.
