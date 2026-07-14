---
from: Ozer supervisor (pippenz @ /home/pippenz/Petrastella/ozer)
date: 2026-07-14
priority: P2
---

# BUG: `task action=close` on an already-Closed task succeeds silently — no already-closed detection

## Summary

Two supervising sessions (director + supervisor) raced to close epic `cas-ea3e`. The director closed it at 14:04. My close at 14:05 **also returned success** ("Closed task: cas-ea3e - … (verification skipped — orphaned task, no assignee)") with no indication the task was already Closed. The task now carries two consecutive `Closed:` notes with different reason texts, and both sessions believe *they* performed the close.

Additionally, my close attempt was first rejected with `CODE_REVIEW_REQUIRED` — meaning the review gate ran its full check against a task that was (very likely, given timestamps) already Closed, instead of short-circuiting with "already closed".

## Environment

- `cas 2.27.0 (dd8bcbd-dirty 2026-07-11)`, factory mode, role supervisor (`fast-kestrel-14`), session `07275a32-c0d5-4695-abbb-5c04663df721`, project `/home/pippenz/Petrastella/ozer`
- Concurrent director session operating on the same epic

## Repro / evidence

1. Session A closes epic `cas-ea3e` (14:04 note: "Closed: All subtasks complete and merged. …").
2. Session B calls `task action=close id=cas-ea3e` at 14:05 → first gets `CODE_REVIEW_REQUIRED`, then with an envelope gets **success**: `Closed task: cas-ea3e - HOTFIX: … (verification skipped — orphaned task, no assignee)`.
3. `task action=show id=cas-ea3e` now shows both 14:04 and 14:05 `Closed:` notes; `Closed: 2026-07-14 14:05` (the second close overwrote the close timestamp).

## Expected

- Close on a task whose status is already `Closed` should return a distinct non-destructive result ("already closed at <ts> by <agent>; note appended" or an outright rejection), never a plain success that implies this call performed the close.
- The close timestamp/audit fields should not be silently overwritten by a second close.
- The `CODE_REVIEW_REQUIRED` gate should check status first — demanding a review envelope for an already-closed task wastes a full multi-persona review cycle if the caller obeys.

## Impact

Racing supervisors both get positive confirmation, which corrupts the audit trail (who closed it, when) and hides coordination races that the humans/leads would want to know about. In this instance it was harmless, but the same pattern on a task with side-effectful close hooks (verification spawns, notifications) would double-fire them.
