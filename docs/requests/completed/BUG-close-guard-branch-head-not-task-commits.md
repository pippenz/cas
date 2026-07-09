# BUG: task-close merge guard checks branch HEAD, not the task's own commits — serial tasks on one worker branch deadlock the earlier close

- **Date:** 2026-07-08 · **Session:** gabber-studio factory `gabber-studio-sharp-hawk-84`
- **Severity:** Medium — blocks the intended one-worker-many-tasks workflow; forces supervisor force-closes that bypass the guard entirely

## Reproduction (observed live)

1. Worker completes task A (cas-b451) on `factory/worker-viz`; supervisor merges those commits into the epic branch. Close attempt correctly transitions once... except:
2. The same worker starts task B (cas-33af) **on the same branch** (same file domain, same worker — the natural continuation), pushes Phase-1 commits.
3. Worker retries close on task A → `MERGE REQUIRED` again: the guard evaluates "is the branch HEAD an ancestor of the epic/target", and the branch HEAD is now task B's unmerged commit. Task A's own commits ARE merged; the guard cannot see that.
4. No worker-side resolution exists short of never starting task B before task A's close ceremony fully completes — which serializes on supervisor merge latency.

## Knock-on defect (same afternoon)

The supervisor's fallback — force-close via `task update status=closed` — triggered the pre-close backend-typecheck hook, which ran against an **unrelated worker's dirty worktree** (`.cas/worktrees/worker-pulse/apps/backend`, mid-edit, transient TS error) and blocked the close of a *frontend* task. The hook appears to pick a worktree by recency rather than by the closing task's own branch/worktree. Two independent guards both anchored to the wrong git context.

## Suggested fixes

- Close guard: verify the task's own commits are merged — e.g. record the commit range (or `cherry`-equivalence) attributed to the task at close-attempt time, or accept `git merge-base --is-ancestor <task-tip-at-first-close-attempt> <target>`. Branch HEAD is the wrong anchor whenever branches carry >1 task serially.
- Pre-close hooks must run in the closing task's own worktree/branch context (or the canonical checkout at the merged target ref) — never "most recently active worktree".
- Provide a first-class supervisor close-with-override that records WHY (audit note), instead of teams re-discovering the status=closed workaround.

## Resolved (cas-4b3f)

Implemented exactly the first suggested fix: `TaskDeliverables` gained a
`factory_branch_anchor: Option<String>` field (JSON-blob column, no migration
needed). `park_task_awaiting_merge` snapshots the worker's `factory/<assignee>`
branch tip onto the task the FIRST time the merge-state gate rejects it (the
only call site already guarded to fire once per task).
`run_factory_branch_merge_gate` then anchors its stranded-commit check to that
recorded sha (falling back to the live branch name if the anchor is stale/
missing) instead of the branch's current HEAD. Regression coverage: unit test
`serial_second_task_on_same_branch_does_not_restrand_first_tasks_close` +
full-stack integration test
`test_serial_second_task_on_same_branch_does_not_restrand_first_close_cas_4b3f`
(`cas-cli/tests/mcp_tools_test/task_tools/verification_flow.rs`) reproduce this
doc's exact scenario end to end through `cas_task_close`.

The pre-close-hook-picks-wrong-worktree issue described above is a distinct
defect (not in `close_ops.rs`'s merge-state guard) and is out of scope for this
fix — filed separately if not already tracked.
