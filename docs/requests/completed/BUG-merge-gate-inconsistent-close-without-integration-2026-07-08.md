# BUG: merge-gate inconsistent — tasks closed without branch push OR epic merge

- **Date:** 2026-07-08
- **Reporter:** supervisor (steady-phoenix-5), Ozer factory session `ozer-strong-crow-55`
- **Area:** factory / task-close merge gate + epic-completion signalling
- **Severity:** HIGH (tasks report done while their code is not integrated; epic
  falsely signalled complete → near-miss shipping incomplete work)

## Summary
The task-close "MERGE REQUIRED" gate fired for two workers but NOT a third,
letting that third worker close two merge-gated tasks whose commit was neither
pushed to origin nor merged into the epic branch. The director then announced
"all subtasks closed — close the epic," which — if trusted — would have shipped
the epic to staging missing two fixes.

## Observed (same epic cas-7a62, three isolated workers, identical task type)
- **bright-shark-38** and **calm-shark-81**: attempted `task action=close` →
  hard-blocked with "MERGE REQUIRED... This guard cannot be bypassed." Stayed
  open until the supervisor merged their `origin/factory/<name>` branches into
  the epic branch. Correct behavior.
- **zealous-crow-37**: closed BOTH cas-d035 and cas-51b4 successfully. But:
  - `origin/factory/zealous-crow-37` **did not exist** (never pushed), and
  - its commit `3c75cbe4` (a linked-worktree local branch, based on the epic
    base) was **not reachable from the epic branch**.
  So both tasks were marked closed with code that was neither pushed nor merged.
- The **director** then emitted: "All subtasks of epic cas-7a62 are now closed…
  Close the epic," treating task-status as integration-state.

## Impact
- Epic completion was signalled while 2 of 6 fixes (subscriptions "per once"
  copy fix; sign-in desktop contrast fix) existed only in one worker's local
  worktree. Closing the epic + PR-to-staging at that point would have shipped
  without them. Caught only because the supervisor independently verified each
  worker's commits were actually in the epic branch before closing the epic.

## Expected
1. The "MERGE REQUIRED" close gate must be **consistent** — it should block
   close for ALL workers whose branch is not merged into the epic, not just
   some. (Ideally it verifies the worker's commits are reachable from the epic
   branch tip, using origin refs, not local/worktree-only refs.)
2. Epic-completion signalling ("all subtasks closed → close the epic") should
   verify **integration** (subtask commits reachable from the epic branch), not
   merely task status, before recommending epic close.

## Hypothesis
The gate may check reachability via shared local/linked-worktree refs (which
see `factory/zealous-crow-37` locally) rather than requiring the commit to be
reachable from the epic branch itself; combined with zealous never pushing, the
gate passed on a ref that was not actually integrated. Needs confirmation.

## Workaround applied
Before closing the epic, the supervisor diffed each worker's commits against the
epic branch, discovered zealous's commit `3c75cbe4` was unpushed + unmerged,
recovered it from the linked worktree `.cas/worktrees/zealous-crow-37`, reviewed
it, and merged it into the epic (`15fc39be`). Then re-verified all 6 tasks'
changes are reachable from the epic tip before proceeding.

## Repro (approximate)
1. Spawn isolated workers on one epic; close gate = MERGE REQUIRED.
2. Have one worker commit locally but NOT push, then `task action=close`.
3. Observe whether close is (incorrectly) allowed while its commit is absent
   from the epic branch, and whether the director then signals epic-complete.

## Additional evidence — STRONGER variant (2026-07-08, gabber session `gabber-studio-sharp-hawk-84`)
A second, worse instance: worker `sturdy-finch-54` closed **TWO** merge-gated
tasks (cas-7500, cas-eaeb) while the code was **entirely UNCOMMITTED** — not just
unpushed. Its worktree `.cas/worktrees/sturdy-finch-54` was on `factory/sturdy-finch-54`
still at the epic base (`c6b0e077`), with 3 modified files + 2 untracked test
files sitting in the working tree (485 lines). Both closes succeeded; the tasks
reported done+verified. One `shutdown_workers`/worktree cleanup would have
destroyed the entire picker→reporting-tz fix + its tests, with the tasks still
marked closed.

Meanwhile, in the SAME session, other workers (`agile-gopher-92`, `wild-lion-17`)
were correctly hard-blocked by MERGE REQUIRED until the supervisor merged their
pushed `factory/*` branches into the epic. So the gate is not just origin-vs-local
inconsistent — it can be satisfied (or skipped) with **no commit object at all**.

Strengthened expectation: the close gate must verify the task's changes exist as
**committed objects reachable from the epic branch tip (via origin)** — reject
close if the worktree has uncommitted/untracked changes OR if the branch head has
no task commits OR if those commits aren't in the epic. "Verified" must never be
reported for working-tree-only edits.

Supervisor recovery here: detected via `git worktree list` + per-worktree
`status`/`log` diffing (the closed tasks had a base-commit branch head), committed
the orphaned working tree to `factory/sturdy-finch-54` (`40b6170ae`), merged to
the epic (`7f5eb8ce8`), and pushed — before any shutdown.

## Resolved (cas-4b3f)

Root cause confirmed: `resolve_worker_worktree_path` (`close_ops.rs`) only ever
consulted "System A" — the `WorktreeStore` keyed by `task.worktree_id`, which is
populated ONLY for epic-type tasks and only when `[worktrees] enabled = true`
(disabled by default). The actual day-to-day factory path — `spawn_workers
isolate=true`, one real git worktree per worker at
`<cas_root>/worktrees/<assignee>` — is never registered there, so
`task.worktree_id` was always `None` for ordinary worker tasks. Every gate keyed
off that resolution (cas-895d uncommitted-work, cas-490f commit-claim, cas-762e
merge-reality, cas-ee2b zero-commit ambiguity) silently no-opped for real
factory workers — exactly this incident.

Fix: `resolve_worker_worktree_path` now falls back to the "System B" convention
(`<cas_root>/worktrees/<assignee>`, checked for a real `.git` entry) whenever
System A doesn't resolve. Regression test:
`test_task_close_blocks_on_uncommitted_system_b_worker_worktree_cas_4b3f`
(`cas-cli/tests/mcp_tools_test/task_tools/verification_flow.rs`) reproduces this
exact shape — a task with `task.assignee` set and a real (unregistered) worktree
directory — and proves the close now hard-blocks on uncommitted work, then
succeeds once committed.
