# Release notes — Factory truth-signals + backlog hardening (2026-07-02)

Deploy target: **Live on production** (cas-src `main`). PR #38.
Two #cas-internal threads. Each = top-level punch + one threaded reply. Was → Now. No ticket labels.

---

## USER thread

**Top-level:**
Live on production · **User** · The factory coordinator used to confidently tell you things that weren't true — now it only reports what's actually real.

**Reply:**
- **"Task completed / closed" notices** — Was: the coordinator announced work as done or closed when zero work existed behind it, and pushed you to assign tasks to workers that had already been shut down (or never existed). Now: those notices only fire on a real, verified completion, and only ever mention workers that are actually alive.
- **Silent stalls** — Was: a worker could freeze mid-task — still "alive," producing nothing — and you'd only catch it by manually checking. Now: a stalled worker is auto-nudged once and, if it stays stuck, surfaced to you with a clear ⚠ STALLED marker.
- **Dangerous merge advice** — Was: finishing an epic suggested "cherry-pick to main," which would drop raw work straight onto production. Now: the coordinator never prescribes a merge strategy at all.
- **Empty "done"** — Was: a worker could mark a task finished having produced no actual change. Now: closing a task with no real changes is rejected.

---

## DEV thread

**Top-level:**
Live on production · **Dev** · Director nudges now source from live authoritative state instead of stale queued events, plus close-path, sync, and branch-base hardening. (PR #38)

**Reply:**
- **Director** — Was: idle/ready/completion nudges derived from cached rosters + queued events, could name shut-down or phantom workers, and hard-coded a "cherry-pick to main" next-step. Now: every nudge gates on the live worker roster + real task-status transitions; `shutdown_workers` evicts from the nudge set; the merge prescription is gone; assignee guidance uses display names.
- **Stall detection** — Was: no signal distinguished alive-but-inactive from healthy. Now: `DirectorEvent::WorkerStalled` (fresh heartbeat + in-progress task + activity past `[factory] stall_threshold_secs`, default 300s) → one-shot nudge, then escalate; `⚠ STALLED` in `worker_status`.
- **Close / sync / storage** — Was: the close guard let zero-diff sync-merge commits through; rule-id generation could collide on `rules.id`; team push shipped one unchunked payload. Now: zero-diff closes rejected; rule-id collision recovery with sequence fast-forward; team push chunked per entity type with per-batch scoping preserved.
- **Branch base + close guidance** — Was: factory branches could anchor to the supervisor's current HEAD; the close gate told workers to open a PR against a local-only epic branch. Now: workers base off the active epic (else trunk); the close gate hands local-only-epic closes a push+supervisor-merge handoff instead of failing PR instructions.
