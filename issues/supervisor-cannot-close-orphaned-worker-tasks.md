# Supervisor Cannot Close Tasks From Dead Workers

## Summary

When workers from a previous session die (session ends, worktree cleaned up), their in-progress tasks become orphaned. The supervisor cannot claim or close these tasks because CAS enforces "supervisors cannot claim non-epic tasks." The only options are to reassign to a new worker or delete the task.

## Reproduction

1. Session 1: Supervisor spawns workers, assigns tasks
2. Workers complete work, commit code, add progress notes
3. Session ends before workers close tasks (or workers hit verification jail)
4. Session 2: Supervisor resumes, finds tasks still in_progress with dead assignees
5. `mcp__cas__task action=claim id=<task>` → "Supervisors cannot claim non-epic tasks"
6. `mcp__cas__task action=close id=<task>` → "VERIFICATION REQUIRED"
7. Supervisor must spawn new workers just to formally close already-done tasks

## Impact

- In today's session: 8 tasks from previous workers needed closure. Required spawning workers + multiple message rounds just to close verified work.
- Tasks sit in limbo between sessions
- Task history gets cluttered with stale in_progress items

## Proposed Fix

Allow supervisors to close tasks that have no active assignee (assignee's agent is not heartbeating). If the work is done and the worker is dead, the supervisor should be able to close it with a reason.

Or: automatically release task assignments when a worker's session ends, so the task returns to "open" state and can be reassigned cleanly.
