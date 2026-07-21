# Factory communication conformance spike

Task: `cas-563d`  
Epic: `cas-04a6`  
Date: 2026-07-21  
Status: final synthesis from the merged provider probes, state-machine audit, alternate-supervisor matrix, and urgent re-close deadlock characterization.

## Source Register

| Source | Class | Status in this branch | Use |
|---|---|---|---|
| `docs/reviews/2026-07-21-comm-conformance-grok.md` (`cas-5c02`) | LIVE + source audit | merged | Codex-sup/Grok-worker live rows, HOL diagnosis, cas-126b support |
| `docs/reviews/2026-07-21-comm-conformance-codex.md` (`cas-13fa`) | LIVE | merged | Codex-sup/Codex-worker live rows, urgent X-U1, normal pending rows |
| `docs/reviews/2026-07-21-comm-conformance-claude.md` (`cas-e76b`) | LIVE | merged | Codex-sup/Claude-worker live rows, director_events split, Claude FIFO/duplicate/negative-path rows |
| `docs/reviews/2026-07-21-factory-message-state-machine-audit.md` (`cas-291c`) | SOURCE + design | merged | state machine, blind spots, automation seam |
| `docs/reviews/2026-07-21-alternate-supervisor-communication-matrix.md` (`cas-474b`) | AUTOMATED + STATIC + historical LIVE | merged | alternate-supervisor routing matrix and safe runbooks |
| `docs/reviews/2026-07-21-urgent-merge-reclose-deadlock.md` (`cas-69e1`) | LIVE + SOURCE | merged | urgent-stop x AwaitingMerge deadlock, safe recovery, SRP remediation spec |
| `cas-126b` | OPEN bug | CAS task | umbrella for Grok MERGE DONE / re-close failure |

Evidence classes:

- `LIVE`: observed in a real factory session with message IDs and timestamps.
- `AUTOMATED`: covered by green routing/store tests.
- `SOURCE`: source/state-machine audit, no live model reaction.
- `STATIC`: contract-derived but not test-pinned for that exact shape.
- `UNKNOWN` / `BLOCKED`: not safely exercised or missing merged evidence.

## Executive Decision

Current factory communication is not conformant for normal supervisor/worker coordination under the Codex-supervised `cas-src-patient-tiger-71` session. The dominant failure is not model reaction quality; it is transport starvation before delivery: normal-priority `prompt_queue` rows stay pending while urgent rows jump the queue and deliver in about 1.2 seconds.

Routing contracts are mostly correct and test-pinned for recipient-aware channel selection. End-to-end delivery, wake, reaction, ack, and merge re-close behavior are not yet reliable enough for a "real-time supervisor" SLO.

## 3x3x2 Matrix

Each cell is `down / up`: supervisor-to-worker, then worker-to-supervisor.

| Supervisor | Worker | Down result | Up result | Evidence |
|---|---|---|---|---|
| Codex | Grok | FAIL normal; BLOCKED live urgent-to-Grok | FAIL normal | LIVE `cas-5c02`: S2W 3602 pending >5m; W2S 3605, 3607-3611, 3616-3620 all pending. Urgent same-factory controls 3631/3632 delivered but not targeted to Grok. |
| Codex | Codex | FAIL normal; PASS urgent | FAIL normal; PASS urgent transport | LIVE `cas-13fa`: S2W normal 3603/3646 unprocessed; W2S 3612-3626 pending; urgent 3631/3632 processed in 1.219s/1.206s; X-U1 reaction observed. |
| Codex | Claude | FAIL normal/FIFO; BLOCKED urgent | SEND PASS, CONFIRM FAIL | LIVE `cas-e76b`: S2W contract 3601 never surfaced; scripted S2W 3662-3674 0/13 delivered even across idle wake; W2S 3633-3642 enqueued FIFO but 0/10 processed/acked. |
| Claude | Grok | BLOCKED E2E; routing PASS | BLOCKED E2E; routing PASS | AUTOMATED `cas-474b` SHAPES[4]: Grok recipient uses PTY/gate/no frame; upward Claude supervisor uses TeamsInbox. No safe live alt-sup factory launched. |
| Claude | Codex | BLOCKED E2E; routing PASS | BLOCKED E2E; routing PASS | AUTOMATED `cas-474b` SHAPES[0] and historical `cas-ca04` code-proven/live-pending for Codex legs. |
| Claude | Claude | historical partial PASS | historical partial PASS | AUTOMATED + historical LIVE `cas-ca04`: Claude worker/supervisor path observed as new turns on 2026-06-25; fresh normal/FIFO/urgent suite not re-run. |
| Grok | Grok | BLOCKED E2E; routing PASS | BLOCKED E2E; routing PASS | AUTOMATED `cas-474b` SHAPES[5]; no disposable Grok-sup factory launched. |
| Grok | Codex | BLOCKED E2E; routing STATIC PASS | BLOCKED E2E; routing STATIC PASS | STATIC `cas-474b`: Grok-sup/Codex-worker not in SHAPES; contract says Codex recipient PTY+frame, Grok recipient PTY/no frame. |
| Grok | Claude | BLOCKED E2E; routing STATIC PASS | BLOCKED E2E; routing STATIC PASS | STATIC `cas-474b`: Grok-sup/Claude-worker not in SHAPES; Claude worker is PTY fallback because teams=false. |

## Scenario Rollup

| Scenario | Result | Evidence |
|---|---|---|
| Startup first-contact | MIXED | Director side channel wakes Grok/Codex/Claude; prompt_queue first-contact rows 3598/3601/3603 stayed unprocessed. |
| 10 normal S->W | FAIL/BLOCKED | Codex: missing full S2W series and normal rows unprocessed. Grok: only 3602 sent, pending. Claude: 3662-3671 plus FIFO 3672-3674 0/13 delivered. |
| 10 normal W->S | FAIL transport | Grok and Codex 10/10 enqueued but stayed pending; Claude 10/10 enqueued FIFO but 0/10 processed/acked. |
| FIFO burst | UNKNOWN at inject | Parallel MCP enqueue reorders logical order for Grok; Claude FIFO enqueue order by ID was FB/FC/FA but 0/3 delivered; no injected FIFO order measured. |
| Idle wake | MIXED | Director quiet-nudges wake workers; normal prompt_queue idle rows do not. |
| Busy urgent redirect | PASS transport for Codex legs; UNKNOWN for Grok/Claude live | Codex urgent X-U1 and worker->supervisor urgent 3631 delivered ~1.2s. Grok urgent to worker not sent; Claude urgent intentionally blocked to avoid cas-b269 deadlock. |
| Duplicate/replay | PARTIAL | prompt_queue has no content idempotency; Claude duplicate bodies 3644/3645 got distinct IDs. No duplicate model reaction observed because delivery failed. |
| Unknown/malformed target | PASS fail-closed | Worker non-supervisor/unknown targets return `-32600`; missing summary returns `-32602`. |
| Task close/status transitions | FAIL for push delivery; PASS via CAS task APIs | Starts/notes/closes work through MCP. No supervisor_queue lifecycle push for task start/close; workers rely on explicit messages or polling. |
| MERGE DONE / re-close | FAIL without recovery | `cas-69e1`: urgent wakes Grok/Codex but sets `halt_task_work`; close is refused until a different Open task is started. Linked to open `cas-126b`. |

## Latency Distributions

Small sample; treat as SLO proposal input, not production telemetry.

| Path | Samples | Transport | Wake / reaction |
|---|---:|---|---|
| Director first-contact -> Grok | 1 | inject_ms ~0.13ms; refresh_to_deliver ~33.6ms | first token ~1.736s; first tool ~4.744s |
| Harness startup/assignment -> Codex | 2 | rollout user_message direct | reactions 5.860s and 4.973s |
| Normal prompt_queue under Codex sup | dozens | unbounded in window; >3m to >5m pending | no recipient turn |
| Urgent prompt_queue Codex legs | 2 | 1.206s, 1.219s | X-U1 rollout receipt 1.600s after processed; first reaction 13.047s after receipt |
| Claude director_events lane | 4 | inject ~0.06ms; refresh_to_deliver ~19-34ms | reached only director-channel turns; supervisor prompt_queue rows did not flush |
| Claude supervisor prompt_queue lane | 30+ | pending through cutoffs; 0/13 scripted S2W delivered; 0/15 W2S processed | no model reaction to supervisor messages |
| Urgent MERGE DONE Grok G-M1 | 1 | delivered/woke; prompt_history at 14:55:55.262Z | turn_started +4ms; first token ~4.0s |
| Urgent MERGE DONE Codex X-M1 | 1 | log message_id 3651 delivered in 1.204s | first visible reaction ~6-8s after delivery by task note |

## Proposed SLO Contract

Separate each stage. A single "message queued" response is not a delivery SLO.

| Stage | Proposed SLO | Measurement |
|---|---|---|
| Enqueue acceptance | p99 <= 500ms, error text exact for reject paths | MCP tool duration + queue row created_at |
| Normal transport | p95 <= 2s, max <= 10s for registered ready target | `processed_at - created_at`; fail if pending while daemon ticks advance |
| Urgent transport | p95 <= 2s, max <= 5s including 1200ms settle floor | `processed_at - created_at` and log `deliver_ms` |
| Turn wake | p95 <= 5s after transport delivered | harness adapter: Codex rollout, Grok prompt_history/events, Claude inbox/session |
| First visible reaction | p95 <= 15s after wake for non-tool-blocked turns | harness first token/tool/commentary timestamp |
| FIFO | serial sends preserve enqueue and delivery order per target/priority | id/created_at order and transcript order |
| Duplicate handling | identical replay either suppressed with explicit dedup event or delivered with `redelivery=true` | queue id + ack/redelivery metadata |
| Status/blocker/ready to supervisor | delivered <= 5s without supervisor polling | worker->supervisor row processed + supervisor wake artifact |
| Merge re-close | after merge notification, worker close attempt <= 30s and succeeds if merge gate is green | task notes/MCP close result; no urgent halt deadlock |

## Root-Cause Clusters

| Cluster | Finding | Evidence |
|---|---|---|
| R1: normal queue HOL | `peek_for_targets` mixes legacy NULL-session rows with live session rows and `LIMIT 10`; normal priority rows starve | Grok report: ~45 lower-id/equal-priority rows ahead of patient-tiger; urgent priority 0 bypasses. State audit B1. |
| R2: observability conflates queued with pending forever | `message_status=pending` hides whether a row was never peeked, gated, skipped, or undelivered | Codex/Grok/Claude normal rows all pending; state audit B2/B3/B11. |
| R3: CAS core is blind to turn wake/reaction | `processed_at` is only transport; model wake lives in harness files | state audit TURN_WAKE/REACTED external; reports manually parse rollout/prompt_history/task notes. |
| R4: director_events bypasses prompt_queue | Startup/idle nudges can work while prompt_queue is broken, masking transport failure | Grok/Codex/Claude all woke through director/harness messages while normal queue rows remained pending; Claude report proves director_events woke but supervisor rows 3601/3662-3674/3681/3682 did not. |
| R5: urgent halt composes badly with AwaitingMerge | urgent MERGE DONE wakes the worker and then blocks the required close | `cas-69e1` G-M1/X-M1; linked open `cas-126b`. |
| R6: routing tests are incomplete for Grok-sup mixed workers | Grok-sup/Codex and Grok-sup/Claude are STATIC, not AUTOMATED | `cas-474b` SHAPES gap. |
| R7: no durable replay/ack contract | prompt_queue inserts duplicate bodies with distinct IDs; `message_ack` is rarely used | Claude 3644/3645; state audit B5/B8. |

## Remediation Backlog

Non-duplicative recommendations; link existing tasks where possible.

| Priority | Unit | Scope | Links / dedup |
|---|---|---|---|
| P0 | Fix normal queue HOL for live sessions | Change `peek_for_targets(session)` so legacy NULL-session rows cannot permanently occupy `LIMIT 10` ahead of session-tagged rows; add store tests with lower-id NULL rows and live session rows. | New task; references R1, `prompt_queue_store.rs`, `cas-5c02`, `cas-291c`. |
| P0 | Add a merge re-close halt exemption or close-time exception | Urgent MERGE DONE / re-close should interrupt without blocking the AwaitingMerge close it requests. Prefer `cas-69e1` Option A halt-exempt re-close urgents. | Do not duplicate; attach to open `cas-126b` or make a child linked to `cas-126b`. |
| P1 | Promote delivery status from string to state ladder | Expose Enqueued/Peeked/Gated/Delivered/Woke/Reacted/Confirmed or at least pending reason + queue-head diagnosis. | New task; builds on `cas-f9e8` debug telemetry and state audit B2/B3/B11. |
| P1 | Repair Claude supervisor-message bridge | Ensure supervisor `prompt_queue` messages to Claude workers reach the same worker turn surface as director_events or are explicitly marked unsupported; update `processed_at` semantics to distinguish inbox write from model wake. | New task; references `cas-e76b` CF-1/CF-2/CF-3. |
| P1 | Build `cas factory probe-comm` core + adapters | Core emits messages, polls DB/status/logs; adapters parse Claude/Codex/Grok wake artifacts; outputs JSONL evidence bundle. | New task; implementation design from state audit §8. |
| P1 | Extend delivery matrix SHAPES | Add Grok-sup/Codex-worker and Grok-sup/Claude-worker to automated routing matrix. | Small test-only task from `cas-474b`; no live factory required. |
| P1 | Wire task lifecycle events to supervisor push path | Task start/close/blocker/ready should produce delivery-verifiable supervisor events, not depend on free-form worker messages or polling. | New task; avoid duplicating completed stale-message docs by requiring delivery-time state validation. |
| P2 | Add prompt_queue id -> harness turn correlation | Store transcript/turn identifier or payload hash on delivery so evidence does not rely on substring/time joins. | New task; state audit missing correlation key. |
| P2 | Make duplicate/replay behavior explicit | Add idempotency key or `redelivery=true`; suppress or mark replay in both prompt_queue and Claude inbox paths. | Related completed docs: `BUG-stale-message-sequencing-2026-07-07.md`, `BUG-coordination-message-duplicate-delivery-and-error-styled-success-2026-07-14.md`; new work should cite them. |
| P2 | Harden Codex/Grok liveness diagnostics | Resolve actual child PIDs and transcript paths so "starved" does not fire during active turns. | Completed doc exists for Codex false-starved; only create follow-up if current code still lacks Grok/Codex coverage. |

## Automation Design

Implement one conformance runner with stage assertions and harness adapters.

Core responsibilities:

1. Emit normal and urgent MCP coordination messages.
2. Capture returned message IDs and queue rows.
3. Poll `message_status`, `prompt_queue`, and daemon logs.
4. Fail pending rows with queue-head diagnostics.
5. Emit a JSONL evidence bundle per trial.

Adapters:

- Claude: TeamsInbox/session JSONL; distinguish inbox write from turn wake.
- Codex: rollout `user_message`, framing check for `Message from`.
- Grok: `prompt_history.jsonl`, `events.jsonl`, first_token/tool timing.

Required automated suites:

- Store: legacy NULL-session HOL, session isolation, urgent priority bypass.
- Routing: all 9 supervisor/worker pairings x 2 directions.
- Lifecycle: AwaitingMerge + urgent MERGE DONE does not block close.
- Replay: duplicate body gets deterministic suppress/mark behavior.

## Rollout / Recovery Guidance

Until fixes land:

1. Do not treat `Message queued` or `message_status=pending` as delivery.
2. For merge re-close, avoid urgent MERGE DONE unless the halt exemption exists. If urgent was already sent, assign a legitimate next Open task so `task start` clears halt, then re-close the AwaitingMerge task.
3. Do not kill or force-reset responsive workers solely for "starved" alerts; inspect transcripts/PIDs/branch movement first.
4. For live conformance, use an isolated disposable `CAS_DIR` or throwaway project copy; do not launch nested factories against the parent session DB.
5. Use read-only DB inspection when MCP status lacks timestamps, and record why.

## Open UNKNOWNs

| Unknown | Needed evidence |
|---|---|
| Grok urgent-to-worker live behavior | Send urgent to Grok worker in disposable factory; current run has urgent controls only on Codex legs and G-M1 merge path. |
| Alternate-supervisor LIVE E2E | Run disposable Grok-sup and Claude-sup factories with isolated CAS roots. |
| FIFO after transport fix | Re-run serial and burst trials after HOL is fixed; current failures happen before inject. |
| Claude urgent mid-turn | Run only after urgent halt/re-close safety is fixed or in a disposable factory; cas-e76b intentionally did not send C-URG. |

## Decision

The communication contract should be stage-based and evidence-driven: enqueue, transport delivery, turn wake, model reaction, and confirmation are separate claims. Today, normal `prompt_queue` traffic under the Codex-supervised live factory fails before transport delivery, while urgent traffic proves the PTY interrupt route can work but exposes a separate urgent-halt lifecycle deadlock. The next engineering work should fix queue HOL first, then the merge re-close halt composition, then add the conformance runner so future harness changes cannot regress silently.
