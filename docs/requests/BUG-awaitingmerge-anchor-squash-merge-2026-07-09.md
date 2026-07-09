---
from: Ozer factory worker food-route (relay)
date: 2026-07-09
priority: P1
cas_task: cas-24fc
---

# BUG: AwaitingMerge `factory_branch_anchor` permanently rejects squash-merged tasks

- **Date:** 2026-07-09
- **Reporter:** worker `food-route` + supervisor `golden-merlin-17`, Ozer factory session
- **Incident task:** cas-ac7c (closed via audited status-close recovery)
- **Area:** factory task-close merge-state gate / `park_task_awaiting_merge` / `factory_branch_anchor`
- **Severity:** HIGH — once parked, a correctly squash-merged task cannot pass `task action=close` for worker **or** supervisor; forces audited status-close that bypasses the gate

## Summary

cas-4b3f introduced `TaskDeliverables.factory_branch_anchor` so serial tasks on one `factory/<worker>` branch do not re-strand an earlier close when a later task advances HEAD. `park_task_awaiting_merge` snapshots the factory tip as **anchor A** on first MERGE REQUIRED.

That design breaks for the common **GitHub squash-merge** path:

1. Worker commits task work as **A** on `factory/<name>` and pushes.
2. Worker (or supervisor) opens a PR into the integration branch (`staging` / epic / `main`) and **squash-merges** → integration tip becomes **B** (new SHA; **A** is not an ancestor of the integration tip).
3. Worker force-aligns `factory/<name>` to the same **B** (`git reset --hard origin/<integration>` / re-push). Live branch has **zero commits ahead** of integration.
4. Task remains **AwaitingMerge**. Close still evaluates stranded commits against stored **`factory_branch_anchor=A`**, which is not reachable from the integration tip after squash.
5. Worker close and supervisor close both return `⚠️ MERGE REQUIRED` forever.
6. Only recovery is audited `status=closed` (or equivalent lifecycle override), which skips the intended merge-state gate.

This is the inverse failure of the cas-4b3f bug: the anchor correctly protects serial-task A from task B’s live HEAD, but **never converges** when A itself is integrated via squash (SHA rewrite).

## Reproduction (observed live — cas-ac7c)

| Step | Evidence |
|------|----------|
| Factory commit A | `1a88a89c` on `factory/food-route` — `fix(frontend): home Food tile navigates to /food/scan (cas-ac7c)` |
| PR squash-merge | [Richards-LLC/ozer-health#683](https://github.com/Richards-LLC/ozer-health/pull/683) → `staging` |
| Integration tip B | `93ada485` — same subject line with `(#683)` squash commit on `origin/staging` |
| Factory ref aligned | `factory/food-route` force-updated to `93ada485`; `origin/staging..factory/food-route` empty; `git merge-base --is-ancestor HEAD origin/staging` success for tip B |
| Gate result | `cas__task action=close` → `MERGE REQUIRED: factory/food-route has 1 commit(s) not on staging` (repeated after align) |
| Start while parked | `cas__task action=start` → rejected: task is AwaitingMerge |
| Supervisor | Normal close also false-rejected against pre-squash anchor A; forced lifecycle completion with audit decision note + verification `ver-a2cf85a3e3e0` |
| Fresh proof (product) | `npm run test:run -- pages/home.spec.ts` exit 0 (8/8); `npm run typecheck` exit 0 — product was fine; gate was wrong |

## Impact

- **Permanent false rejection** after correct squash integration + live-ref alignment.
- Workers loop on merge remediation; supervisors re-discover status-close override (same class of workaround cas-4b3f tried to reduce).
- Undermines confidence in MERGE REQUIRED (real unmerged work becomes indistinguishable from squash-SHA drift).
- Blocks factory throughput on any project that squash-merges PRs into staging/main/epic (Ozer default).

## Actual behavior

- First close rejection parks task and stores `factory_branch_anchor = <factory tip A>`.
- Subsequent close uses ancestry of **A** vs integration target (not live factory HEAD when anchor is trusted in AwaitingMerge — see cas-cf64 anchor trust rules).
- Squash rewrites A→B; A is not an ancestor of B; gate never clears even when factory HEAD == integration tip == B.

## Expected behavior

1. **Squash-equivalent integration clears the gate:** if the task’s deliverable diff is present on the integration branch (or the parked anchor’s patch is cherry-equivalent / empty `git cherry` / empty `git range-diff` vs integration), close succeeds.
2. **Safe live-ref convergence clears the gate:** if `factory/<worker>` tip is an ancestor of (or equal to) the integration tip **and** has zero commits ahead, the stranded-commit check should not keep failing solely because a *prior* parked anchor A is not in the ancestry graph.
3. **Serial-task anchor protection remains:** if task A is parked with anchor A and the same factory branch later advances with task B’s unmerged commits, task A’s close must still evaluate **A** (or A’s tree), not B’s HEAD — preserve cas-4b3f intent.
4. **Genuinely unmerged commits still reject:** unpushed, unmerged, or uncommitted factory work must continue to hard-block close.

## Related completed requests / fixes

| ID | Doc / area | Relationship |
|----|------------|--------------|
| **cas-4b3f** | `docs/requests/completed/BUG-close-guard-branch-head-not-task-commits.md` | Introduced `factory_branch_anchor` + park-on-first-reject so serial tasks don’t re-strand earlier close. **This regression is a side effect of that anchor being permanent across squash.** |
| **cas-4b3f** | `docs/requests/completed/BUG-merge-gate-inconsistent-close-without-integration-2026-07-08.md` | System-B worktree resolution; same close_ops gate family. |
| **cas-4b3f** | `docs/requests/completed/BUG-close-guard-nonepic-task-targets-main-2026-07-08.md` | Non-epic integration target resolution (staging/main) — same gate, different failure mode. |
| **cas-cf64** | `close_ops.rs` + `verification_flow.rs` (no dedicated BUG md) | Anchor freshness / trust only while `status == AwaitingMerge`; reopen clears anchor; standalone-task backstop. **Does not handle squash A↛B convergence while still AwaitingMerge.** |

Primary code touchpoints (for implementers; **do not change in this request PR**):

- `cas-cli/src/mcp/tools/core/task/lifecycle/close_ops.rs` — `park_task_awaiting_merge`, `run_factory_branch_merge_gate`, anchor resolution
- `cas-cli/tests/mcp_tools_test/task_tools/verification_flow.rs` — cas-4b3f / cas-cf64 regression tests
- `crates/cas-types/src/task.rs` — `factory_branch_anchor` field

## Suggested fix directions (non-binding)

- On gate evaluation, if live factory tip is fully merged (0 commits ahead of integration) **and** equals integration tip (or is ancestor), clear/refresh the parked anchor and allow close.
- Or: treat anchor as “content claim” — if `git patch-id` / tree of anchor matches a commit reachable from integration, pass.
- Or: when recording the PR squash merge commit B on the factory branch, update `factory_branch_anchor` to B (or clear it) via an explicit “merge acknowledged” path.
- Add regression test: park with tip A → squash-merge to B on integration → reset factory to B → `task close` must succeed; and keep cas-4b3f serial-task test green.

## Regression acceptance criteria (for the future CAS fix)

1. Repro scenario above (A → squash B → factory==B, 0 ahead) → `task close` succeeds without status override.
2. cas-4b3f serial-second-task scenario still parks/rejects correctly for the first task while second task’s commits are unmerged.
3. Unmerged commits still produce MERGE REQUIRED.
4. No change to non-factory close paths beyond the gate convergence logic.

## Out of scope for this request PR

- Implementing the CAS fix (triage in cas-src separately).
- Any Ozer product code changes.
- Changing cas-ac7c history (already closed with audit note).

## Canonical report only

This file is the durable inbox report for cas-24fc. Implementation is intentionally deferred.
