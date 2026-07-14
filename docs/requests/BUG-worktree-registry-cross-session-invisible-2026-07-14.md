---
from: Ozer supervisor (pippenz @ /home/pippenz/Petrastella/ozer)
date: 2026-07-14
priority: P2
---

# BUG: `coordination worktree_list` returns "No worktrees found" while CAS-created worktrees from sibling sessions exist

## Summary

While integrating epic `cas-ea3e`, `git worktree list` showed three worktrees:

```
/home/pippenz/Petrastella/ozer                           b3022aa6 [staging]
/home/pippenz/Petrastella/ozer/.cas/worktrees/hv-food-qa a90f98a6 [factory/hv-food-qa]
/tmp/ozer-epic-ea3e-hv                                   ec0087ce [epic/…-cas-ea3e]
```

Two were created by CAS factory tooling in *other* sessions (a director session and its worker). Yet `mcp__cas__coordination action=worktree_list` in my session returned **"No worktrees found."** — so `worktree_show` / `worktree_merge` / `worktree_cleanup` were unusable for exactly the cleanup the factory pattern says the supervisor should run. I had to fall back to raw `git worktree remove` / `branch -D` / `push --delete`.

## Environment

- `cas 2.27.0 (dd8bcbd-dirty 2026-07-11)`, factory mode, supervisor `fast-kestrel-14`, session `07275a32-c0d5-4695-abbb-5c04663df721`, project `/home/pippenz/Petrastella/ozer`
- Worktrees created by a concurrent director session (same project, same `.cas` root)

## Expected

The worktree registry should be project-scoped (keyed under `.cas/` for the repo), not session-scoped — any supervisor in the project should see and be able to manage worktrees created by sibling/predecessor sessions. Alternatively, `worktree_list` could reconcile against `git worktree list` and report unregistered CAS-pattern worktrees (`.cas/worktrees/*`, epic worktrees) as "untracked".

## Impact

Cross-session handoffs (director spawns workers, supervisor integrates — exactly the flow the director requested of me) lose all worktree bookkeeping: no merge-state tracking, no policy checks before cleanup, and orphaned worktrees accumulate invisibly unless someone happens to run raw git commands.
