# BUG: director stall detector contradicts is-wedged — repeated false "stalled, consider shutdown + respawn" advice on live workers

**From:** petra-stella-cloud team (supervisor session, 2026-07-07)
**Severity:** Medium — the suggested remediation (shutdown + respawn) destroys in-flight work if followed; two false alarms in one session
**Component:** director stall detection / worker activity heuristic

## What happened

Twice in one session the director sent the supervisor "Worker hv-query has been stalled on task X for about Nm — alive heartbeat, no activity, and an auto-nudge did not unstick it... consider shutdown + respawn":

1. ~14:33, cas-8a1d, "about 5m" — `cas factory is-wedged hv-query` returned `alive` (transcript mtime age **5s**); the transcript tail showed the worker actively syncing its worktree at that exact moment.
2. ~15:45ish, cas-47f6, "about 8m" — `is-wedged` again returned `alive` (transcript mtime age **7s**, no crash signature); worktree already synced to the epic tip, worker mid-implementation.

Both times the worker was fine and shipped its task shortly after.

## The defect

Two liveness subsystems disagree. `worker_status` / the director track "last activity" from **checkpoint-class events**, and flag `STALLED (no activity ≥300s while task in progress)`. `is-wedged` (the canonical triage from cas-4513) reads **transcript mtime**, which was single-digit seconds old in both incidents. Long think/read stretches between checkpoints — normal for sonnet/high on a heavy task — trip the 300s checkpoint heuristic even though the session is continuously writing its transcript.

The dangerous part is the advice: "consider shutdown + respawn (safe if the worktree is clean)". A clean worktree does NOT mean safe — a worker mid-task before its first commit has a clean worktree and a head full of un-persisted work. The worker-recovery runbook explicitly names "pane looks broken → shutdown_workers" as the anti-pattern that destroyed in-progress work before (silent-owl-56, 2026-04-23), and the director's message points supervisors straight at it.

## Suggested fixes

1. **Make the stall detector consult transcript mtime before alerting** — same signal is-wedged uses. `checkpoint age > 300s AND transcript mtime > 60s` is a much better predicate than checkpoint age alone.
2. **Change the advice text**: point at the triage triad (`cas factory is-wedged <worker>` → classify → only kill on `wedged`/`dead`), never directly at shutdown + respawn, and drop "safe if the worktree is clean" (it isn't — uncommitted work isn't the only loss; a clean worktree with an active session still means losing all in-flight context).
3. Optionally suppress repeat stall alerts for a worker whose is-wedged classification was `alive` within the last N minutes.

## Related reports (same session)

- BUG-stale-message-sequencing-2026-07-07.md (same theme: notifications asserting state without checking ground truth at delivery time)
