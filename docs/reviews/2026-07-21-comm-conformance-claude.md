# Claude Live Communication Conformance Probe — cas-e76b

- **Worker (subject + investigator):** `comm-claude` (agent id `12ce6226-2bc3-4df3-8d7a-e4576ed62221`), a Claude Code agent-teams worker.
- **Supervisor:** `quick-finch-86` — a **Codex** primary/supervisor.
- **Factory session:** `cas-src-patient-tiger-71`. Siblings running the same probe: `comm-codex` (cas-13fa), `comm-grok` (cas-5c02).
- **Window:** 2026-07-21 14:45Z → 15:03Z (UTC). All timestamps RFC3339 UTC.
- **Scope:** additive-only. This report is the ONLY artifact created. No production/test/config/skill/doc edits.
- **Evidence access:** coordination MCP (`message`, `message_status`, `queue_peek`), read-only `SELECT` on `.cas/cas.db` (APIs expose only a 3-state enum, not timestamps, so read-only DB was required for latency), and the factory daemon log `.cas/logs/cas-2026-07-21.log`.

---

## Verdict (one line per scenario)

| # | Scenario | Direction | Result | Key evidence |
|---|----------|-----------|--------|--------------|
| C-S0 | Startup first-contact | S→W | **FAIL (CF-1)** | msg 3601 body never surfaced in worker turn; only generic director template arrived |
| C-N01..10 | 10 normal supervisor→worker | S→W | **FAIL (CF-3)** | ids 3662–3671 enqueued 14:59:18Z; **0/10 delivered** at 15:03:13Z cutoff and still 0/10 at 15:06:33Z; never surfaced to worker turn |
| C-WS01..10 | 10 normal worker→supervisor | W→S | **SEND PASS; CONFIRM FAIL (CF-2)** | ids 3633–3642 enqueued, FIFO-ordered; 0/10 `processed_at`/`acked_at`; supervisor never consumed |
| C-ACK/status | worker→supervisor acks/status | W→S | **SEND PASS; CONFIRM FAIL** | ids 3606, 3630, 3655, 3683 enqueued; never consumed |
| C-FB/FC/FA | FIFO burst | S→W | **ORDER PASS (enqueue); DELIVERY FAIL** | ids 3672/3673/3674 monotonic by ID; **0/3 delivered** — delivery-order unverifiable because none delivered |
| C-IDLE | Idle wake | S→W | **SPLIT: director PASS / supervisor FAIL (CF-3)** | idle-wake fired via `director_events` (4 wakes) — director nudges reached my turn; but supervisor `prompt_queue` messages (3601, burst, 3681, 3682) never surfaced across the same idle boundaries |
| C-URG | Busy urgent mid-turn redirect | S→W | **BLOCKED (see cas-5c02 urgent-halt deadlock)** | supervisor sent no `urgent=1` to comm-claude in window; sibling cas-5c02 (comm-grok) hit an urgent-halt deadlock, so the interrupt leg is independently suspect |
| C-DUP | Duplicate / replay | W→S | **PASS (no dedup observed)** | identical bodies → distinct ids 3644/3645 |
| C-NEG | Malformed / unknown-target | W→S | **PASS (fail-closed)** | -32600 peer/unknown target; -32602 missing summary |
| C-TASK | Task close/status transitions | both | **PASS w/ finding (CF-4)** | start recorded in task store; no `supervisor_queue` push event |

**Headline:** In this Codex-supervised factory the Claude worker is effectively **comms-degraded**. Message *enqueue* (send) works and preserves FIFO order in both directions, and negative paths fail-closed correctly. But **every message touching `comm-claude` — 2 inbound + 15 outbound — has `processed_at = NULL` and `acked_at = NULL`**, while sibling `comm-codex` and `comm-grok` traffic is transport-confirmed in ~1.2 s both ways. The Claude worker receives supervisor traffic through a **separate `director_events` inbox bridge** that (a) is decoupled from `prompt_queue` bookkeeping, (b) fires on a sparse refresh cadence rather than per-message, and (c) only surfaces messages at a **turn boundary**, so a worker in a long turn does not react. `message_status` therefore reports a permanent, misleading `Pending` for the Claude worker.

---

## Delivery model (mechanism, verified from code + daemon log)

Send path (`cas-cli/src/mcp/tools/service/agent_search_system/message.rs::message_send`, harness-agnostic):
`message` → worker-only target guard → `prompt_queue.enqueue_urgent` (`created_at`) → `notify_daemon` best-effort wake.

Transport states (`crates/cas-store/src/prompt_queue_store.rs::message_status`):
- `processed_at IS NULL` → **Pending**
- `processed_at` set, `acked_at` NULL → **Delivered**
- `acked_at` set → **Confirmed**

Two distinct delivery bridges observed in this session:
1. **prompt_queue processed-marking** — used for `comm-codex` and `comm-grok` (both directions), ~1.2 s after enqueue. Never used for `comm-claude`.
2. **`director_events` inbox injection** (daemon log `cas::coordination … channel="director_events" target_agent=comm-claude`) — the Claude agent-teams path. `inject_ms` ≈ 0.06 ms, `refresh_to_deliver_ms` ≈ 19–34 ms, but it fired only 3× in ~17 min (14:46:16, 14:58:25, 15:02:59Z) and does **not** update `prompt_queue.processed_at`.

Consequence: for a Claude worker, `prompt_queue` timestamps are not a valid transport signal, and the real delivery latency to the model is dominated by (sparse refresh cadence) + (time to next idle/turn boundary), not the ~20 ms inject.

---

## W→S trials (worker → supervisor)

All enqueued via `coordination action=message target=quick-finch-86`. Enqueue returns immediately ("Message queued, ID …").

| Trial | msg id | created_at (Z) | processed_at | acked_at | Result |
|-------|--------|----------------|--------------|----------|--------|
| ACK | 3606 | 14:47:10.084 | — | — | send ok / never consumed |
| READY-PROBE | 3630 | 14:50:18.073 | — | — | send ok / never consumed |
| C-WS01 | 3633 | 14:50:45.526 | — | — | send ok / never consumed |
| C-WS02 | 3634 | 14:50:46.528 | — | — | " |
| C-WS03 | 3635 | 14:50:48.190 | — | — | " |
| C-WS04 | 3636 | 14:50:49.026 | — | — | " |
| C-WS05 | 3637 | 14:50:50.520 | — | — | " |
| C-WS06 | 3638 | 14:50:51.525 | — | — | " |
| C-WS07 | 3639 | 14:50:53.025 | — | — | " |
| C-WS08 | 3640 | 14:50:54.080 | — | — | " |
| C-WS09 | 3641 | 14:50:55.531 | — | — | " |
| C-WS10 | 3642 | 14:50:56.423 | — | — | " |
| C-DUP-1 | 3644 | 14:51:04.377 | — | — | duplicate test |
| C-DUP-2 | 3645 | 14:51:05.371 | — | — | duplicate test |
| status/nudge | 3655 | 14:57:20.061 | — | — | send ok / never consumed |

- **Send:** 10/10 (+5 control) attempted and enqueued. IDs are strictly monotonic ⇒ FIFO enqueue order preserved.
- **Transport delivery / confirmation:** 0/15. `message_status(id)` returns `Pending` for all as of 15:03Z. Contrast: sibling `comm-codex→super` (3631) and `comm-grok→super` (3648/3652/3658) were `processed_at` within ~1.2 s. The Codex supervisor consumed sibling messages but **none** from `comm-claude`.
- **Model reaction (supervisor acting on my messages):** No reply, no `notes`, no consumption signal observed in-window ⇒ **unmeasurable / negative**. Transport-vs-reaction cannot be separated on this leg because transport confirmation itself never occurred.

## S→W trials (supervisor → worker)

Supervisor fired the scripted set as a burst at **14:59:18Z** (after two nudges).

| Trial | msg id | created_at (Z) | urgent | processed_at | Surfaced to my turn? |
|-------|--------|----------------|--------|--------------|----------------------|
| C-N01 | 3662 | 14:59:18.258 | 0 | — | no (as of 15:03Z) |
| C-N02 | 3663 | 14:59:18.280 | 0 | — | no |
| C-N03 | 3664 | 14:59:18.286 | 0 | — | no |
| C-N04 | 3665 | 14:59:18.293 | 0 | — | no |
| C-N05 | 3666 | 14:59:18.298 | 0 | — | no |
| C-N06 | 3667 | 14:59:18.303 | 0 | — | no |
| C-N07 | 3668 | 14:59:18.309 | 0 | — | no |
| C-N08 | 3669 | 14:59:18.314 | 0 | — | no |
| C-N09 | 3670 | 14:59:18.320 | 0 | — | no |
| C-N10 | 3671 | 14:59:18.324 | 0 | — | no |
| C-FB (FIFO) | 3672 | 14:59:18.330 | 0 | — | no |
| C-FC (FIFO) | 3673 | 14:59:18.334 | 0 | — | no |
| C-FA (FIFO) | 3674 | 14:59:18.339 | 0 | — | no |

- **Enqueue:** 10/10 normal + 3 FIFO received into `prompt_queue`, IDs monotonic.
- **FIFO test design:** tokens deliberately scrambled (enqueue order = FB, FC, FA at ids 3672<3673<3674). Enqueue order is correct by ID; delivery order to the model is **not yet verifiable** because none have surfaced.
- **Transport delivery to Claude inbox:** **0/13**. The scripted burst never appears in the `director_events` delivery log; the four `director_events` deliveries to `comm-claude` (14:46:16, 14:58:25, 15:02:59, 15:04:14Z) carried only *director-channel* traffic (spawn/assignment template + quiet-check nudges), never the supervisor's `prompt_queue` coordination messages. Supervisor declared an evidence cutoff at 15:03:13Z with delivery still 0/13; re-confirmed 0/13 at 15:06:33Z.
- **Model reaction:** **0/13**. Two later supervisor coordination messages — 3681 (15:03:22Z "Supervisor triage…") and 3682 (15:04:41Z "Evidence cutoff reached…", the completion order) — *also* never surfaced to my turn and were recovered only by DB read. The idle boundary that woke me (via director nudges) did **not** flush any supervisor `prompt_queue` message. This is the decisive split: **director_events messages reach the Claude worker's turn; supervisor coordination-channel (`prompt_queue`) messages do not.** Every substantive supervisor instruction this session (contract 3601, burst 3662–3674, triage 3681, cutoff 3682) was learned only via read-only DB inspection, not delivery.

---

## Candidate findings

- **CF-1 — Supervisor first-contact body not surfaced to the Claude worker.** msg 3601 (`source=supervisor`, created 14:46:27Z) carried the real contract ("acknowledge by messaging exactly `READY-PROBE CLAUDE <RFC3339 UTC>` …"). Its body never appeared in the worker's turn; the worker saw only the generic `director` task-assignment template (delivered via `director_events` at 14:46:16Z, *before* 3601 existed). The instruction was recovered only by a read-only DB `SELECT`. Repro below.
- **CF-2 — `prompt_queue` transport confirmation dead for the Claude worker.** 17 comm-claude-touching rows this session (2 inbound 3599/3601 + 15 outbound) all `processed_at=NULL/acked_at=NULL`, while every `comm-codex`/`comm-grok` row is confirmed ~1.2 s. `message_status` is a false `Pending` signal for the Claude worker — dangerous because supervisor recovery playbooks key on `processed_at`/`acked_at` (see `worker-recovery.md`).
- **CF-3 — Supervisor `prompt_queue` coordination messages are never delivered to the Claude worker; only `director_events` messages are.** Across the whole session the Claude worker received exactly 4 `director_events` injections (assignment template + quiet-check nudges) and reacted to them, but **0** of the supervisor's 15 `prompt_queue` messages (contract 3601, burst 3662–3674, triage 3681, cutoff 3682) ever surfaced in a turn — including across the idle boundary that woke the worker at ~15:04Z. `director_events` inject is fast (~20–34 ms) when it fires; the supervisor coordination channel simply does not reach the Claude worker at all. This is the load-bearing conformance gap: a Claude worker under this factory cannot receive scripted supervisor messages, only director/team-lifecycle nudges. (Distinct from and compounding CF-2, which is the missing transport bookkeeping.)
- **CF-4 — No task-lifecycle push.** `supervisor_queue` recorded no `task_started`/`task_completed`/`task_blocked` for cas-e76b (only stale `reminder_fired`/`worker_died`). Task transitions are observable only via task-store poll or explicit worker message.
- **Observation — no send-side idempotency.** Identical bodies enqueue as distinct ids (3644/3645); a replay would deliver twice.

## Positive conformance (what works)

- Send/enqueue in both directions: immediate, monotonic IDs, FIFO enqueue order preserved (W→S 3633–3642; S→W 3662–3674).
- Worker→supervisor target guard fail-closed: a worker may message only its supervisor; peer target and unknown target both rejected `-32600`; missing `summary` rejected `-32602`.
- Sibling (Codex/Grok) bidirectional transport healthy at ~1.2 s — proves the daemon delivery+mark path itself is functional this session; the gap is specific to the `comm-claude` lane.

## Mapping to Claude-supervisor paths

`message_send` and the `prompt_queue` schema are harness-agnostic (shared Rust), so a Claude *supervisor* would enqueue identically. The divergence is entirely on the **recipient** side: a Claude *worker* is fed by the `director_events` bridge. Therefore CF-1/CF-2/CF-3 are properties of Claude-worker delivery and would reproduce under a Claude supervisor for any Claude worker; they are not caused by the supervisor being Codex. CF-4 (no lifecycle push) is likewise shared. The one leg that is supervisor-harness-specific is *supervisor consumption* of W→S messages (CF-2 W→S half): here the Codex supervisor never marked/consumed comm-claude's messages while it did consume codex/grok — a Claude supervisor consuming via the same store API would need separate verification.

## BLOCKED / not-exercised, with prerequisites

- **C-URG (busy urgent mid-turn redirect): BLOCKED.** Prerequisite: supervisor sends an `urgent=true` message to `comm-claude` while it is mid-turn. None sent in-window (supervisor's only `urgent` probe went to `comm-codex`, id 3632). Additionally suspect: sibling probe **cas-5c02 (comm-grok) reported an urgent-halt deadlock**, so even had the interrupt been sent, the `urgent`+halt path is independently at risk; cross-reference that report. Repro to run: `coordination action=message target=comm-claude urgent=true …` then diff worker turn interruption.
- **C-IDLE (idle wake) & C-N reaction: RESOLVED → FAIL.** The investigator did end its turn (forced idle at ~15:04Z via director quiet-nudges). The idle boundary flushed the *director* channel (wakes recorded) but flushed **none** of the supervisor `prompt_queue` messages — the C-N01..10 burst stayed 0/13 delivered through the idle event and past the 15:03:13Z evidence cutoff. Idle-wake works only for the director channel, not the supervisor coordination channel (CF-3).

## Minimal repros

- **CF-1:** In a Codex-supervised factory, spawn a Claude worker; from the supervisor `coordination action=message target=<claude-worker> message="<contract text>"`. Observe: worker turn shows only the generic assignment template; `SELECT prompt,processed_at FROM prompt_queue WHERE target='<worker>'` shows the contract row present but `processed_at=NULL` and its body absent from the worker transcript.
- **CF-2:** From the Claude worker send any `coordination message` to the supervisor; `SELECT processed_at,acked_at FROM prompt_queue WHERE id=<returned id>` stays NULL indefinitely while a sibling Codex/Grok worker's row flips within ~1.2 s.
- **CF-3:** Enqueue N messages to a busy Claude worker; grep daemon log for `director_events … target_agent=<worker>` — deliveries lag by minutes and none surface until the worker idles.
- **C-NEG:** As a worker, `coordination action=message target=<peer>` → `-32600`; omit `summary` → `-32602`.

## Appendix — raw ledger (message ids)

- Inbound to comm-claude (all `processed_at=NULL`, none surfaced to turn): 3599 (`cas` bootstrap), 3601 (`supervisor` contract), 3662–3671 (C-N01..10), 3672–3674 (C-FB/FC/FA), 3681 (`supervisor` triage 15:03:22Z), 3682 (`supervisor` cutoff/completion order 15:04:41Z).
- Outbound from comm-claude (all `processed_at=NULL`): 3606, 3630, 3633–3642, 3644, 3645, 3655, 3683.
- Sibling transport-confirmed (~1.2 s, for contrast): 3631, 3632, 3648, 3650, 3651, 3652, 3656, 3658.
- daemon `director_events` deliveries to comm-claude (the only channel that reached the worker): 14:46:16.465Z (34 ms), 14:58:25.137Z (19 ms), 15:02:59.844Z (22 ms), 15:04:14.358Z (34 ms).
- Note on delivery irony: the supervisor's own completion instructions (3681/3682) telling this worker to finalize were themselves undelivered via the coordination channel and had to be read out of `prompt_queue` — a live demonstration of the reported gap.
