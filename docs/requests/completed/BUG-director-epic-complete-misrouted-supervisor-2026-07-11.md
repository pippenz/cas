# BUG: director epic-completion notification delivered to a supervisor session that doesn't own the epic

**From:** Ozer supervisor session (fair-parrot-35, session 260b260a-1872-4e65-82ed-83565a3e13c5, 2026-07-11)
**Severity:** Medium-high — wrong-session supervisor burns tokens re-verifying another session's epic, and the suggested action list includes `shutdown_workers`, which in the wrong hands (count=0 = all) would kill workers belonging to the owning session. Only a human interrupt stopped the double-driving today.
**Component:** director notifications / epic-completion routing / teammate message addressing

## What happened

1. 2026-07-11 ~21:30: this session started fresh (immediately after a `/clear`). The very first thing in the transcript was a `teammate-message` from `director`: "All subtasks of epic 'EPIC: /food visual remediation …' (cas-f4ef) are now closed. Next steps: Verify the integrated result; Close the epic (`task action=close id=cas-f4ef`); Shut down idle workers if no more work."
2. This session is registered as a supervisor (fair-parrot-35) but **did not create, plan, or run epic cas-f4ef** — a different concurrently-running supervisor session did. Evidence from this session's seat: none of the epic's worker worktrees (`food-contrast`, `food-hierarchy`, `food-visual-qa`) appear in `git worktree list` here; the five worktrees present all belong to unrelated work (ayurveda-layout, visual-auditor, widget-*).
3. The message reads as an actionable instruction, so this supervisor did what any supervisor would: pulled `task show cas-f4ef` + `epic_status`, built a temp worktree at the epic tip, and ran fresh verification proofs (which passed, 82/82) — all duplicate work, since the owning session's independent verifier (cas-375d) had already verified the same tip e129ecf7 minutes earlier.
4. The operator interrupted manually: "this is not your task — director is talking about another session." Work was abandoned and the scratch worktree removed. Had the operator not been watching, this session would have proceeded to close the epic and issue `shutdown_workers` — racing the owning supervisor on both.

## The defect

The director's "all subtasks closed → verify + close + shutdown" notification is addressed by **role** (a supervisor) rather than by **ownership** (the supervisor that owns the epic). Two gaps compound:

1. **No ownership routing.** Tasks already carry an `epic_verification_owner` field (the task schema documents it as "Agent ID responsible for epic verification (supervisor in factory mode)"), and epics have a creating session. The completion notification ignored both and landed in an unrelated supervisor session — possibly "most recently active supervisor" or broadcast-to-any-supervisor semantics.
2. **The message carries no ownership context for the recipient to self-filter.** It names the epic ID and next steps but nothing that lets the receiving agent cheaply detect "not mine" (owner agent ID, originating session, worker names involved). A supervisor receiving it has no signal to bounce it; the rational move is to comply, which is exactly the failure.

## Why this is worse than wasted tokens

- `shutdown_workers` from the wrong session can terminate the owning session's live workers mid-task.
- Two supervisors can race on `task action=close id=<epic>` — best case a confusing double-close audit trail, worst case one closes while the other is still driving remediation.
- The wrong session's "verification" can diverge from the owning session's context (different flags, different local state) and produce a contradictory verdict on the same tip.

## Suggested fixes

1. **Route by owner:** deliver epic-completion notifications to the epic's `epic_verification_owner` (or, absent that, the session/agent that created the epic and spawned its workers). Fall back to broadcast only if the owner is unreachable, and say so in the message.
2. **Stamp ownership in the payload:** include `owner=<agent-id>` and originating session in the notification body so a mis-delivered copy is self-identifying and the recipient can decline ("not my epic") instead of complying.
3. **Guard the actions, not just the routing:** `task action=close` on an epic and `shutdown_workers` could warn (or require `force`) when the caller is neither the epic's owner nor its verification owner. That converts a mis-route from a destructive race into a bounced call.

## Impact if unfixed

Any multi-supervisor host (multiple concurrent factory sessions on one machine — the normal state here) will keep cross-firing completion notifications. Each mis-route costs a re-verification pass at best; at worst it closes epics and kills workers out from under the session that owns them, with no audit trail explaining why.

## Related

- BUG-worker-first-claim-stall-2026-07-07 — same theme: director/coordination messaging asserting or instructing without verifying ground truth about which agent should act.
- Project memory "Don't reassign a transiently crashed worker" — supervisors already know to distrust liveness signals; this adds "distrust addressing" to the list, which shouldn't be necessary.

---

## Resolution (cas-9fff, 2026-07-11)

**Status:** Fixed on `factory/hv-director`.

### Root cause (verified)

`DirectorData::load_with_stores` session-filters **agents** via `CAS_FACTORY_SESSION`, but loads **all** epic/task rows from the shared project DB. `EpicAllSubtasksClosed` then always targeted *this* factory session's `supervisor_name` with no ownership check. Two concurrent factory sessions therefore both fired the "verify → close epic → shutdown workers" prompt when any epic's subtasks drained.

### Fix

1. **Ownership routing** (`route_epic_completion` in `cas-cli/src/ui/factory/director/prompts.rs`):
   - Prefer `Task.epic_verification_owner` (agent id or name).
   - Else session affinity (focused epic / session workers on subtasks).
   - Foreign owner → suppress (no delivery).
   - Unreachable-owner fallback only when explicitly allowed, and stamped as such.
   - Epic present with no affinity → suppress (concurrent foreign epic).

2. **Delivery gate:** `revalidate_event_for_delivery_with_focus` drops non-owner `EpicAllSubtasksClosed` before inject; daemon passes `current_epic_id` for affinity.

3. **Payload stamp:** completion prompt always includes `OWNERSHIP: owner=… session=… source=…` so a mis-delivery is self-identifying.

4. **Sticky owner (fail closed):** factory-mode epic create resolves `epic_verification_owner` from caller identity; if identity cannot be resolved, create is **rejected** (no silent `None` that would disable routing/guard).

5. **Defense-in-depth close guard (fail closed):** `task action=close` on an epic with `epic_verification_owner` set requires a matching caller identity. **Unknown identity is a rejection**, not a fall-through.

6. **Shutdown:** existing `factory_shutdown_workers` already scopes by `CAS_FACTORY_SESSION` + `supervisor_owned_workers` — documented, not redesigned.

7. **Out of scope (tracked separately):** unguarded `epic_verification_owner` mutation surface on task update — not absorbed into this fix.

### Proof

```text
cargo test -p cas --lib -- test_9fff
# routing + close-gate + factory-create owner tests
```

Key tests: `test_9fff_two_supervisors_only_owner_gets_epic_complete_prompt`,
`test_9fff_route_prefers_epic_verification_owner`,
`test_9fff_unreachable_owner_fallback_is_explicit`,
`test_9fff_epic_complete_prompt_stamps_session_context`,
`test_9fff_unknown_caller_identity_fail_closed`,
`test_9fff_factory_epic_create_rejects_when_identity_unresolvable`.
