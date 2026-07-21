# Grok live communication conformance probe

**Task:** cas-5c02 · **Epic:** cas-04a6 · **Related bug:** cas-126b  
**Date:** 2026-07-21 · **Agent:** `comm-grok` (`d6cdb24a-62f2-4f75-bdfb-32b9c56c0784`)  
**Factory shape:** Codex supervisor (`quick-finch-86`) · Grok worker (`comm-grok`, model `grok-4.5`)  
**Session:** `cas-src-patient-tiger-71`  
**Branch:** `factory/comm-grok` · **Posture:** spike, deep, additive-only (this report only)

## Verdict (one screen)

| Scenario | Result | One-line finding |
|---|---|---|
| Startup first-contact | **PASS** | Director/PTY inject woke Grok; first turn within ~1.7s of deliver |
| S2W normal ×10 | **FAIL** | Only msg `3602` enqueued; still `pending` after >5 min; no new Grok turn |
| W2S normal ×10 | **FAIL** (enqueue PASS / deliver FAIL) | 10/10 MCP-queued; 0/10 `processed_at`; never injected |
| FIFO burst | **PARTIAL** | Enqueue order not logical-send order under parallel tools; no deliver to measure |
| Idle wake | **BLOCKED** | Cannot observe idle inject while normal queue is HOL-blocked |
| Busy urgent redirect | **BLOCKED** (Grok live) / **PASS** (peer urgent path) | No urgent→Grok this session; urgent→Codex + urgent→Codex-sup delivered ~1.2s |
| Duplicate / replay | **BLOCKED** | No second user prompt on Grok; cannot observe duplicate reaction |
| MERGE-REQUIRED / cas-126b | **REPRODUCED** (characterization) | Normal `MERGE DONE` left unprocessed; urgent re-close sibling delivered |
| Malformed / unknown target | **PASS** | Worker → non-supervisor target rejected at MCP (`-32600`) |

**Headline:** Under this Codex-supervised factory, **normal-priority `prompt_queue` traffic does not reach Grok (or the supervisor) end-to-end**. Enqueue + `message_status=pending` is **not** delivery. **Urgent** (`priority=0`) jumps the queue and delivers in ~1.2s (including interrupt settle). First-contact worked only via the **director_events** path, which bypasses the blocked normal queue head.

---

## Environment & code map (Grok-specific)

| Item | Value / location |
|---|---|
| Recipient channel | Grok **always PTY** (`choose_channel` → `DeliveryChannel::Pty`) — `cas-cli/src/ui/factory/daemon/runtime/delivery.rs` |
| Framing | **No** `Message from …` framing for Grok (`pty_payload_needs_framing` is Codex-only) |
| Queue drain | `process_prompt_queue` peeks `LIMIT 10`, `ORDER BY priority ASC, id ASC` — `queue_and_events.rs` + `prompt_queue_store.rs` |
| PTY readiness | `total_bytes_received > 0` && age ≥ 5s — `crates/cas-mux/src/pane/mod.rs` |
| Transcripts | `~/.grok/sessions/.../comm-grok/d6cdb24a-.../{prompt_history,events,updates,chat_history}.jsonl` |
| Daemon log | `/home/pippenz/Petrastella/cas-src/.cas/logs/cas-2026-07-21.log` |
| DB (read-only URI) | `file:.../cas-src/.cas/cas.db?mode=ro` · table `prompt_queue` |

Shared matrix tests already assert Grok PTY + unframed: `delivery_matrix_tests.rs` (claude-sup/grok-worker, grok-sup/grok-worker).

---

## Latency definitions used

| Metric | Definition |
|---|---|
| **Transport latency** | Enqueue (`prompt_queue.created_at` or coordination MCP return) → daemon inject complete (`processed_at` and/or log `deliver_ms` / `inject_ms`) |
| **Model reaction latency** | Inject / prompt recorded → first model signal (`first_token` or first tool) in Grok `events.jsonl` |
| **Queue wait** | Time sitting with `processed_at IS NULL` while daemon is alive |

Non-goal honored: **queue ACK / “Message queued” ≠ end-to-end success**.

---

## Scenario results

### 1. Startup first-contact — **PASS**

| t (UTC) | Event | Source |
|---|---|---|
| 14:45:56.877 | Worktree + pane spawn `comm-grok` | daemon log |
| 14:45:57.448 | Agent registered | daemon log |
| 14:45:56.980 | `prompt_queue` id **3598** (`cas`→`comm-grok`, startup text) enqueued — still unprocessed | DB |
| 14:46:16.465 | **director prompt delivered** `target_agent=comm-grok` `inject_ms=0.134` `refresh_to_deliver_ms=33.58` channel=`director_events` | log L44579 |
| 14:46:16.630 | `prompt_history.jsonl` records user prompt (task assign text) | Grok session |
| 14:46:16.631 | `turn_started` turn 0 model `grok-4.5` | events.jsonl |
| 14:46:18.201 | `first_token` | events.jsonl |
| 14:46:21.209 | first `tool_completed` (`search_tool`) | events.jsonl |

**Latencies**

| Segment | ms (approx) |
|---|---|
| Transport (director inject_ms) | **0.13** |
| refresh_to_deliver_ms | **33.6** |
| Deliver → prompt_history | ~165 |
| Deliver → first_token | **~1,736** |
| Deliver → first tool | **~4,744** |

**Notes**

- First-contact body was the factory assign template (“You have been assigned a new task… cas-5c02”), **not** the later READY-PROBE contract body in msg **3602**.
- Row **3598** never got `processed_at` — first-contact did **not** depend on normal `prompt_queue` drain for that row.

---

### 2. Supervisor → worker normal ×10 — **FAIL**

| Trial | Message ID | Enqueued (UTC) | `message_status` / `processed_at` | New Grok turn? | Result |
|---|---|---|---|---|---|
| S2W-01 | **3602** | 14:46:27.668 | pending / NULL | **No** (still 1 prompt_history line through 14:51) | **FAIL** |
| S2W-02..10 | — | not sent by supervisor during window | — | — | **BLOCKED** (prerequisite: supervisor scripted series; also same HOL risk) |

**Evidence that 3602 never reached the model**

- Body requires: `READY-PROBE GROK <RFC3339>` + acceptance criteria — worker never saw that text as a user prompt.
- `prompt_history.jsonl` line count = **1** (first-contact only).
- Worker sent belated `READY-PROBE GROK 2026-07-21T14:49:30Z` as msg **3629** after discovering 3602 still pending (self-observation, not a reaction to 3602 inject).

**Minimal repro (S2W normal fail)**

```text
1. Codex supervisor factory; spawn Grok worker; worker starts a long turn.
2. coordination action=message target=comm-grok summary="probe" message="S2W-NORMAL-1" (urgent=false).
3. Observe: Message queued with id N; message_status(N)=pending for minutes.
4. Observe: Grok prompt_history gains no new line; no Injecting/delivered log for N.
5. Control: same factory, urgent=true message (any target with live pane) delivers in ~1.2s (see §6).
```

---

### 3. Worker → supervisor normal ×10 — **FAIL** (enqueue 10/10, deliver 0/10)

All via `cas__coordination action=message target=supervisor` from `comm-grok`.

| Logical | Message ID | created_at (UTC) | processed_at | message_status | Result |
|---|---|---|---|---|---|
| W2S-01 | **3611** | 14:48:06.392 | NULL | pending | FAIL deliver |
| W2S-02 | **3607** | 14:48:06.388 | NULL | pending | FAIL deliver |
| W2S-03 | **3608** | 14:48:06.390 | NULL | pending | FAIL deliver |
| W2S-04 | **3609** | 14:48:06.391 | NULL | pending | FAIL deliver |
| W2S-05 | **3610** | 14:48:06.391 | NULL | pending | FAIL deliver |
| W2S-06 | **3619** | 14:48:18.248 | NULL | pending | FAIL deliver |
| W2S-07 | **3620** | 14:48:18.249 | NULL | pending | FAIL deliver |
| W2S-08 | **3618** | 14:48:18.248 | NULL | pending | FAIL deliver |
| W2S-09 | **3616** | 14:48:18.248 | NULL | pending | FAIL deliver |
| W2S-10 | **3617** | 14:48:18.248 | NULL | pending | FAIL deliver |

Also: ACK **3605** (14:46:40) still pending at 14:51+ (~5+ min).

**Transport vs reaction**

| Layer | Observation |
|---|---|
| MCP enqueue | Immediate success; IDs returned |
| Daemon | Logs `Processing 10 prompts from queue` continuously (~10 Hz) after 14:46 |
| Inject | **Zero** `Injecting prompt` / `prompt_queue message delivered` for these IDs |
| Supervisor reaction | Not observable (no deliver) |

**Minimal repro (W2S normal fail)**

```text
1. From Grok worker: 10× coordination message target=supervisor message="PROBE-W2S-N …".
2. Read-only: SELECT id, processed_at FROM prompt_queue WHERE id IN (...);
3. Expect under SLO: processed_at set and supervisor PTY inject within ≤1s p95.
4. Actual: processed_at remains NULL; queue wait grows unbounded while daemon peeks legacy head.
```

---

### 4. FIFO burst — **PARTIAL**

**Enqueue ordering (parallel MCP calls, first burst ~14:48:06):**

| id order | Logical token | Δt from first of burst |
|---|---|---|
| 3607 | W2S-02 | 0 |
| 3608 | W2S-03 | +2 ms |
| 3609 | W2S-04 | +2 ms |
| 3610 | W2S-05 | +3 ms |
| 3611 | W2S-01 | +4 ms |

Second burst (~14:48:18): ids 3616–3620 map to logical 09,10,08,06,07 — **not** sequential.

**Finding:** Parallel tool dispatch does **not** preserve caller intent order in `id` / `created_at`. FIFO-at-inject could not be measured because **no inject occurred**. Serial send would be required for a clean FIFO trial once delivery works.

---

### 5. Idle wake — **BLOCKED**

**Prerequisite missing:** a normal-priority inject to an idle Grok pane after turn completion.

- Worker remained in one long first turn for the entire probe window.
- Even after a future idle, normal messages sit behind the HOL blocker (§Root cause).
- First-contact shows idle/new-session wake **can** work via `director_events`, not via stuck normal `prompt_queue`.

---

### 6. Busy urgent redirect — **BLOCKED** for live Grok; **PASS** for urgent path in same factory

No `urgent=true` message targeted `comm-grok` during this window (supervisor did not send Grok urgent probe).

**Same-factory urgent controls (Codex legs, same daemon):**

| ID | Direction | urgent | created_at | processed_at | deliver_ms (log) | Result |
|---|---|---|---|---|---|---|
| **3631** | codex-worker → supervisor | 1 | 14:50:31.503 | 14:50:32.722 | **1204** | PASS transport |
| **3632** | supervisor → codex-worker | 1 | 14:50:41.586 | 14:50:42.792 | **1204** | PASS transport |

Log pattern: `urgent_interrupt` → settle_ms=1200 → `delivered`.

**Implication for Grok:** routing code treats Grok like Codex for PTY; urgent should use `interrupt_and_inject`. Live Grok interrupt reaction remains **unverified this session** (needs supervisor `urgent=true` → `comm-grok` while busy).

**Minimal repro (proposed for Grok urgent)**

```text
1. Keep Grok mid-tooling.
2. coordination action=message target=comm-grok urgent=true message="URGENT-PROBE GROK-1".
3. Expect: deliver_ms ~1.2s; new prompt_history line; turn break + new turn_started.
4. Compare with normal twin message that stays pending.
```

---

### 7. Duplicate / replay observation — **BLOCKED**

- Only one user prompt delivered to this Grok session.
- No intentional duplicate inject occurred.
- Idle-message dedup exists in daemon (`is_idle_message`, 5 min) for **sources**, not exercised here.

---

### 8. MERGE-REQUIRED re-close / cas-126b — **REPRODUCED** (characterization, not falsified)

**Live same-day evidence (prior Grok factory workers under prior supervisor `true-otter-34`):**

| ID | Target | urgent | priority | created_at | processed_at | Body (abbrev) |
|---|---|---|---|---|---|---|
| **3568** | std-life | 0 | 2 | 13:54:54 | **NULL** | `MERGE DONE for cas-f53c… Re-close now…` |
| **3569** | std-life | 1 | 0 | 13:55:01 | **13:55:02** | `URGENT: cas-f53c merged… Close immediately` |

Log for 3569: `urgent_interrupt` + `deliver_ms=1202` (and sibling urgents 3570–3571 delivered).

**Interpretation aligned with cas-126b**

- Bug report: workers idle after MERGE DONE / interrupt re-close prompts; heartbeats continue.
- Characterization: **normal MERGE DONE may never leave the queue** (still unprocessed hours later), while **urgent re-close is delivered ~1.2s**. If the worker already finished its turn and only the normal MERGE DONE was sent, or urgent inject fails to restart a Grok turn, the observed “idle after merge” follows.
- This session did **not** re-hit MERGE REQUIRED on cas-5c02 (additive report-only; no production merge cycle).

**Does not falsify cas-126b.** Strengthens it with queue-level evidence.

**Minimal repro**

```text
1. Grok worker hits MERGE REQUIRED on close; goes idle.
2. Supervisor enqueues normal MERGE DONE (priority 2) + optional URGENT close.
3. Observe prompt_queue: normal row may remain processed_at NULL; urgent row processes ~1s.
4. Observe worker: no re-close attempt if only normal path was relied upon / if inject did not start a turn.
```

---

### 9. Malformed / unknown-target negative path — **PASS**

| Attempt | Result |
|---|---|
| `target=no-such-agent-xyz-5c02` | MCP error **`-32600`**: `Workers can only message their supervisor. Use target='supervisor' or 'quick-finch-86'` |

No queue row created. Negative path correctly rejected at authorization layer.

---

## Root cause: normal queue head-of-line blocking

### Observed daemon behavior (14:46–14:51)

- Continuous `Processing 10 prompts from queue` (thousands of ticks).
- **No** `Injecting prompt to 'comm-grok'` / `'quick-finch-86'` for normal patient-tiger rows.
- Only **urgent** patient-tiger rows **3631/3632** delivered.

### Peek semantics (`peek_for_targets`)

When `factory_session = Some(session)`:

```sql
WHERE processed_at IS NULL
  AND (
    (factory_session IS NULL AND target IN (<session agent names + supervisor + director + all_workers>))
    OR factory_session = ?
  )
ORDER BY priority ASC, id ASC
LIMIT 10
```

### Head composition at probe time

| Bucket | Count | id range | Effect |
|---|---|---|---|
| Legacy NULL-session `supervisor`/`director` | **42** | 1217–2614 (from 2026-04-15 onward) | Occupy LIMIT 10 at priority 2 |
| Legacy NULL cas→workers | 3 | 3597–3599 | Also ahead of some traffic |
| `cas-src-patient-tiger-71` | 44+ | 3600+ | Starved unless **priority=0 (urgent)** |

**~45 rows with lower id / equal priority sit ahead of the first patient-tiger message.**  
Urgent (`priority=0`) sorts **before** all priority-2 legacy rows → delivers.

### Why first-contact still worked

Director path: `channel=director_events` with `inject_ms≈0.13` — **not** dependent on clearing the normal prompt_queue head.

### Supported Grok paths (code + this live run)

| Path | Code | Live this session |
|---|---|---|
| Director → Grok PTY inject | yes | **PASS** (first-contact) |
| Normal prompt_queue → Grok | yes (PTY, unframed) | **FAIL** (HOL) |
| Urgent prompt_queue → Grok | yes (`interrupt_and_inject`) | **not exercised** |
| Grok → supervisor normal | yes (PTY to Codex sup) | **FAIL** (HOL) |
| Peer urgent → supervisor | yes | **PASS** (3631, Codex worker) |

---

## SLO candidates vs this run (Grok row)

| Candidate SLO (from epic) | Grok result |
|---|---|
| 10/10 delivery each direction | **Miss** — W2S 0/10 deliver; S2W 0/1 deliver (9 not sent) |
| FIFO per target | **Not measurable** at inject; enqueue reorders under parallel tools |
| Zero unintended duplicate reactions | **N/A** (no multi-inject) |
| Enqueue→recipient transport ≤1s p95 | **Miss** for normal; **~1.2s** for urgent (includes 1200 ms settle floor) |
| Idle recipient new turn ≤5s p95 | **PASS** only on director first-contact (~1.7s to first_token); normal idle **BLOCKED** |
| Urgent busy redirect ≤5s p95 | **PASS** transport on Codex legs; Grok live **BLOCKED** |
| Worker status without polling ≤5s | **Miss** — ACK/probes never reached supervisor PTY |

---

## Evidence index

| Artifact | Path / ref |
|---|---|
| Report | `docs/reviews/2026-07-21-comm-conformance-grok.md` (this file) |
| Grok prompt_history | `~/.grok/sessions/%2Fhome%2Fpippenz%2FPetrastella%2Fcas-src%2F.cas%2Fworktrees%2Fcomm-grok/prompt_history.jsonl` |
| Grok events | `.../d6cdb24a-62f2-4f75-bdfb-32b9c56c0784/events.jsonl` |
| Daemon log | `.cas/logs/cas-2026-07-21.log` (spawn ~L44329, director L44579, urgent L47086–47176) |
| DB | `prompt_queue` ids 3568–3569, 3598, 3602, 3605, 3607–3611, 3616–3620, 3627, 3629, 3631–3632 |
| Task notes | cas-5c02 discovery/progress notes |
| Code | `delivery.rs`, `queue_and_events.rs`, `prompt_queue_store.rs` (`peek_for_targets`), `pane/mod.rs` (`ready_for_injection`) |

---

## Remediation pointers (no fixes in this spike)

Non-duplicative links / suggested follow-ups for epic synthesis (cas-563d):

1. **Drain or scope-out legacy NULL-session `supervisor`/`director` rows** so `LIMIT 10` cannot HOL-block live sessions (or exclude NULL-session rows when session is set, except intentional legacy).
2. **cas-126b** — treat as confirmed risk: do not rely on normal-priority MERGE DONE; urgent path delivers transport but Grok reaction still needs a dedicated live interrupt trial.
3. **Do not treat MCP “Message queued” / `message_status=pending` as SLO success** — require `processed_at` + recipient transcript line.
4. Optional: serial W2S/S2W conformance harness once queue head is healthy; measure FIFO at inject, not parallel MCP issue time.

---

## Decision

**Grok under Codex supervision (session `cas-src-patient-tiger-71`, 2026-07-21): real-time supervisor↔worker communication does not meet the proposed normal-path SLOs.** First-contact and urgent transport work; normal bidirectional prompt_queue delivery does not. cas-126b is **not falsified** and is **supported** by unprocessed normal MERGE DONE vs delivered urgent re-close.

---

*End of Grok conformance packet for cas-5c02 / cas-04a6.*
