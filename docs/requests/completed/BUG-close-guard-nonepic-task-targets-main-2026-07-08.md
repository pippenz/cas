# BUG: close guard for NON-epic-scoped tasks targets `main`, blocking legitimate closes (chore/bug/review tasks)

- **Date:** 2026-07-08
- **Reporter:** supervisor (quiet-tiger-24), gabber-studio factory session `gabber-studio-sharp-hawk-84`
- **Area:** task-close MERGE-REQUIRED guard — merge-target resolution for tasks with no epic parent
- **Severity:** MEDIUM (recurring — forced ~6 supervisor `status=closed` workarounds in one session; the workaround bypasses the guard entirely, eroding its value)

## Summary
The MERGE-REQUIRED close guard resolves its merge TARGET from the task's epic
(via the ParentChild dependency). For a task with **no epic** (a standalone
chore/bug/review created mid-run), the guard **defaults the target to `main`**.
That is wrong whenever the task's work legitimately lives on an **epic branch**
or is **docs/review-only**: the guard demands the worker's factory branch be
merged into `main`, which (a) violates staging-first (never PR features straight
to main) and (b) is impossible for a read-only review task that made no code
changes. The worker cannot resolve it; every such task requires a supervisor
`task update status=closed` override.

## Observed (this session — 6 instances)
All were non-epic-scoped tasks whose close bounced with `MERGE_REQUIRED` citing
**`main`** (not the epic branch), despite the work being committed + merged onto
the correct epic branch (or being a pure review):

| Task | Type | Actual state | Guard demanded |
|------|------|--------------|----------------|
| cas-7814 | chore (release-notes draft) | committed, merged onto epic `cas-2abd` | merge to `main` |
| cas-f115a | chore (release-notes draft) | committed, merged onto epic `cas-8bed` | merge to `main` |
| cas-4014e | chore (read-only review, ZERO code) | findings in notes, no commits | merge to `main` |
| cas-50c1 | chore (read-only review) | findings in notes, no commits | merge to `main` |
| cas-ce27 | bug (error-feed caption) | committed, merged onto epic + staging | merge to `main` (27 commits ahead) |
| cas-ca33 | task (webhook) | code merged onto epic/staging | merge to `main` |

The sibling behavior for **epic-scoped** tasks is correct: they target the epic
branch and clear once the supervisor merges + pushes the epic branch (origin).
The defect is specifically the **no-epic → `main`** fallback.

## Impact
- Every standalone review/chore/bug task filed during a factory run is
  un-closeable by its worker and must be force-closed by the supervisor via the
  `status=closed` workaround (which skips verification/integration checks
  entirely — see BUG-merge-gate-inconsistent).
- A **read-only review task** can NEVER satisfy a merge guard (it has no commits
  to merge), yet the guard still blocks it. Review/analysis tasks should be
  exempt from the merge gate outright.

## Expected
1. A task with no epic should NOT default its merge target to `main`. Prefer:
   the branch the task was worked on, the current integration branch (e.g.
   `staging`), or — if none is inferable — treat merge-target as unset and skip
   the merge gate rather than assuming `main`.
2. Review/analysis/docs tasks (no reviewable code diff) should be exempt from the
   merge-required gate — closeable on their findings/notes alone.
3. If a first-class "supervisor close-with-reason" exists (see
   BUG-close-guard-branch-head), route these through it instead of teams
   re-discovering `status=closed`.

## Related
- BUG-close-guard-branch-head-not-task-commits.md (wrong git anchor on close)
- BUG-merge-gate-inconsistent-close-without-integration-2026-07-08.md (the
  `status=closed` workaround bypasses integration checks)

## Resolved (cas-4b3f)

Implemented Expected #1 and #2 directly: `cas_task_close` no longer defaults
the merge-gate's `parent_branch` to `"main"` when `get_parent_epic` returns
`None` — it now skips the merge-state gate outright (treats the target as
unset) instead of guessing. Additionally, non-code-expecting task types
(`Chore`/`Spike` — the same classification `check_zero_commit_close` already
used elsewhere in this file) are exempt from the gate outright, satisfying
"review/docs tasks close on notes alone" for that subset of cases. Expected
#3 (first-class supervisor close-with-reason) was already covered by the
existing `bypass_code_review=true` override path and is unchanged here.

Regression test: `test_nonepic_task_does_not_default_merge_target_to_main_cas_4b3f`
(`cas-cli/tests/mcp_tools_test/task_tools/verification_flow.rs`) reproduces a
standalone task whose work is committed on a real integration branch that
isn't `main`, and proves the close no longer false-rejects with
`MERGE REQUIRED`.
