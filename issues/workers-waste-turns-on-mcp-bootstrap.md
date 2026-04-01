# Workers Waste 2-4 Turns Trying to Bootstrap CAS Before Working

## Summary

Every worker's first action is to try `mcp__cas__task action=mine` or `cas factory agents` to check their tasks. When CAS MCP is unavailable (see worktree issue), they spend 2-4 turns attempting various recovery strategies before the supervisor redirects them to just use regular tools. This pattern repeats identically for every worker in every spawn batch.

## Observed Recovery Attempts (in order)

1. `mcp__cas__task action=mine` — fails (tools not loaded)
2. `cas factory agents` via Bash — fails (wrong project path)
3. `cas list --json` via Bash — finds no sessions
4. `cas factory message --target supervisor` via Bash — fails
5. `cas init -y --force` via Bash — runs but doesn't fix MCP
6. Reports to supervisor: "MCP tools unavailable, awaiting instructions"
7. Supervisor sends: "Ignore CAS, use regular tools, your tasks are in my previous message"
8. Worker finally starts working

## Token/Time Cost

- ~3,000-5,000 tokens wasted per worker on bootstrap attempts
- ~30-60 seconds of wall time per worker
- With 3 workers per batch and 3 batches today: ~9 workers x ~4,000 tokens = ~36,000 tokens wasted
- Supervisor also wastes tokens sending "stop trying CAS" messages

## Proposed Fix

The worker system prompt (cas-worker skill) should include:

```
IF you are in a git worktree (.cas/worktrees/):
  - CAS MCP tools WILL NOT work. Do not attempt to use them.
  - Do not run cas init, cas factory, or any cas CLI command.
  - Your task details are in the supervisor's message.
  - Start working immediately with Read, Edit, Write, Bash, Glob, Grep.
```

Workers should detect they're in a worktree on first turn (`git rev-parse --is-inside-work-tree` + check if cwd contains `.cas/worktrees/`) and skip all CAS bootstrap.
