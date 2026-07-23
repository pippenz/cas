# BUG: task-close gates fire on non-session / non-code content (3 distinct false-positive classes in one day)

**Date:** 2026-07-11
**Reporter:** supervisor session (ozer project, epics cas-dcae / cas-8936 / cas-1cc5)
**Severity:** medium — every instance needs a supervisor override or source-side workaround; erodes trust in the gates

## Resolution

All three false-positive classes are resolved. Class 1's remaining
post-MERGE-REQUIRED case (cas-0a2d) was fixed by cas-3f7f: when a parked
`factory_branch_anchor` is already integrated, additive-only validation now
diffs the supervisor merge against its first parent. Pre-existing epic changes
are excluded while genuine modified, deleted, or renamed files introduced by
the task still fail. Regression coverage reproduces both outcomes.

## Class 1 — additive-only + lint gates scan branch baseline, not session deltas

- **Where:** cas-a88c (host-only emulator setup, ZERO commits, clean worktree).
- **Symptom 1:** close rejected `ADDITIVE-ONLY VIOLATION` listing hundreds of backend/frontend `M` files — the factory branch's baseline diff vs epic parent, none of it session work.
- **Symptom 2 (after execution_note cleared):** close rejected by lightweight lint "Lines +5–+10: 6 consecutive commented-out lines" — again from baseline content the task never touched.
- **Expected:** gates on a task close should evaluate the task's own commits (or working-tree delta), not the whole branch-vs-parent diff.

## Class 2 — code-review gate demands envelope for tasks/epics with no reviewable diff

- **Where:** epic cas-dcae close (`CODE_REVIEW_REQUIRED`) — epic whose entire output was host SDK/AVD state + task-note runbooks; also observed on cas-a88c retry.
- **Expected:** zero-commit closes should skip the review-envelope requirement instead of forcing `bypass_code_review`.

## Class 3 — "consecutive commented-out lines" heuristic flags XML block doc-comments

- **Where:** cas-0f9c close on epic branch @ 42140bfd; `res/values/colors.xml` lines 2–10.
- **Symptom:** a standard XML `<!-- ... -->` file-header doc comment (>5 lines) is reported as commented-out code.
- **Note:** XML has no line comments — every doc header is a block comment; any Android resource file with a decent header trips this. Also unclear why a *task* close lints a file the task's commits didn't touch (overlaps Class 1).
- **Workaround applied:** compacted comments to ≤5 lines (epic commit f6991142) — which is backwards pressure: the lint is now discouraging documentation.

## Class 3 addendum — lint pins to the task-tagged commit; follow-up fixes are invisible

- After compacting the flagged comment at epic HEAD (f6991142), cas-0f9c close STILL failed with
  the identical finding: the lint evaluates the merge commit tagged to the task (42140bfd),
  whose own diff contains the pre-fix content. No worker-reachable resolution exists — fixing
  the tree doesn't count, and rewriting a pushed merge commit is destructive. Only path was
  supervisor `bypass_code_review`.
- **Ask 3b:** when a close gate fails, re-evaluate against the current branch tree (or allow
  associating a follow-up fix commit with the task), not the frozen task-tagged commit diff.

## Related cosmetic defect

- Close output prints a "Committed diff stat (vs main)" of ~1,000 lines for a single-feature
  task on a staging-based epic — same wrong-baseline confusion (staging is far ahead of main),
  and it blows past MCP output limits.

## Asks

1. Scope close-gate diffing to the closing task's own commits (or explicit session delta), not branch-vs-parent.
2. Skip code-review envelope when the computed (correctly-scoped) diff is empty.
3. Teach the commented-out-code heuristic that XML/HTML block comments at file head are documentation (or exempt `<!-- -->` entirely in `*.xml`).

## Class 3, second instance (cas-9e08) — finding lacks file attribution; frozen diff confirmed empirically

- Same "Lines +5–+10: 6 consecutive commented-out lines" finding, byte-identical across 3 close
  retries, INCLUDING after two real comment-shortening commits pushed in between — empirical
  proof the gate scans a frozen/foreign diff rather than the worker's current commits.
- The finding names line numbers but NO file path, so the worker cannot even locate what is
  flagged; this induced two pointless appeasement commits before they correctly stopped.
- **Ask 3c:** include the file path in lint findings.
