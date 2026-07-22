# 2026-07-22 — Comms observability truth fixes — #cas-internal drafts

## Post 1 — User

Factory monitoring used to gaslight you: delivered messages showed as "still waiting", a rejected close was logged as if the task had closed, and a worker busily posting progress notes could still get flagged as stalled. Now the records tell the truth.

- Message delivery status is internally consistent again — a message the system delivered shows as delivered, including messages from before the new delivery telemetry existed. No more cross-checking raw logs to find out what actually happened.
- The task lease history now says *why* a worker's claim was released — "parked awaiting merge", "reset", "transferred", "shutdown cleanup" — instead of stamping everything "Task closed", even when the close was actually rejected.
- Posting a progress note now counts as being alive. Workers that communicate steadily while thinking through a long stretch no longer trip the "stalled, consider interrupting" alarm — which means fewer healthy runs get interrupted by a worried operator.

## Post 2 — Dev

The delivery/lease/stall records could contradict the logs; now the three biggest sources of false signals are closed at the store layer.

- `message_status` historical contradiction: the lifecycle columns (`highest_stage`, `transport_delivered_at`) were added without backfill, so pre-telemetry rows reported `legacy_status: Delivered` + `stage: enqueued` + `pending_reason: awaiting_delivery` simultaneously. `init()` now backfills `highest_stage='delivered'` / `transport_delivered_at=processed_at` — gated to the one-time column-creation migration only, so live legacy paths (`queue poll`/`ack`, which set `processed_at` without stamping transport delivery) can never be silently promoted to a fabricated "delivered" on a later open. Regression tests cover both directions.
- `release_lease_for_task` took no reason and hardcoded `"Task closed"` into lease history for every release — including MERGE-REQUIRED rejections. It now takes a `reason` threaded through the `AgentStore` trait and all call sites (awaiting-merge park, verification timeout, supervisor-review queue, reset, force-transfer, worker shutdown, preassign abort, wedged recovery, actual close).
- Director stall detection now counts task notes as activity end-to-end: `task action=notes` records an `EventType::TaskNoteAdded` event with the caller's session (non-fatal if the event store hiccups), and the director's worker-activity whitelist includes it. Integration tests exercise the real handler — including a forced event-insert failure — not a hand-built event.
