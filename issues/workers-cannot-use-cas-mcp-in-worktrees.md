# Workers Cannot Use CAS MCP Tools in Git Worktrees

## Summary

When the supervisor spawns isolated workers (`isolate=true`), each worker gets its own git worktree under `.cas/worktrees/<worker-name>/`. The CAS MCP server fails to connect in these worktree sessions, leaving workers unable to use `mcp__cas__task`, `mcp__cas__coordination`, `mcp__cas__memory`, or any CAS MCP tool. Workers waste multiple turns trying to bootstrap CAS (running `cas init -y`, `cas factory agents`, `cas factory message`) before finally being told to use regular tools.

## Severity

**High** — This affects every factory session with isolated workers. In today's session alone, 9+ workers across 3 spawn batches all hit this issue. It wastes significant time and tokens on every spawn cycle.

## Reproduction

1. Start a factory session as supervisor
2. `mcp__cas__coordination action=spawn_workers count=3 isolate=true`
3. Workers spawn in `.cas/worktrees/<name>/`
4. Every worker reports: "MCP tools (mcp__cas__*) are unavailable despite .mcp.json being present"
5. Workers then try `cas init -y --force`, `cas factory agents`, `cas factory message` — all fail with:
   ```
   [ERROR] No running factory sessions found for project '/home/pippenz/Petrastella/ozer/.cas/worktrees/<worker-name>'.
   Try 'cas list'.
   ```
6. Workers are stuck until supervisor manually redirects them to use regular tools (Read, Edit, Bash, etc.)

## Root Cause Analysis

The `.mcp.json` file exists in the worktree (copied or symlinked from the main repo), and `cas serve` starts, but the MCP server tools never become available in the worker's Claude Code session. Possible causes:

1. **Session registration mismatch** — The CAS MCP server registers against the project path. In a worktree, the project path is `.cas/worktrees/<name>/` which doesn't match the main project path where the factory session is registered.

2. **Factory session scoping** — `cas factory` commands scope to the project path. The worktree path is a different directory, so factory commands can't find the running factory session.

3. **MCP server startup timing** — The MCP server may start but fail to register tools before the worker's first turn, and there's no retry mechanism.

## Impact

- Workers cannot check their assigned tasks (`mcp__cas__task action=mine`)
- Workers cannot close tasks or record progress
- Workers cannot message the supervisor via `mcp__cas__coordination`
- Workers waste 2-4 turns per spawn trying to connect before being redirected
- Supervisor must repeat task details in plain text messages
- Task verification/close flow is broken (workers can't self-verify)

## Current Workaround

Supervisor sends detailed task instructions via `mcp__cas__coordination action=message` and tells workers to ignore CAS MCP tools entirely. Workers use only built-in tools (Read, Edit, Write, Bash, Glob, Grep). Supervisor handles all CAS task management centrally.

This works for implementation tasks but breaks the verification/close flow, since CAS requires the worker (not supervisor) to verify and close individual tasks.

## Proposed Fixes

### Option A: Fix MCP server project path resolution in worktrees
Make the CAS MCP server detect that it's running inside a git worktree and resolve to the main repository's project path for session registration and factory commands.

```
# In a worktree, git provides:
git rev-parse --git-common-dir  # → /home/pippenz/Petrastella/ozer/.git
# Use this to find the real project root instead of cwd
```

### Option B: Worker prompt/system instructions
Add a hard rule to the worker system prompt:
```
IMPORTANT: CAS MCP tools (mcp__cas__*) do NOT work in git worktrees.
Do NOT attempt to use them, run 'cas init', or 'cas factory' commands.
Use only built-in tools: Read, Edit, Write, Bash, Glob, Grep.
Your task details are in the supervisor's message — scroll up.
```

### Option C: Shared-mode workers (no worktrees)
Use `isolate=false` so workers share the main working directory. CAS MCP tools would work since they're in the main project path. Requires more careful file-overlap coordination.

### Option D: Symlink .cas session data into worktrees
When creating a worktree, symlink the `.cas/` session directory so the MCP server in the worktree can find the active factory session.

## Recommended

Option A is the proper fix. Option B is a quick mitigation that should be applied immediately regardless. Option C is an acceptable fallback for smaller task sets.

## Environment

- CAS version: 1.1.0 (dbd830e-dirty 2026-03-24)
- Claude Code model: claude-opus-4-6 (1M context)
- OS: Linux 6.17.0-19-generic
- Git: worktrees created via `git worktree add`
- Affected sessions: d6397c6f-fc54-4069-bc2d-9ebaa91aa8c6 (and all prior factory sessions)
