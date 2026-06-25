# EPIC cas-ca04 — Cross-harness factory communication: verification

**Task:** cas-47b7 (verification half) · **Epic:** cas-ca04 · **Date:** 2026-06-25
**Branch verified:** `factory/tender-hound-15` fast-forwarded to epic tip (`5bd0c29` + this task's tests)
**Siblings:** cas-b68a (recipient-aware delivery, core) · cas-83c8 (codex worker/supervisor prompts)

## Summary

The fix makes supervisor↔worker message delivery route on the **recipient's**
harness instead of the supervisor's team-mode. A Codex agent (worker *or*
supervisor) is always reached over its PTY — the only channel it can read —
even inside a Claude Agent-Teams factory, and the injected text is framed
`Message from <sender>: …` so the codex prompt recognizes it as an actionable
turn. Claude agents keep using the team inbox unchanged.

Two deliverables: (1) in-repo routing-decision regression matrix — **complete &
green**; (2) live e2e smoke matrix — **routing code-proven for all legs;
live-rollout confirmation of the codex legs is supervisor-gated** (needs a
rebuilt binary + factory restart with codex panes) and is documented below as a
runbook with the all-Claude legs already observed live in this session.

## Deliverable 1 — routing-decision regression matrix (in-repo, no live MCP)

Added `cas-cli/src/ui/factory/daemon/runtime/delivery_matrix_tests.rs`. It drives
the pure routers (`choose_channel`, `requires_pty_readiness_gate`,
`pty_payload_needs_framing`) over every factory shape and both directions,
re-deriving the expected answer from the contract.

`teams_active` is true exactly when the supervisor is Claude (only a Claude
supervisor launches native Agent Teams; a Codex-supervised factory runs
`teams = None`). The recipient is the worker for downward (`target=<worker>`)
and the supervisor for upward (`target=supervisor`).

| # | Factory shape | Direction | Recipient | teams | Channel | PTY gate | Framed |
|---|---|---|---|---|---|---|---|
| 1 | claude-sup / codex-worker | down → worker | Codex | yes | **PTY** | yes | yes |
| 1u| claude-sup / codex-worker | up → supervisor | Claude | yes | TeamsInbox | no | no |
| 2 | codex-sup / claude-worker | down → worker | Claude | no | PTY (fallback) | yes | no |
| 2u| codex-sup / claude-worker | up → supervisor | Codex | no | **PTY** | yes | yes |
| 3 | codex-sup / codex-worker | down → worker | Codex | no | PTY | yes | yes |
| 3u| codex-sup / codex-worker | up → supervisor | Codex | no | **PTY** | yes | yes |
| 4 | claude-sup / claude-worker | down → worker | Claude | yes | TeamsInbox | no | no |
| 4u| claude-sup / claude-worker | up → supervisor | Claude | yes | TeamsInbox | no | no |

Bold cells are the legs the bug previously broke: a Codex recipient was written
to a team inbox it cannot read. Rows 1 and 2u/3u are the load-bearing fixes
(codex worker reachable downward; codex supervisor woken upward). Rows 4/4u are
the all-Claude regression baseline — must stay on the inbox, byte-for-byte.

`all_workers` fan-out is covered too: a broadcast in a mixed (teams-active)
factory routes per-recipient — codex members over framed PTY, claude members
over the inbox — matching the per-worker loop in `queue_and_events.rs`.

**Ownership note:** `delivery.rs` (cas-b68a's file) is left byte-for-byte
unchanged; the matrix is a new test file registered with one `#[cfg(test)] mod`
line. The pure routers are `pub(crate)`, so the matrix must live in-crate (an
integration test under `tests/` cannot reach them) — hence a sibling test
module, not a `tests/` file.

### Full assembled-epic gate

`cargo test --no-fail-fast` on the assembled epic: **4259 passed, 0 failed.**

The two reds cas-b68a flagged for this task to adjudicate:

- **`factory_codex_skill_guardrails::codex_worker_runtime_instruction_allows_close_then_escalate`**
  — CONFIRMED pre-existing, red on `main` (`92b7a78`) before the epic. The worker
  prompt has read "close **it** with `mcp__cs__task action=close…`" since
  cas-bbc2 (`32a3796`), but the guardrail asserted the literal "close with".
  Fixed in the **test file** (owned by this task) to assert on the close-command
  form (`` `mcp__cs__task action=close ``) so prose tweaks don't re-break it.
  `pty.rs` source was correct and was **not** touched. Not filed back to cas-83c8
  — there is no source bug, only a stale assertion.
- **parallel cwd-race flake** — did **not** recur across two full `--no-fail-fast`
  runs. No code change; flagged as environmental (see the "no tmpfs / cwd-race"
  hygiene notes). If it resurfaces it is a test-isolation issue, not a delivery
  regression.

## Deliverable 2 — live e2e smoke matrix

Each path must show the recipient producing a **new** turn (codex: a new
`user_message`/`turn` in its rollout JSONL; claude: a new entry in its session
JSONL) within seconds of a `coordination action=message`.

| # | Path | Status | Evidence |
|---|---|---|---|
| 1 | claude-sup → codex-worker | code-proven; live-pending | row 1 |
| 2 | codex-sup → claude-worker | code-proven; live-pending | row 2 |
| 3 | codex-sup → codex-worker | code-proven; live-pending | row 3 |
| 4 | claude-sup → claude-worker (regression) | **live-observed** | this session: task assignments + messages delivered to this Claude worker as new turns |
| 5 | codex-worker → claude-sup | code-proven; live-pending | row 2u (claude recipient via inbox) |
| 6 | codex-worker → codex-sup | code-proven; live-pending | row 3u |
| 7 | claude-worker → codex-sup | code-proven; live-pending | codex recipient → framed PTY |

**Live-observed (path 4 + claude-worker→claude-sup):** in this very session a
Claude worker (tender-hound-15) under a Claude supervisor (proud-owl-91)
received two task assignments and several messages as fresh turns, and its
replies reached the supervisor — the all-Claude inbox path is intact (no
regression).

**Why the codex legs are live-pending (not a worker action):** a real codex-leg
rollout requires the **fixed binary** running the daemon and a **factory
restarted with codex panes**. The currently-attached daemon is not guaranteed to
be the epic-built binary, and a worker cannot rebuild + restart the factory
(`cas factory` restart is a supervisor/operator action). The routing decision
for every codex leg is proven by Deliverable 1; what remains is a human-driven
rollout confirmation. SQLite/MCP restart caveat applies after swapping the
binary.

### Runbook to close the live codex legs

1. Build the epic binary: `cargo build --release` on the epic branch, install it
   (`cas update --user` or point PATH at the fresh `cas`).
2. Restart the factory with at least one codex worker (and, for legs 5–7, a
   codex supervisor): `cas factory --attach` with codex panes.
3. For each codex-recipient leg, send `mcp__cas__coordination action=message
   target=<recipient> message="ping <leg-id>"` (or assign a task).
4. Confirm the new turn in the recipient's rollout:
   - codex: `tail -f ~/.codex/sessions/$(date +%Y/%m/%d)/rollout-*.jsonl` and
     watch for a new `user_message` carrying `Message from <sender>: ping …`.
   - claude: the recipient's `~/.claude/projects/<flattened-path>/<uuid>.jsonl`.
5. Confirm the **framing** on codex recipients (the literal `Message from
   <sender>: ` prefix) and the **absence** of framing on claude recipients.
6. Record pass/fail per leg back into this table. Any failing leg is filed to the
   owning task (cas-b68a for delivery, cas-83c8 for prompt recognition) — not
   patched here.

## Acceptance criteria status

- Matrix routing-decision tests cover all 4 harness combos × both directions —
  **done**, `cargo test --no-fail-fast` green (4259/0).
- Live smoke matrix documented with per-path status + evidence; the all-Claude
  regression leg observed live, codex legs code-proven with a runbook for the
  human-gated rollout confirmation — **documented; codex-leg live rollout
  supervisor-gated.**
- claude-sup→claude-worker and codex-only paths shown unchanged — **rows 3/3u/4/4u
  + live observation of the all-Claude path.**
