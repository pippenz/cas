# Worker Prompt Has No Worktree Awareness

## Summary

The cas-worker skill/prompt doesn't mention git worktrees or the CAS MCP limitation. Workers follow the standard flow (check tasks via CAS, report via CAS coordination) which fails immediately in worktrees. Every worker independently discovers the same problem and independently tries the same failed recovery steps.

## Current Worker Prompt Behavior

The cas-worker skill instructs workers to:
1. `mcp__cas__task action=mine` — check assigned tasks
2. `mcp__cas__task action=start` — claim and start work
3. `mcp__cas__task action=close` — close when done
4. `mcp__cas__coordination action=message` — communicate with supervisor

None of these work in worktrees, but the prompt doesn't say so.

## Proposed Addition to cas-worker Skill

```markdown
## Git Worktree Mode

If you are running in a git worktree (your working directory contains `.cas/worktrees/`):
- CAS MCP tools (`mcp__cas__*`) are NOT available and will not connect
- Do NOT attempt `cas init`, `cas factory`, or any cas CLI commands
- Your task details were sent by the supervisor via message — check your conversation history
- Use only built-in tools: Read, Edit, Write, Bash, Glob, Grep
- When done, commit your work and message the supervisor via SendMessage
- The supervisor will handle task management (close, verify, etc.)

To detect worktree mode on your first turn:
```bash
[[ "$PWD" == *".cas/worktrees"* ]] && echo "WORKTREE MODE" || echo "NORMAL MODE"
```
```
