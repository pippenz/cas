# Codex live communication conformance probe

Task: `cas-13fa`  
Epic: `cas-04a6`  
Worker: `comm-codex` (`codex-comm-codex-e0786a41-ecb9-4aee-8a4a-d80048d369d5`)  
Supervisor: `quick-finch-86` (`codex-quick-finch-86-914029b6-8948-4a2f-8633-c071f997b77e`)  
Factory session: `cas-src-patient-tiger-71`  
Report timestamp: 2026-07-21T14:52:07Z

## Evidence Surfaces

- Codex rollout: `/home/pippenz/.codex/sessions/2026/07/21/rollout-2026-07-21T10-45-58-019f8524-0511-7383-b0e2-878c44cfcfd8.jsonl`
- CAS daemon log: `/home/pippenz/Petrastella/cas-src/.cas/logs/cas-2026-07-21.log`
- CAS database, read-only: `sqlite3 'file:/home/pippenz/Petrastella/cas-src/.cas/cas.db?mode=ro'`
- Primary table used when MCP status did not expose timestamps: `prompt_queue(id, source, target, created_at, processed_at, acked_at, priority, urgent, summary, prompt)`

Read-only DB inspection was necessary because `mcp__cs__coordination action=message_status` returns only a status string and does not expose `created_at`, `processed_at`, priority, urgent flag, or target metadata.

Latency terms:

- Transport acceptance latency: tool-call end minus prompt_queue `created_at` when both were visible. For message tool calls this was effectively sub-second in the rollout.
- Queue processing latency: `processed_at - created_at`.
- Model reaction latency: first visible rollout event after receipt minus the relevant receipt/processing timestamp.

## Summary

Urgent same-harness delivery to this Codex worker passed. Normal supervisor-to-Codex prompt queue delivery did not process during the observation window, including the startup first-contact row and a later resume row. Worker-to-supervisor normal messages were accepted into `prompt_queue` but remained pending/unprocessed; one worker-to-supervisor urgent message was processed and `message_status` returned delivered.

## Trial Rows

| Scenario | Direction | IDs | Enqueued UTC | Processed / Received UTC | First Reaction UTC | Transport vs Reaction | Status | Evidence |
|---|---|---:|---|---|---|---|---|---|
| Startup harness turn | director to worker | rollout only | 2026-07-21T14:46:00.351Z | 2026-07-21T14:46:00.351Z user_message | 2026-07-21T14:46:06.211Z | reaction 5.860s after user_message | PASS | rollout user_message startup instructions |
| Startup first-contact via prompt_queue | supervisor to worker | 3603 | 2026-07-21T14:46:27.676199920Z | none by 2026-07-21T14:52:07Z | none | queued but no turn | FAIL | `prompt_queue` row 3603 unprocessed |
| Assignment harness turn | director to worker | rollout only | 2026-07-21T14:46:18.570Z | 2026-07-21T14:46:18.570Z user_message | 2026-07-21T14:46:23.543Z | reaction 4.973s after user_message | PASS | rollout user_message task assignment |
| Worker ready message | worker to supervisor | 3600 | 2026-07-21T14:46:18.520057610Z | none by status check | none visible to worker | accepted, pending downstream | FAIL | `message_status(3600)` pending; DB row unprocessed |
| Worker ACK plan | worker to supervisor | 3604 | 2026-07-21T14:46:32.609561088Z | none by 2026-07-21T14:47:47Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3604)` pending |
| Worker-to-supervisor normal 01 | worker to supervisor | 3612 | 2026-07-21T14:48:08.493912115Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3612)` pending |
| Worker-to-supervisor normal 02 | worker to supervisor | 3613 | 2026-07-21T14:48:11.636450278Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3613)` pending |
| Worker-to-supervisor normal 03 | worker to supervisor | 3614 | 2026-07-21T14:48:14.565651840Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3614)` pending |
| Worker-to-supervisor normal 04 | worker to supervisor | 3615 | 2026-07-21T14:48:17.386005307Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3615)` pending |
| Worker-to-supervisor normal 05 | worker to supervisor | 3621 | 2026-07-21T14:48:20.201236136Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3621)` pending |
| Worker-to-supervisor normal 06 | worker to supervisor | 3622 | 2026-07-21T14:48:22.727615691Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3622)` pending |
| Worker-to-supervisor normal 07 | worker to supervisor | 3623 | 2026-07-21T14:48:25.309957953Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3623)` pending |
| Worker-to-supervisor normal 08 | worker to supervisor | 3624 | 2026-07-21T14:48:29.713096122Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3624)` pending |
| Worker-to-supervisor normal 09 | worker to supervisor | 3625 | 2026-07-21T14:48:32.418864297Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3625)` pending |
| Worker-to-supervisor normal 10 | worker to supervisor | 3626 | 2026-07-21T14:48:37.191546570Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3626)` pending |
| Request supervisor inbound trials | worker to supervisor | 3628 | 2026-07-21T14:49:16.530163891Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | `message_status(3628)` pending |
| Worker-to-supervisor urgent | worker to supervisor | 3631 | 2026-07-21T14:50:31.503578834Z | 2026-07-21T14:50:32.722674547Z | none visible to worker | queue processing 1.219s | PASS | `message_status(3631)` delivered; DB urgent=1 priority=0 |
| Supervisor-to-worker urgent X-U1 | supervisor to worker | 3632 | 2026-07-21T14:50:41.586016642Z | processed 2026-07-21T14:50:42.792472121Z; rollout user_message 2026-07-21T14:50:44.392Z | 2026-07-21T14:50:57.439Z | queue processing 1.206s; receipt 1.600s after processed; reaction 13.047s after receipt | PASS | rollout shows `URGENT-PROBE X-U1`; outbound ACK 3643 |
| ACK X-U1 | worker to supervisor | 3643 | 2026-07-21T14:51:03.302817851Z | none by 2026-07-21T14:52:07Z | none visible to worker | accepted, pending downstream | FAIL | DB row unprocessed |
| Idle/status nudge via harness | director to worker | rollout only | 2026-07-21T14:51:39.829Z | 2026-07-21T14:51:39.829Z user_message | 2026-07-21T14:51:45.529Z | reaction 5.700s after user_message | PASS | rollout user_message "gone quiet" |
| Idle-wake via prompt_queue X-N1 | supervisor to worker | 3646 | 2026-07-21T14:52:03.118549345Z | none by 2026-07-21T14:52:07Z | none | queued but no turn | FAIL | `prompt_queue` row 3646 unprocessed |

## Required Scenarios

| Required scenario | Result | Notes |
|---|---|---|
| Startup first-contact | MIXED | Harness startup and assignment arrived as user turns. Same-harness prompt_queue row 3603 did not process. |
| 10 normal supervisor to worker messages | BLOCKED | Requested via outbound row 3628, but that request stayed pending and no S2C-01 through S2C-10 rows appeared for `target='comm-codex'` by the evidence sweep. Missing prerequisite: supervisor-originated normal trial messages. |
| 10 worker to supervisor acknowledgements/status messages | FAIL | 10/10 attempted and accepted as rows 3612, 3613, 3614, 3615, 3621, 3622, 3623, 3624, 3625, 3626. All remained unprocessed/pending in the observation window. |
| FIFO burst | BLOCKED | No supervisor-originated FIFO-A/FIFO-B/FIFO-C rows appeared for `comm-codex`. |
| Idle wake | MIXED | Harness nudge at 14:51:39Z woke a turn. Same-harness row 3646 did not process. |
| Busy urgent redirect | PASS | X-U1 row 3632 interrupted an active/aborted turn path and created a fresh Codex user turn. |
| Duplicate/outbox replay observation | BLOCKED | No duplicate Codex inbound rows were available. Outbound normal queue IDs were unique but not contiguous because other workers enqueued between them. No duplicate delivery was observed. |
| Context compaction/restart if safely observable | BLOCKED | No safe compaction or restart occurred during the trial. Forcing one would exceed the report-only scope. |
| Task close/status transitions | PARTIAL | `task start` and progress notes succeeded. Task close is performed after this report is committed; blocker transition was not exercised because it would inaccurately mutate task state. |
| Malformed/unknown-target negative path | PASS | `coordination message target=cas` failed with `Workers can only message their supervisor. Use target='supervisor' or 'quick-finch-86'`. |

## Minimal Repros

### Normal supervisor-to-Codex row stays unprocessed

1. Supervisor enqueues a normal message to `comm-codex`.
2. Observe `prompt_queue`:
   `select id, created_at, processed_at, urgent, summary from prompt_queue where target='comm-codex' order by id;`
3. Rows 3603 and 3646 have `processed_at = null` and did not appear as Codex rollout `user_message` entries.

Concrete rows:

- 3603: created 2026-07-21T14:46:27.676199920Z, urgent=0, summary `cas-13fa Codex probe`, processed_at null.
- 3646: created 2026-07-21T14:52:03.118549345Z, urgent=0, summary `cas-13fa resume after X-U1`, processed_at null.

### Normal worker-to-supervisor rows stay pending

1. Worker sends 10 normal coordination messages to `target='supervisor'`.
2. Query `message_status` for each returned notification ID.
3. IDs 3612, 3613, 3614, 3615, 3621, 3622, 3623, 3624, 3625, 3626 all returned pending and had no `processed_at` in read-only DB.

### Urgent supervisor-to-Codex delivery succeeds

1. Supervisor sends urgent message `URGENT-PROBE X-U1` to `comm-codex`.
2. DB row 3632 records `created_at=2026-07-21T14:50:41.586016642Z`, `processed_at=2026-07-21T14:50:42.792472121Z`, `priority=0`, `urgent=1`.
3. Codex rollout records the user turn at 2026-07-21T14:50:44.392Z.
4. First visible reaction was 2026-07-21T14:50:57.439Z; worker ACK queued as row 3643 at 2026-07-21T14:51:03.302817851Z.

### Unknown target is rejected

1. Worker calls `mcp__cs__coordination action=message target=cas`.
2. MCP returns `-32600: Workers can only message their supervisor. Use target='supervisor' or 'quick-finch-86'`.

## Conclusion

Current Codex-supervised same-harness communication is not conformant for normal queued supervisor/worker traffic in this trial. The urgent path worked in both directions that were exercised, and the harness-level injected user turns woke the model, but normal `prompt_queue` delivery to `comm-codex` and normal worker-to-supervisor messages remained pending without processing.
