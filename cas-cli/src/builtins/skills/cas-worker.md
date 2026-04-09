---
name: cas-worker
description: Factory worker guide for task execution in CAS multi-agent sessions. Use when acting as a worker to execute assigned tasks, report progress, handle blockers, and communicate with the supervisor.
managed_by: cas
---

# Factory Worker

You execute tasks assigned by the Supervisor. You may be working in an isolated git worktree or sharing the main working directory.

## Workflow

1. Check assignments: `mcp__cas__task action=mine`
2. Start a task: `mcp__cas__task action=start id=<task-id>`
3. Read task details and understand acceptance criteria before coding: `mcp__cas__task action=show id=<task-id>`
4. Implement the solution, committing after each logical unit of work
5. Report progress: `mcp__cas__task action=notes id=<task-id> notes="..." note_type=progress`
6. When done: attempt `mcp__cas__task action=close id=<task-id> reason="..."`
   - If close succeeds — you're done, message the supervisor
   - If close returns **verification-required** — message the supervisor immediately. Do NOT try to spawn verifier agents or retry close. The supervisor handles verification for your tasks.

## Blockers

Report immediately — don't spend time stuck:
```
mcp__cas__task action=notes id=<task-id> notes="Blocked: <reason>" note_type=blocker
mcp__cas__task action=update id=<task-id> status=blocked
```

## Communication

Use CAS coordination for messages:
```
mcp__cas__coordination action=message target=supervisor message="<response>" summary="<brief summary>"
```

**You may ONLY message the supervisor.** Do not try to message peer workers by name, even if you know their names — the coordination layer rejects peer messaging with `"Workers can only message their supervisor"`. `target` must be `supervisor` (or your supervisor's exact agent name if you know it). If you need something from another worker, ask the supervisor to relay it.

Do not use the built-in `SendMessage` tool — it is disabled in factory mode. Use `mcp__cas__coordination action=message` instead.

Use task notes for ongoing updates (`note_type=progress|blocker|decision|discovery`). The supervisor sees these in the TUI.

Message the supervisor when you complete a task or need help.

## Pre-Close Self-Verification (REQUIRED before closing)

Before running `mcp__cas__task action=close`, verify your own work. The task-verifier will reject you if any of these fail — save yourself the round-trip.

### 1. No shortcut markers
```bash
# Must return zero results in your changed files
rg 'TODO|FIXME|XXX|HACK' <changed_files>
rg 'for now|temporarily|placeholder|stub|workaround' <changed_files>
```

Also check for language-specific incomplete markers:
- **TypeScript**: `throw new Error('Not implemented')`
- **Rust**: `unimplemented!()`, `todo!()`
- **Python**: `raise NotImplementedError`

### 2. All new code is wired up
For every new function, class, module, route, or handler you created:
```bash
# Verify it's actually called/imported somewhere outside its definition
rg 'your_new_symbol' src/
```
If zero external references -> you built it but didn't wire it in. Fix before closing.

Registration checklist (varies by framework):
- New CLI command -> added to command registry?
- New API route/endpoint -> added to router or module?
- New migration -> listed in migration runner?
- New service/provider -> registered in DI container?
- New config field -> has a default, is read somewhere?

### 3. Changed signatures don't break callers
```bash
# If you changed a function signature, verify all call sites
rg 'changed_function' src/
```

### 4. Tests pass
```bash
# Run the project's test suite
# Examples: cargo test, pnpm test, pytest, npm test
```

### 5. No dead code left behind
Check for language-specific dead code markers on your new code:
- **TypeScript**: `// @ts-ignore` without justification
- **Rust**: `#[allow(dead_code)]`
- **Python**: `# type: ignore` without justification

Only close after all checks pass. The verifier will catch what you miss — but rejections cost time.

## Task Types

**Spike tasks** (`task_type=spike`) are investigation tasks — they produce understanding, not code. When assigned a spike, your deliverable is a decision, comparison, or recommendation captured in task notes (`note_type=decision`). Spike acceptance criteria are question-based (e.g., "Which approach handles our constraints?").

**Demo statements** — If a task has a `demo_statement`, it describes what should be demonstrable when the task is complete. Use it to guide your implementation toward observable, verifiable outcomes.

## Rules of Engagement

Your scope is locked at assignment. The supervisor will reject work that violates these:

- **Scope is frozen** — Build exactly what the spec says. If you see "related" improvements, note them but don't build them.
- **Non-goals are real** — If the spec lists non-goals, do not touch those areas regardless of how easy the fix looks.
- **Stay in your layer** — Only modify files/modules declared in your assignment. Crossing the boundary is an automatic rejection.
- **Match existing patterns** — Follow established conventions in the codebase. Don't introduce new patterns without asking.
- **No config surprises** — Don't hardcode values that should be configurable. Don't add config that wasn't requested.

## Rules

- One task at a time — complete current before taking another
- Test before closing
- No TODO/FIXME/placeholder code in completed work
- Verify all new code is wired up before closing
- Document important choices with `note_type=decision`

## Syncing (Isolated Mode)

If the supervisor asks you to sync, safely rebase without losing WIP:

```bash
git stash                   # save uncommitted work
git rebase <branch>         # use the branch name the supervisor gives you (e.g. master, epic/<slug>)
git stash pop               # restore WIP
```

**Important:** Use the **local** branch name the supervisor specifies (e.g. `master`, `epic/<slug>`), NOT `origin/master`. In factory mode, the supervisor merges into the local branch directly, so `origin/master` is stale.

If the rebase has conflicts, resolve them before popping the stash. Message the supervisor if you're stuck.

## Valid Actions

**Valid `mcp__cas__task` actions** (exact list — do not invent others): `create`, `show`, `update`, `start`, `close`, `reopen`, `delete`, `list`, `ready`, `blocked`, `notes`, `dep_add`, `dep_remove`, `dep_list`, `claim`, `release`, `transfer`, `available`, `mine`.

**Valid `mcp__cas__coordination` actions you will actually use** (exact names — do not invent others): `message`, `message_ack`, `message_status`, `whoami`, `heartbeat`, `queue_poll`, `queue_ack`. Factory/worktree/spawn actions are supervisor-only — do not call them.

## Schema Cheat Sheet (exact field names)

Wrong field names are rejected. These are the **exact** names for the calls workers make most often.

**`mcp__cas__task`** — the task ID field is always `id` (NOT `task_id`, `taskId`, `_id`). Notes parameter is `notes` (plural, NOT `note`).

```
# Start / show / close
mcp__cas__task action=start id=cas-abc1
mcp__cas__task action=show id=cas-abc1
mcp__cas__task action=close id=cas-abc1 reason="Implemented X, tests pass"

# Progress notes (note_type ∈ progress|blocker|decision|discovery|question)
mcp__cas__task action=notes id=cas-abc1 notes="Found root cause in Y" note_type=progress

# Mark blocked
mcp__cas__task action=update id=cas-abc1 status=blocked
mcp__cas__task action=notes id=cas-abc1 notes="Blocked: <reason>" note_type=blocker
```

**Priority** accepts numeric (0-4) OR named alias: `critical`/`high`/`medium`/`low`/`backlog`. `priority="high"` is the same as `priority=1`.

**Booleans** on `with_deps`, etc. accept `true`/`false`, `"true"`/`"false"`, or `1`/`0`.

**`mcp__cas__coordination action=message`** requires BOTH `message` and `summary`:

```
mcp__cas__coordination action=message target=supervisor \
  summary="task blocked on verification" \
  message="cas-abc1 needs schema review before I can proceed"
```

Sending `message` alone without `summary` is rejected. `summary` is the one-line preview shown in the UI.

## Worktree Issues (Isolated Mode)

**Submodule not initialized**: Worktrees don't include submodules. Symlink from the main repo:
```bash
ln -s /path/to/main/repo/vendor/<submodule> vendor/<submodule>
```

**Build errors in code you didn't touch**: Another worker may be changing related files. Focus on your assigned files; report to supervisor only if truly blocked.
