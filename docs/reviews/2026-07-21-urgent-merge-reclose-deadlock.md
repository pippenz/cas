# Characterize urgent-stop × AwaitingMerge re-close deadlock

**Task:** cas-69e1 · **Epic:** cas-04a6 · **Related:** cas-126b, cas-b269  
**Date:** 2026-07-21 · **Agent:** `comm-grok` · **Posture:** spike, additive-only  
**Non-goals:** no production fix, no force-reset, no new destructive urgent live trials

## Verdict

**Proven on both Grok and Codex** under the same Codex-supervised factory (`cas-src-patient-tiger-71`):

1. Supervisor sends **urgent** MERGE DONE / re-close → transport delivers (~1.2s) and **wakes** the worker.  
2. The same urgent send **sets `halt_task_work`** (cas-b269) on the target worker.  
3. Worker `task close` → **WORK HALTED** (cannot re-close the parked task).  
4. Worker `task start` on that same **AwaitingMerge** task → **rejected** (work already complete; wait for merge then close).  
5. Halt clear requires a **successful start of a different Open task** → operational multi-step recovery.

This is the cross-product of three intentional guards that were never composed for the “merge done → re-close” handoff.

---

## State machine (deadlock core)

```text
                    ┌──────────────────┐
  close (merge gate)│  AwaitingMerge   │  park_task_awaiting_merge
  ─────────────────►│  lease released  │  close_ops.rs
                    └────────┬─────────┘
                             │ supervisor merges factory→epic
                             │ then urgent MERGE DONE (re-close)
                             ▼
                    ┌──────────────────┐
  message_send      │  HALT_TASK_WORK  │  agent.metadata halt_task_work=1
  urgent=true ─────►│  + gen millis    │  + interrupt_and_inject wake
                    └────────┬─────────┘
                             │
              ┌──────────────┼──────────────┐
              ▼              ▼              ▼
        task close      task start      task start
        (same id)       (same id)       (new Open task)
              │              │              │
              ▼              ▼              ▼
        WORK HALTED    Cannot start     SUCCESS → clear halt
        (close_ops)    AwaitingMerge    (lifecycle start)
                       (lifecycle)            │
                                              ▼
                                        task close (parked id)
                                        may succeed if merge OK
```

### Transition → source

| # | Transition | Condition | File:symbol |
|---|---|---|---|
| P1 | InProgress → AwaitingMerge | close hits factory-branch merge gate | `close_ops::park_task_awaiting_merge` L218–266 |
| P2 | enqueue urgent + halt | supervisor/director → worker, urgent | `message_send` L299–408 · `stale_close_guard::{apply_halt_metadata,should_persist_urgent_halt,halt_targets_for_urgent}` |
| P3 | inject wake | daemon urgent path | `queue_and_events` urgent → `Mux::interrupt_and_inject` |
| P4 | close → WORK HALTED | `agent_task_work_halted` | `close_ops.rs` L318–334 · message from `halt_blocks_task_work_message` L57–62 |
| P5 | start same AwaitingMerge → reject | `task.status == AwaitingMerge` | `lifecycle.rs` L403–409 |
| P6 | start other Open task → clear halt | after start succeeds, gen ≤ ceiling | `lifecycle.rs` L728–744 · `should_clear_halt_at_generation` L86–88 |
| P7 | start does **not** clear newer halt | concurrent urgent during start | gen check L736–739 |

**Note:** `halt_targets_for_urgent` **never** targets `supervisor` (L151–152). Worker→supervisor urgent does not set halt on the supervisor.

---

## Live evidence

### G-M1 — Grok worker `comm-grok` / task cas-5c02

| t (UTC) | Event | Evidence class |
|---|---|---|
| 14:53 | Close → MERGE REQUIRED; park AwaitingMerge; lease released | LIVE (task notes) |
| 14:54–14:55 | Normal merge-ready msgs queue; urgent **3648** W→S delivered | LIVE (DB/log) |
| 14:55 | Supervisor merges `a01c52a` → epic as `db27ec44` | LIVE (supervisor note) |
| **14:55:55.262** | G-M1 MERGE DONE user prompt recorded | LIVE `prompt_history` |
| **14:55:55.266** | `turn_started` | LIVE `events.jsonl` |
| **14:55:59.258** | `first_token` (~4.0s reaction) | LIVE |
| ~14:56:13 | `task close` → **WORK HALTED** (cas-b269) | LIVE (MCP error + log ~L50342 class) |
| ~14:56 | `task start cas-5c02` → **Cannot start … awaiting merge** | LIVE (MCP error) |
| 14:56 | Worker messages supervisor; cannot self-recover | LIVE notes |
| 14:57 | Recovery: assign **cas-291c** (Open) | LIVE supervisor decision |
| 14:58 | `start cas-291c` clears halt; **close cas-5c02 SUCCESS** | LIVE |

**Queue / merge IDs:** factory commit `a01c52a`, epic merge `db27ec44`, urgent supervisor notice path observed via G-M1 inject (MERGE DONE text in prompt_history).

### X-M1 — Codex worker `comm-codex` / task cas-13fa

| t (UTC) | Event | Evidence class |
|---|---|---|
| 14:55 | Close → MERGE REQUIRED; AwaitingMerge | LIVE (task notes; status still `awaiting_merge` in DB as of audit) |
| 14:56 | Supervisor merges `f644e1d` → epic as `a5254a13`; sends urgent X-M1 | LIVE notes |
| **14:56:21.570** | Inject X-M1 to `comm-codex` | LIVE log |
| **14:56:21.570** | `urgent_interrupt` message_id=**3651** settle_ms=1200 | LIVE log |
| **14:56:22.772** | delivered deliver_ms=**1204** | LIVE log |
| ~14:56:29 | Worker reaction/commentary (notes) | LIVE notes |
| **14:56:36.086** | `task close` → **WORK HALTED** cas-b269 | LIVE log L50555 |
| later | Still AwaitingMerge until recovery path / further supervisor action | LIVE DB status snapshot |

**Cross-harness conclusion:** Same MCP error string and same log signature on **Codex** after urgent MERGE DONE as on **Grok**. Not Grok-only.

### Earlier same-day pattern (supporting, not X-M1)

Multiple WORK HALTED close refusals ~13:48–13:55 (prior factory workers) — same cas-b269 string. Shows halt×close is production-active beyond this epic’s probes.

---

## Why the three guards deadlock together

| Guard | Intent | Failure mode when composed with MERGE DONE urgent |
|---|---|---|
| **AwaitingMerge park** | Don’t close until factory commits are on epic; release lease | Task cannot be `start`ed; only `close` after merge |
| **halt_task_work on any urgent** | Stop stale close/verify loops after Esc interrupt | Blocks the **one** action AwaitingMerge still needs: `close` |
| **Clear halt only on successful start** | Require intentional new work to resume tool path | Start of the parked task is illegal → **no clear path without a second task** |

`stale_close_guard::looks_like_close_or_verify_guidance` **exists** (L177+) but is **not** used to skip halt for MERGE DONE / re-close prompts — so the system already almost knows this class of message but still applies full halt.

---

## Relationship to cas-126b

| Aspect | cas-126b (filed) | This characterization |
|---|---|---|
| Symptom | Grok idle after MERGE DONE / interrupt re-close | Broader: **wake can succeed** yet **re-close still fails** |
| Evidence class | Idle / no reaction (observed earlier) | **Reaction + WORK HALTED + AwaitingMerge start reject** (G-M1/X-M1) |
| Root | “not acting on prompt” vs transport | **Acting on prompt is insufficient** if halt blocks close |
| Overlap | Same handoff moment | This report is the **lifecycle composition** proof; cas-126b remains the product bug umbrella |

**Do not treat as duplicate of pure idle-no-wake.** G-M1 falsifies “always no wake” for urgent MERGE DONE; it **reproduces** “no successful re-close without supervisor escape hatch.”

---

## Legitimate recovery (safe)

Documented and used live on Grok (cas-5c02 → cas-291c):

1. Supervisor merges factory branch → epic (required anyway).  
2. **Do not rely on urgent MERGE DONE alone** for re-close if halt will arm. Prefer **normal** MERGE DONE **or** assign next Open task without re-urgenting mid-close.  
3. If halt already set: assign a **legitimate next Open task** (not dummy force-reset).  
4. Worker `task start <new>` → clears halt (if gen not superseded).  
5. Worker `task close <AwaitingMerge id>` → succeeds if merge gate green.  
6. Worker continues new task.

### Operational cost (observed)

| Cost item | Magnitude |
|---|---|
| Extra supervisor attention | invent + assign next task, wait for start |
| Worker context | switch tasks; delayed close of completed work |
| Factory throughput | AwaitingMerge + halt can stall a lane for many minutes |
| Risk | Concurrent urgent during start can re-halt (gen race) |
| Rebase | Worker often 1 merge-commit behind epic; assignment gate may fail until G-R* rebase |

**Recovery used this session repeatedly:** G-R1/G-R2/G-R3 rebase + next-task start → re-close. Works but **expensive**.

---

## Unsafe workarounds (do not recommend)

| Workaround | Why unsafe |
|---|---|
| `task reset --force` / force-close AwaitingMerge from supervisor while worker responsive | Destroys lease/ownership semantics; race with live worker |
| Kill worker PID | Loses in-flight context; cas-126b “escape hatch” class |
| Worker clears own `halt_task_work` via DB write | Not exposed; corrupts security model |
| Start AwaitingMerge task to clear halt | **Hard-rejected** by lifecycle (correct for “work complete”) |
| Spam urgent re-close | Re-arms halt gen; worsens deadlock |

---

## Minimal repro (no new live trial required — recipe)

```text
Preconditions:
  - Factory worker (Grok or Codex) with commits only on factory/<name>
  - Shared merge gate enabled (factory branch not on epic)

Steps:
  1. Worker completes work, pushes factory branch, task close
     → MERGE REQUIRED → status=AwaitingMerge, lease released
  2. Supervisor merges factory/<name> into epic (commit lands)
  3. Supervisor: coordination message target=<worker> urgent=true
       message="MERGE DONE … re-close now: task action=close id=<id>"
  4. Observe: worker new turn (wake PASS); agent.metadata.halt_task_work=1
  5. Worker: task action=close id=<same>
     → error WORK HALTED (cas-b269)
  6. Worker: task action=start id=<same>
     → error Cannot start a task that is awaiting merge
  7. (Recovery) Supervisor assigns unrelated Open task T2
  8. Worker start T2 → close <id> succeeds

Expected without fix: step 5–6 deadlock until step 7–8.
```

**Fixtures already capturing this:** G-M1 (cas-5c02 notes + prompt_history + events); X-M1 (log message_id=3651 + WORK HALTED 14:56:36; cas-13fa notes).

---

## SRP-sized remediation unit (spec only — link cas-126b)

**Problem statement (one unit):**  
Urgent delivery used for **merge-complete re-close** must not leave the worker unable to `task close` the AwaitingMerge task that the message itself instructs them to close.

**Recommended single PR / task (pick one primary approach):**

### Option A (preferred): Classify re-close urgents as “halt-exempt”

- In `message_send` halt fan-out, if `looks_like_close_or_verify_guidance(message)` **and** (optional) task id in message is AwaitingMerge, **skip** `apply_halt_metadata` while still doing `interrupt_and_inject`.  
- Keeps halt for true “stop / re-scope” urgents.  
- Touches: `message_send` + unit tests on `should_persist_urgent_halt` / new predicate.  
- Links: cas-126b, cas-b269 composition.

### Option B: Close-time exception

- In `close_ops` halt check: if `task.status == AwaitingMerge` **and** factory merge gate would pass, allow close despite halt (then clear halt).  
- Risk: broader than A; still blocks verify loops if verify remains halted.  
- Touches: `close_ops` only.

### Option C: Explicit `coordination action=merge_done`  

- Separate from generic urgent; never sets halt; always interrupt+inject.  
- Larger API surface; cleaner long-term.

**Out of scope for the unit:** rewriting merge gate, changing AwaitingMerge start policy globally, supervisor auto-close of responsive workers.

**Acceptance for the fix task:**  
G-M1/X-M1 recipe steps 1–5 end with **successful close** without assigning a second task; existing halt behavior for non-reclose urgents preserved by tests.

---

## Inference vs evidence

| Claim | Status |
|---|---|
| Grok wakes on urgent MERGE DONE | **Evidence** (prompt_history + events) |
| Grok close halted after that | **Evidence** (MCP + notes) |
| Grok cannot start AwaitingMerge | **Evidence** (MCP) |
| Codex receives urgent X-M1 id 3651 | **Evidence** (daemon log) |
| Codex close WORK HALTED after X-M1 | **Evidence** (log 14:56:36) |
| Codex also hit start-AwaitingMerge reject | **Not directly logged in notes** — **INFER** from same code path `lifecycle.rs:403` (harness-agnostic) |
| Claude same composition | **INFER** same MCP server; no C-URG live trial (explicitly avoided on cas-e76b) |
| Normal (non-urgent) MERGE DONE avoids halt | **Evidence** from code (`urgent` false → no halt block); live normal MERGE DONE often **undelivered** due to queue HOL (cas-5c02) — separate bug |

---

## Operator cheat-sheet

**If you need re-close after merge:**

1. Prefer **non-urgent** inject once queue is healthy **or** director assignment of next work.  
2. If you already sent urgent MERGE DONE: **immediately assign next Open task** so the worker can start→clear halt→close.  
3. Do **not** kill or force-reset a responsive worker.  
4. Expect rebase if factory branch is one merge commit behind epic before new assignment.

---

## Decision

The urgent-stop × AwaitingMerge re-close deadlock is a **real, cross-harness (Grok+Codex proven), source-mapped** lifecycle bug. Wake success does not imply re-close success. Safe recovery is **legitimate next-task start**, at high operational cost. Remediation should be a **small SRP unit** (prefer halt-exempt re-close urgents or AwaitingMerge close exception) filed/linked under **cas-126b**, not another epic-wide rewrite.

---

*End cas-69e1 characterization — no production code changes.*
