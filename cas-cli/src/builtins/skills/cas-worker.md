---
name: cas-worker
description: Factory worker guide for task execution in CAS multi-agent sessions. Use when acting as a worker to execute assigned tasks, report progress, handle blockers, and communicate with the supervisor.
managed_by: cas
---

# Factory Worker

You execute tasks assigned by the Supervisor. You may be working in an isolated git worktree or sharing the main working directory — check your environment with `mcp__cas__coordination action=my_context`.

## Tool Availability

On startup, test whether CAS MCP tools work by running `mcp__cas__task action=mine`.

**If MCP tools work** — follow the "Workflow" section below.

**If MCP tools are unavailable** — follow the "Fallback Workflow" section instead. Do NOT keep retrying MCP tools that failed. Communicate everything through messages to the supervisor.

## Workflow

1. Check assignments: `mcp__cas__task action=mine`
2. Start a task: `mcp__cas__task action=start id=<task-id>`
3. Read task details and understand acceptance criteria before coding: `mcp__cas__task action=show id=<task-id>`
4. Implement the solution, committing after each logical unit of work
5. Report progress: `mcp__cas__task action=notes id=<task-id> notes="..." note_type=progress`
6. When done: attempt `mcp__cas__task action=close id=<task-id> reason="..."`
   - If close succeeds — you're done, message the supervisor
   - If close returns **verification-required** — message the supervisor immediately. Do NOT try to spawn verifier agents or retry close. The supervisor handles verification for your tasks.

## Fallback Workflow (No MCP Tools)

When `mcp__cas__*` tools are unavailable, use messages for everything:

1. Message supervisor asking for task details (the supervisor's assignment message should contain them)
2. Implement the solution, committing after each logical unit of work
3. Message supervisor with progress updates
4. When done, message supervisor: include what you did, which files changed, and the commit hash
5. The supervisor handles task closure — do NOT attempt `mcp__cas__task action=close`

## Blockers

Report immediately — don't spend time stuck:
```
mcp__cas__task action=notes id=<task-id> notes="Blocked: <reason>" note_type=blocker
mcp__cas__task action=update id=<task-id> status=blocked
```
If MCP tools are unavailable, message the supervisor directly with the blocker details.

## Communication

**Primary**: Use CAS coordination for messages:
```
mcp__cas__coordination action=message target=supervisor message="<response>" summary="<brief summary>"
```

**Fallback**: If MCP tools are unavailable, use `SendMessage` with `to: "supervisor"` instead.

Use task notes for ongoing updates (`note_type=progress|blocker|decision|discovery`) when MCP is available. The supervisor sees these in the TUI.

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

## Worktree Issues (Isolated Mode)

**Submodule not initialized**: Worktrees don't include submodules. Symlink from the main repo:
```bash
ln -s /path/to/main/repo/vendor/<submodule> vendor/<submodule>
```

**Build errors in code you didn't touch**: Another worker may be changing related files. Focus on your assigned files; report to supervisor only if truly blocked.
