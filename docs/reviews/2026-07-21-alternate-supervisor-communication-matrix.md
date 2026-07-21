# Alternate-supervisor communication matrix (Grok / Claude supervisors)

**Task:** cas-474b · **Epic:** cas-04a6 · **Date:** 2026-07-21  
**Agent:** `comm-grok` · **Parent factory:** Codex supervisor `quick-finch-86` / session `cas-src-patient-tiger-71`  
**Posture:** spike, deep, additive-only · **No nested factory launched** (parent + shared `cas.db` left intact)

## Scope

Account for matrix legs this **Codex-supervised** session cannot directly LIVE-prove: **Grok and Claude as supervisors** with Grok / Claude / Codex workers.

| In scope (6 pairings) | Out of scope this task |
|---|---|
| Grok-sup × {Grok, Claude, Codex} | Codex-sup × * (parent session; covered by cas-5c02 / cas-13fa / cas-e76b) |
| Claude-sup × {Grok, Claude, Codex} | Production code/test changes |

Scenarios per pairing (minimum): normal targeted message, worker status reply, idle wake, urgent busy redirect, FIFO burst, unknown target, session-isolation — each labeled **LIVE / AUTOMATED / STATIC / BLOCKED**.

---

## Evidence class definitions (honest)

| Class | Meaning |
|---|---|
| **LIVE** | Real factory session observed delivering/reacting (this epic or prior documented session) |
| **AUTOMATED** | `cargo test` (or equivalent) green in this worktree on pure routing / contracts |
| **STATIC** | Source/contract derivation only; no green automated assertion for that exact shape |
| **BLOCKED** | Live/automated exercise not run; operator prerequisite listed; **not** claimed as live |

**Non-claim:** STATIC routing ≠ end-to-end delivery. Parent-session Codex-sup LIVE results are **not** re-labeled as alternate-supervisor LIVE.

---

## Automation run (this worktree)

```text
$ cargo test -p cas --lib delivery_matrix -- --nocapture
# 2026-07-21T15:06:31Z approx; exit 0
running 5 tests
test ...::all_workers_broadcast_routes_per_recipient_in_mixed_factory ... ok
test ...::all_claude_factory_uses_inbox_both_directions_unchanged ... ok
test ...::claude_sup_to_codex_worker_is_pty_gated_framed ... ok
test ...::delivery_matrix_all_combos_both_directions ... ok
test ...::worker_to_codex_supervisor_is_pty_gated_framed ... ok
test result: ok. 5 passed; 0 failed
```

**Source:** `cas-cli/src/ui/factory/daemon/runtime/delivery_matrix_tests.rs`  
**Primitives:** `choose_channel` / `requires_pty_readiness_gate` / `pty_payload_needs_framing` in `delivery.rs`

### Shapes covered by AUTOMATED matrix (`SHAPES`)

| # | Shape | In alternate-supervisor set? |
|---|---|---|
| 0 | claude-sup / codex-worker | **yes** |
| 1 | codex-sup / claude-worker | no (Codex sup) |
| 2 | codex-sup / codex-worker | no |
| 3 | claude-sup / claude-worker | **yes** |
| 4 | claude-sup / grok-worker | **yes** |
| 5 | grok-sup / grok-worker | **yes** |

**Not in SHAPES (gap):** grok-sup / claude-worker · grok-sup / codex-worker → STATIC by pure contract only until tests append those shapes.

---

## Pure routing contract (recipient-aware)

`teams_active = (supervisor == Claude)` only.

| Recipient | teams | Channel | PTY gate | Frame `Message from` |
|---|---|---|---|---|
| Codex | any | Pty | yes | yes |
| Grok | any | Pty | yes | no |
| Claude | true | TeamsInbox | no | no |
| Claude | false | Pty | yes | no |

---

## Matrix: six pairings × two directions

Routing column = AUTOMATED when shape ∈ SHAPES; STATIC when pure-derived only.  
Scenario columns describe **full lifecycle** evidence (enqueue → deliver → wake → reaction), not routing alone.

### Legend for scenario cells

`R-auto` = routing AUTOMATED · `R-static` = routing STATIC · then lifecycle class for E2E scenarios.

### Pairing A — Grok-sup / Grok-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle scenarios (N/status/idle/urgent/FIFO/unk/session) |
|---|---|---|---|---|
| ↓ S→W | Grok | Pty / yes / no | **AUTOMATED** (SHAPES[5]) | Routing PASS auto. Full E2E: **BLOCKED** — no disposable Grok-sup session this run |
| ↑ W→S | Grok | Pty / yes / no | **AUTOMATED** | Same. Historical STATIC note: Grok supervisors have operational issues on merge queue (`docs/requests/completed/BUG-grok-supervisor-misses-awaiting-merge-merge-queue-2026-07-10.md`) — not a delivery-path LIVE proof |

**Minimal repro (operator LIVE):** see §Operator runbook A.

### Pairing B — Grok-sup / Claude-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle |
|---|---|---|---|---|
| ↓ S→W | Claude | Pty / yes / no (teams=false under Grok-sup) | **STATIC** (not in SHAPES) | E2E **BLOCKED** |
| ↑ W→S | Grok | Pty / yes / no | **STATIC** | E2E **BLOCKED** |

**Note:** Under Grok supervisor, Claude worker is **PTY fallback**, not TeamsInbox (teams never active). Distinct from Claude-sup factory.

### Pairing C — Grok-sup / Codex-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle |
|---|---|---|---|---|
| ↓ S→W | Codex | Pty / yes / **yes** | **STATIC** (not in SHAPES) | E2E **BLOCKED** |
| ↑ W→S | Grok | Pty / yes / no | **STATIC** | E2E **BLOCKED** |

**Note:** Downward framing required (Codex recipient). Upward Grok supervisor unframed.

### Pairing D — Claude-sup / Grok-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle |
|---|---|---|---|---|
| ↓ S→W | Grok | Pty / yes / no (even with teams=true) | **AUTOMATED** (SHAPES[4]) | E2E **BLOCKED** this session; routing PASS auto (Grok never reads Teams inbox) |
| ↑ W→S | Claude | TeamsInbox / no / no | **AUTOMATED** | E2E **BLOCKED** here; upward uses inbox to Claude supervisor |

### Pairing E — Claude-sup / Claude-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle |
|---|---|---|---|---|
| ↓ S→W | Claude | TeamsInbox / no / no | **AUTOMATED** + **LIVE (historical)** | Routing auto PASS. **LIVE (prior):** cas-ca04 verification `docs/reviews/2026-06-25-cas-ca04-cross-harness-comms-verification.md` — path 4 live-observed (Claude worker under Claude supervisor received assignments as new turns). Session context: worker `tender-hound-15` / supervisor `proud-owl-91` (2026-06-25). **Not re-run this session.** |
| ↑ W→S | Claude | TeamsInbox / no / no | **AUTOMATED** + **LIVE (historical)** | Same document: Claude-worker→Claude-sup observed |

**Scenarios on that LIVE leg (historical class only):** normal targeted + assignment wake: PASS live-observed then. Urgent / FIFO / session-isolation / unknown-target: **not re-measured here** → treat residual scenarios as **BLOCKED** for fresh proof (do not upgrade historical partial to full suite).

### Pairing F — Claude-sup / Codex-worker

| Dir | Recipient | Channel / gate / frame | Evidence class | Lifecycle |
|---|---|---|---|---|
| ↓ S→W | Codex | Pty / yes / yes | **AUTOMATED** (SHAPES[0], load-bearing cas-b68a) | E2E **BLOCKED** live this session; cas-ca04 labeled code-proven; live-pending (2026-06-25) |
| ↑ W→S | Claude | TeamsInbox / no / no | **AUTOMATED** | E2E **BLOCKED** live; code-proven upward to Claude sup |

---

## Scenario rollup (alternate supervisors only)

| Scenario | Best evidence across 6 pairings | Gaps |
|---|---|---|
| Normal targeted | AUTOMATED routing all covered shapes; LIVE only Claude↔Claude historical | No fresh LIVE under Grok-sup; Claude-sup×Grok/Codex live pending |
| Worker status reply | Same as normal upward | No LIVE measurement of status/blocker text under Grok-sup |
| Idle wake | **BLOCKED** alternate-sup LIVE | Parent Codex-sup idle/director wake is out of scope for this matrix |
| Urgent busy redirect | AUTOMATED: urgent uses same channel + interrupt path in code (`interrupt_and_inject`); LIVE **BLOCKED** for alt-sup (cas-b269 halt risk) | Do not run urgent against parent workers |
| FIFO burst | **BLOCKED** live alt-sup; enqueue FIFO is store-order (`priority,id`) AUTOMATED elsewhere | Parallel MCP reorders not alt-sup specific |
| Unknown target | Worker ACL STATIC/same code path all harnesses (`message_send` worker guard) | **Not** harness-pairing dependent |
| Session isolation | `factory_session` peek filter AUTOMATED in store tests (isolation suite); HOL bug STATIC from cas-291c/cas-5c02 | Live multi-session alt-sup **BLOCKED** |

---

## Why LIVE alternate-supervisor legs are BLOCKED here

| Constraint | Detail |
|---|---|
| Parent factory | Codex-sup patient-tiger-71 owns shared project `cas.db` / panes |
| Task rules | No destructive nested factories; do not disturb parent or shared live DB |
| Disposable CAS_DIR | Not exercised: launching `cas factory --new --supervisor-cli grok|claude` against default project dir would contend on same DB/queue (HOL + session isolation risks documented cas-291c) |
| Worker authority | Worker cannot spawn supervisor-class factories safely mid-session |

**Not improvised:** no second factory process started from this worker.

---

## Operator runbook (smallest LIVE prerequisites)

### A. Disposable Grok-supervised factory (pairings A–C)

```bash
# On a free machine/tty, isolated project copy OR empty temp CAS root if supported:
export CAS_DIR=/tmp/cas-probe-alt-grok   # if operator tooling supports isolated root
mkdir -p "$CAS_DIR"
# Prefer a throwaway git worktree of cas-src, not the live patient-tiger worktrees.

cas factory --new --name alt-grok-probe \
  --supervisor-cli grok \
  --worker-cli grok \
  -w 1
# Then variants:
#   --worker-cli claude
#   --worker-cli codex
#   --worker-spec '{"name":"w1","cli":"codex"}' mixed as needed

# From supervisor (or cas factory message):
cas factory message --session alt-grok-probe --target <worker> --message "PROBE-ALT-S2W-1"
# Worker: coordination message target=supervisor ...
# Capture: prompt_queue ids, processed_at, harness transcript wake ts
# Cleanup: shutdown_workers; remove session; unset CAS_DIR
```

**PASS criteria per scenario:** enqueue id → `processed_at` set → harness turn_wake ≤ SLO → optional reaction.

### B. Disposable Claude-supervised factory (pairings D–F)

```bash
cas factory --new --name alt-claude-probe \
  --supervisor-cli claude \
  --worker-cli grok \   # or claude / codex / mixed --worker-spec
  -w 1
# Same probe commands; for Claude recipients expect TeamsInbox path under teams.
```

### C. Automation gap fix (recommended follow-up, not this task)

Append to `SHAPES` in `delivery_matrix_tests.rs`:

```text
FactoryShape { supervisor: Grok, worker: Claude, label: "grok-sup / claude-worker" },
FactoryShape { supervisor: Grok, worker: Codex,  label: "grok-sup / codex-worker" },
```

Then `delivery_matrix_all_combos_both_directions` upgrades pairings B/C from STATIC → AUTOMATED for routing.

---

## Minimal repros (failures / blocks)

### Repro BLOCKED: cannot LIVE-prove Grok-sup from Codex-sup worker

```text
1. Worker is comm-grok under Codex supervisor in shared factory.
2. Task forbids nested factories on shared cas.db.
3. Therefore pairings A–C have no LIVE row this session.
```

### Repro AUTOMATED: Claude-sup → Codex worker is PTY framed

```bash
cargo test -p cas --lib claude_sup_to_codex_worker_is_pty_gated_framed -- --nocapture
# expect ok
```

### Repro AUTOMATED: Grok-sup / Grok-worker both directions in all_combos

```bash
cargo test -p cas --lib delivery_matrix_all_combos_both_directions -- --nocapture
# covers SHAPES[5] both dirs
```

### Repro STATIC gap: grok-sup × codex not in SHAPES

```text
1. Open delivery_matrix_tests.rs SHAPES.
2. Observe no (Grok, Codex) or (Grok, Claude) entry.
3. expected(Codex, teams=false) still Pty+frame; expected(Claude, false) Pty unframed —
   but no assert currently pins those factory shapes.
```

---

## Cleanup status

| Artifact | Status |
|---|---|
| Nested factory sessions | **none created** |
| Temp CAS_DIR | **none** |
| Parent factory patient-tiger-71 | **intact** |
| Tracked source edits | report file only (this doc) |
| Ephemeral /tmp | none retained |

---

## Summary scoreboard

| Pairing | ↓ routing | ↑ routing | Fresh LIVE E2E | Notes |
|---|---|---|---|---|
| Grok×Grok | AUTOMATED PASS | AUTOMATED PASS | BLOCKED | Operational Grok-sup bugs exist historically (merge queue) |
| Grok×Claude | STATIC PASS* | STATIC PASS* | BLOCKED | *pure contract; not in SHAPES |
| Grok×Codex | STATIC PASS* | STATIC PASS* | BLOCKED | *pure contract; not in SHAPES |
| Claude×Grok | AUTOMATED PASS | AUTOMATED PASS | BLOCKED | Grok downward avoids inbox trap |
| Claude×Claude | AUTOMATED PASS | AUTOMATED PASS | LIVE historical PASS (partial scenarios) | cas-ca04 2026-06-25 |
| Claude×Codex | AUTOMATED PASS | AUTOMATED PASS | BLOCKED live | code-proven cas-b68a/ca04 |

\*STATIC = contract-consistent, not test-pinned for that shape.

---

## Decision

1. **Alternate-supervisor routing is largely AUTOMATED** for four of six pairings; two Grok-sup mixed-worker shapes are **STATIC gaps** in `SHAPES`.  
2. **No safe LIVE alternate-supervisor factory was run** from this Codex-sup worker without risking the parent session/shared DB — all full lifecycle legs for alt-sup are **BLOCKED** with operator runbook.  
3. **Only historical LIVE** alternate-sup evidence retained: Claude-sup ↔ Claude-worker (cas-ca04).  
4. Parent Codex-sup conformance remains in sibling reports (cas-5c02/13fa/e76b) and must not be double-counted here.

---

*End cas-474b matrix packet.*
