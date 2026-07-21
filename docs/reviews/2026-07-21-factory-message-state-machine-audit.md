# Factory message state machine & observability audit

**Task:** cas-291c · **Epic:** cas-04a6 · **Date:** 2026-07-21  
**Agent:** `comm-grok` · **Posture:** spike, deep, additive-only (this report only)  
**Inputs:** current tree + live probe evidence from cas-5c02 / cas-13fa / cas-e76b · related: cas-ca04, cas-b68a, cas-b269, cas-f9e8, cas-6913, cas-126b, cas-afb7, cas-f710, cas-73c8, cas-7f57

## Purpose

Map the end-to-end message lifecycle so an engineer can point at the exact state where a factory message was lost, delayed, duplicated, or ignored — and so automation for the 3×3 harness matrix can hang off a **harness-agnostic core** with thin adapters.

---

## 1. Canonical state machine (all harnesses)

States are logical. Storage columns and log stages are the only durable CAS-side signals unless noted.

```text
                    ┌─────────────┐
   MCP / hook /     │  VALIDATING │  schema, role ACLs, summary required
   bridge send ───► │             │
                    └──────┬──────┘
                           │ fail → REJECTED (MCP error; no queue row)
                           ▼
                    ┌─────────────┐
                    │ HALT_PREP   │  urgent only: cas-b269 halt_task_work on
                    │ (optional)  │  session-scoped workers (all-or-none w/ enqueue)
                    └──────┬──────┘
                           ▼
                    ┌─────────────┐
                    │  ENQUEUED   │  prompt_queue row: created_at, id, priority,
                    │  (Pending)  │  urgent, factory_session, processed_at=NULL
                    └──────┬──────┘
                           │ notify_daemon (best-effort)
                           ▼
                    ┌─────────────┐
         daemon     │  PEEKED     │  process_prompt_queue LIMIT 10
         tick ────► │             │  ORDER BY priority ASC, id ASC
                    └──────┬──────┘
           ┌───────────────┼────────────────┬──────────────────┐
           ▼               ▼                ▼                  ▼
     DROP_DEAD       DROP_IDLE_DEDUP   SKIP_NATIVE       GATE_PTY_NOT_READY
     (mark proc)     (mark proc)       (no mark)         (no mark; retry)
           │               │                │                  │
           └───────────────┴────────────────┴──────────────────┘
                           │ proceed
                           ▼
                    ┌─────────────┐
                    │  ROUTING    │  choose_channel(recipient harness, teams)
                    └──────┬──────┘
              ┌────────────┴────────────┐
              ▼                         ▼
       TeamsInbox write            PTY inject / interrupt_and_inject
              │                         │
              ▼                         ▼
                    ┌─────────────┐
                    │ DELIVERED   │  processed_at set; MessageStatus::Delivered
                    │ (transport) │  log stage=delivered + deliver_ms
                    └──────┬──────┘
                           │  ※ no automatic CAS link to model wake
                           ▼
                    ┌─────────────┐
                    │ TURN_WAKE   │  harness-local: new user prompt / inbox poll
                    │ (external)  │  Grok prompt_history · Codex rollout · CC inbox
                    └──────┬──────┘
                           ▼
                    ┌─────────────┐
                    │  REACTED    │  model first_token / tool use (transcript only)
                    └──────┬──────┘
                           │ optional explicit message_ack(id)
                           ▼
                    ┌─────────────┐
                    │ CONFIRMED   │  acked_at set; MessageStatus::Confirmed
                    └─────────────┘

Side channel (not prompt_queue):
  director_events ──► deliver_to_worker ──► TURN_WAKE  (no message id / processed_at)
```

### State ↔ storage / API

| Logical state | Durable signal | API / log |
|---|---|---|
| REJECTED | none | MCP `-3260x` text |
| ENQUEUED / Pending | `prompt_queue.created_at`, `processed_at IS NULL` | `message_status` → `pending`; MCP “Message queued” |
| PEEKED | none (ephemeral) | log `Processing N prompts` + debug `daemon_pickup` |
| DROP_* | `processed_at` set without inject | debug suppress / abandon logs |
| GATE_PTY_NOT_READY | still Pending | silent continue (no structured metric) |
| DELIVERED | `processed_at` | `message_status` → `delivered`; `stage=delivered` |
| TURN_WAKE | harness artifact only | **blind to CAS core** |
| REACTED | harness artifact only | **blind to CAS core** |
| CONFIRMED | `acked_at` | `message_ack` / bridge ack; almost never used in factory practice |

---

## 2. Transition table (source-grounded)

| # | From → To | Trigger | File:symbol | Observable? |
|---|---|---|---|---|
| T1 | ∅ → VALIDATING | `coordination action=message` | `CasService::message_send` — `cas-cli/src/mcp/tools/service/agent_search_system/message.rs:4` | yes (MCP errors) |
| T2 | VALIDATING → REJECTED | missing target/message/summary; worker non-supervisor target | `message_send` L10–98 | yes |
| T3 | VALIDATING → HALT_PREP | `urgent=true` + authorized supervisor/director | `stale_close_guard::{apply_halt_metadata,halt_targets_for_urgent,should_persist_urgent_halt}` + `message_send` L299–408 | partial (agent.metadata) |
| T4 | → ENQUEUED | `PromptQueueStore::enqueue_urgent` | `prompt_queue_store.rs:352–377` | yes (id, created_at) |
| T5 | ENQUEUED → notify | `cas_factory::notify_daemon` | `message_send` L461–489 | debug only; fail soft |
| T6 | ENQUEUED → PEEKED | daemon `process_prompt_queue` | `queue_and_events.rs:189` · `peek_for_targets` `prompt_queue_store.rs:517–575` | log “Processing N” only |
| T7 | PEEKED → DROP_DEAD | dead worker source | `is_dead_worker_source` + mark_processed L271–278 | debug |
| T8 | PEEKED → DROP_IDLE_DEDUP | non-urgent idle-like text, 5 min window | `is_idle_message` L125+ · L287–300 | debug |
| T9 | PEEKED → SKIP_NATIVE | `native_extension=true` agent | L307–309 **continue without mark** | **blind** (spin forever) |
| T10 | PEEKED → GATE_PTY | `!pane_ready_for_injection` | L340–342 · `pane::ready_for_injection` (bytes>0 && age≥5s) | **blind** (retry silent) |
| T11 | PEEKED → ROUTING | ready | `deliver_to_worker` / urgent branch | — |
| T12 | ROUTING → TeamsInbox | Claude + teams_active | `choose_channel` `delivery.rs:46–57` · `TeamsManager::write_to_inbox` | file inbox; may dedup |
| T13 | ROUTING → PTY normal | Codex/Grok always; Claude non-teams | `Mux::inject` | log Injecting + delivered |
| T14 | ROUTING → PTY urgent | `queued.urgent` | `Mux::interrupt_and_inject` (Esc + settle + inject) `mux.rs:863` | log `urgent_interrupt` + delivered |
| T15 | → DELIVERED | success | `mark_processed` L624–627 | yes `processed_at` |
| T16 | → ABANDON unknown pane | pane missing & not current session | L551–606 re-queue notice to supervisor | warn + synthetic row |
| T17 | DELIVERED → CONFIRMED | explicit ack | `CasService::message_ack` L516 · `PromptQueueStore::ack` | rare in practice |
| T18 | director path | refresh tick prompts | `lifecycle.rs` director inject L376–419 | **no queue id**; log `director_events` |
| T19 | REACTED | model | harness transcripts only | **CAS blind** |
| T20 | urgent halt → clear | successful `task start` | `lifecycle.rs` + `should_clear_halt_at_generation` | close blocked until clear (`close_ops.rs` cas-b269) |

---

## 3. Harness adapters (3×3 relevant)

Routing is **recipient-aware** (cas-b68a), not supervisor-mode-aware.

| Recipient harness | Channel | PTY readiness gate | Payload framing | Turn-wake evidence (external) |
|---|---|---|---|---|
| **Claude** + teams | TeamsInbox | no | n/a (inbox JSON) | Agent-Teams inbox files / CC session |
| **Claude** no teams | PTY | yes | bare | session JSONL user message |
| **Codex** | PTY always | yes | `Message from {source}: {text}` | Codex rollout `user_message` |
| **Grok** | PTY always | yes | bare (no Codex framing) | `prompt_history.jsonl` + `events.jsonl` |

**Symbols**

- `choose_channel` — `delivery.rs:46`
- `requires_pty_readiness_gate` — `delivery.rs:65`
- `pty_payload_needs_framing` — `delivery.rs:94` (Codex only; Grok explicitly excluded, cas-8888/cas-9a31)
- `attribute_for_pty` — `delivery.rs:103`
- Matrix tests — `delivery_matrix_tests.rs` (Claude/Codex/Grok combos)

**Unknown / partial**

- Whether Grok TUI always increments `total_bytes_received` enough for readiness: **UNKNOWN** without mux counters in-process; live session did deliver director inject and urgent inject, so readiness eventually true for those panes.
- Exact CC inbox poll latency after `write_to_inbox`: **UNKNOWN** in CAS metrics (file write success ≠ model turn).

---

## 4. Traced paths (normal / urgent / error)

### 4.1 Normal success (intended)

1. `message_send` validates → `enqueue_urgent(..., urgent=false)` priority Normal=2  
2. `notify_daemon`  
3. `peek_for_targets` includes row  
4. readiness ok → `deliver_to_worker` → channel inject/inbox  
5. `mark_processed` → status Delivered  
6. Recipient starts turn (harness) → optional `message_ack` → Confirmed  

**Live (cas-5c02/13fa/e76b session `cas-src-patient-tiger-71`):** steps 1–2 OK; steps 3–5 **failed for normal traffic** for minutes while urgent succeeded — see §6.

### 4.2 Urgent success

1. `urgent=true` → priority Critical=0 (unless explicit)  
2. **Before enqueue:** persist `halt_task_work` + gen on session workers (supervisor/director sources)  
3. Enqueue with `urgent=1`  
4. Peek sorts priority first → jumps normal backlog  
5. `interrupt_and_inject` with harness settle (log `settle_ms=1200` observed)  
6. Delivered ~1.2s wall (`deliver_ms≈1200` dominated by settle floor)  
7. Halt blocks `task close` until successful `task start` clears gen-scoped halt  

**Live evidence:** ids 3631, 3632, 3648, G-M1 MERGE DONE → Grok turn in ~4s reaction; close then WORK HALTED until cas-291c start.

### 4.3 Unknown-target / ACL failure

| Caller | Target | Outcome | Symbol |
|---|---|---|---|
| Worker | peer / unknown name | `-32600` Workers can only message supervisor | `message_send` L86–93 |
| Worker | `all_workers` | rejected | L79–83 |
| Non-worker | unregistered name | still enqueued; MCP says “queued — target not yet registered” | L267–281, L496–497 |
| Daemon | pane missing, not current | abandon + supervisor notice | `queue_and_events` L551–606 |

Worker path never creates a queue row for bad targets — negative path is **pre-enqueue**.

### 4.4 Session isolation

- Enqueue tags `CAS_FACTORY_SESSION` when set (`message_send` L283).  
- `peek_for_targets(session)`:

```sql
(factory_session IS NULL AND target IN (...session agents...))
OR factory_session = ?
ORDER BY priority ASC, id ASC
LIMIT 10
```

  (`prompt_queue_store.rs:535–561`)

- **Implication:** all **legacy NULL-session** rows whose `target` is in `{supervisor, director, current worker names, all_workers}` compete with live session rows at Normal priority.  
- Contrast `poll_for_target_with_session` (L393–411): filters `(session AND target) OR (NULL AND target)` per single target — safer, not used by daemon peek.

### 4.5 Replay / dedup branches

| Layer | Behavior | Symbol |
|---|---|---|
| prompt_queue | **no** content idempotency; each send new id | `enqueue_urgent` insert |
| Idle W→S text | rate-limit drop 5 min / source | `is_idle_message` |
| Claude inbox | identical `(from, text)` skip while entry present | `write_to_inbox` dedup (cas-7f57/cas-73c8) |
| Urgent | never idle-deduped | L285–286 comment |
| message_ack | idempotent acked_at | `ack` SQL |

---

## 5. Event / ID correlation map

| ID / field | Meaning | Correlates to |
|---|---|---|
| `prompt_queue.id` | message id returned by MCP | `message_status(notification_id)` · log `message_id=` |
| `created_at` | enqueue wall time | transport start |
| `processed_at` | daemon inject/inbox success time | Delivered |
| `acked_at` | explicit recipient ack | Confirmed (manual / bridge) |
| `priority` | 0 Critical / 1 High / 2 Normal | sort key with id |
| `urgent` | interrupt path flag | `interrupt_and_inject` |
| `factory_session` | isolation tag | peek filter |
| `source` / `target` | display names (`supervisor` for sup sends) | routing pane name |
| log `deliver_ms` | now − created_at at inject | transport latency |
| log `inject_ms` | director path inject duration | Channel C |
| log `persist_ms` / `notify_ms` | enqueue-side (debug) | cas-f9e8 |
| agent.metadata `halt_task_work(_gen)` | urgent stop generation | close/start guards |
| EventStore `SupervisorInjected` | best-effort injection record | may lag / missing |
| Grok `prompt_history.timestamp` | turn-wake | reaction start |
| Grok `events.first_token` | model reaction | REACTED |
| Codex rollout user_message ts | turn-wake | REACTED start |
| Claude inbox file mtime / session | turn-wake | **weak** |

**Missing correlation key:** there is **no** CAS field linking `prompt_queue.id` → harness prompt id / turn id. Investigators must join by time + target name + payload substring.

---

## 6. Observability blind spots (priority ordered)

| ID | Blind spot | Why it matters | Live proof |
|---|---|---|---|
| B1 | **Peek HOL + LIMIT 10** mixes legacy NULL-session rows with live session | Normal traffic never reaches inject while urgent (prio 0) does | cas-5c02: 42 legacy sup/dir NULL rows; patient-tiger normal pending; urgent 3631/3632 delivered |
| B2 | `message_status=pending` while MCP says “queued for next poll” | Operators treat enqueue as delivery | All normal W2S probes Pending >5 min |
| B3 | PTY not-ready / native skip: **no mark, no metric** | Silent retry storm (`Processing 10 prompts` @ ~10 Hz) | thousands of process logs, zero Injecting for normal |
| B4 | TURN_WAKE / REACTED not in CAS | Cannot SLO “idle wake ≤5s” from DB alone | must read Grok/Codex/CC artifacts |
| B5 | `message_ack` unused by factory skills | Confirmed state effectively dead | status stuck at Delivered or Pending |
| B6 | Director path has **no message id** | First-contact invisible to `message_status` | director deliver log only |
| B7 | Idle dedup can drop legitimate “ready” | False negative on worker→sup heartbeat | `is_idle_message` |
| B8 | Inbox content dedup drops intentional replay | Duplicate probes look like one | `write_to_inbox` |
| B9 | Urgent halt blocks close without linking halt_gen to message id | MERGE DONE urgent wakes worker but re-close deadlocks with AwaitingMerge | G-M1 / cas-126b class |
| B10 | Worker ACL collapses unknown-target vs peer | Negative tests cannot distinguish | single `-32600` string |
| B11 | `RUST_LOG=cas::coordination=debug` required for enqueue/notify/pickup timings | Default logs only show Processing + some delivered | cas-f9e8 stages at debug |

---

## 7. What CAS **can** distinguish today

| Stage | Can distinguish? | How |
|---|---|---|
| Enqueue | **yes** | id + created_at + MCP response |
| Route decision | **yes in code/tests** | pure `choose_channel` / framing; not logged per message at info |
| Transport delivery | **yes if processed_at set** | status Delivered + deliver_ms |
| Turn wake | **no (core)** | harness only |
| Model reaction | **no (core)** | harness only |
| Ack | **yes if used** | acked_at / Confirmed |

---

## 8. Automation seam recommendation (3×3 matrix)

Design goal: **one core probe runner**, three thin harness adapters, zero model-specific business logic in the core.

```text
                    ┌──────────────────────────┐
                    │  ConformanceCore         │
                    │  - emit message (MCP)    │
                    │  - poll message_status   │
                    │  - read prompt_queue RO  │
                    │  - parse coordination log│
                    │  - assert state ladder   │
                    └────────────┬─────────────┘
           ┌─────────────────────┼─────────────────────┐
           ▼                     ▼                     ▼
    ClaudeAdapter          CodexAdapter           GrokAdapter
    - inbox path           - rollout JSONL        - prompt_history
    - session JSONL        - framed prefix check  - events.jsonl
    - turn_wake detect     - turn_wake detect     - turn_wake detect
```

### Core assertions (harness-agnostic)

1. **Enqueue:** MCP returns id; row exists with expected urgent/priority/session.  
2. **Delivered:** `processed_at` within SLO **or** explicit fail with queue-head diagnosis (count of lower-id pending, priority).  
3. **Not pending forever:** fail if Pending > T while daemon Processing ticks advance.  
4. **Urgent vs normal:** same payload, different urgent flag → Critical sorts ahead.  
5. **ACL:** worker→non-sup → error, zero row.  
6. **Halt:** urgent supervisor→worker sets metadata; close blocked; start clears (generation).  

### Adapter assertions (harness-specific)

7. **Turn wake ≤ SLO** after Delivered (or after director inject).  
8. **Framing:** Codex transcript contains `Message from`; Grok/Claude bare.  
9. **Channel:** under teams, Claude path must not require PTY inject success for peer messages.

### Implementation seam (suggested, not implemented)

| Component | Location suggestion | Notes |
|---|---|---|
| Pure routers already testable | `delivery_matrix_tests.rs` | keep |
| Queue peek contract tests | `prompt_queue_store` unit tests | add HOL/legacy NULL cases |
| Live probe binary / `cas factory probe-comm` | CLI | orchestrates Core+Adapter |
| Evidence bundle | JSONL per trial | {message_id, states[], harness_wake_ts} |

**Do not** put Grok/Codex/Claude branches inside `message_send` or `choose_channel` beyond existing recipient harness enum.

---

## 9. Related task linkage

| Task / epic | Relevance |
|---|---|
| cas-ca04 / cas-b68a | Recipient-aware PTY vs inbox |
| cas-f9e8 | staged coordination telemetry (mostly debug) |
| cas-6913 | honest “not yet registered” delivery line |
| cas-b269 | urgent halt + close block + start clear |
| cas-126b | idle after MERGE / re-close — confirmed interaction with halt + AwaitingMerge |
| cas-7f57 / cas-73c8 | inbox dedup / director membership |
| cas-afb7 | idle-ping before queue drain (spawn race) |
| cas-f710 | outbox replay + supervisor-silent |
| cas-5c02 / 13fa / e76b | live conformance legs feeding this map |

---

## 10. Decision summary

1. **State machine is real and mostly centralized** in `message_send` → `prompt_queue` → `process_prompt_queue` → `deliver_to_worker` / urgent inject, with a **parallel director_events** channel.  
2. **Observability collapses ENQUEUED and “waiting forever”** into the same `pending` status; transport success is only `processed_at`.  
3. **TURN_WAKE and REACTED are outside CAS** — any reaction SLO requires harness adapters.  
4. **Highest-impact blind spot for multi-session factories:** `peek_for_targets` legacy NULL-session HOL under LIMIT 10 (B1) — explains normal-path mass failure with healthy urgent path.  
5. **Automation seam:** ConformanceCore + three wake adapters; extend store tests for peek isolation; do not model-specialize core routing.

---

## 11. UNKNOWN register

| Item | Missing evidence |
|---|---|
| Exact daemon tick interval / whether notify_daemon short-circuits peek lag | runtime config not traced this pass |
| native_extension agents in production factories | no live agent.metadata sample with flag true this session |
| Whether Claude teams inbox delivery updates any CAS counter | only file write result |
| Full supervisor_queue role for worker lifecycle vs prompt_queue | separate path; not on critical message inject path for agent chat |
| Historical processed_at null for abandoned vs never-peeked | need per-id daemon debug logs |

---

*End audit cas-291c — no production code changes.*
