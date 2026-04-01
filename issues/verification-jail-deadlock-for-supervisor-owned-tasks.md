# Verification Jail Deadlock When Supervisor Completes Tasks Directly

## Summary

When the supervisor implements a task directly (because workers are unavailable or the task is trivial), `mcp__cas__task action=close` triggers verification jail. The supervisor spawns a task-verifier subagent, which approves the work, but cannot record the verification because CAS enforces that only workers (not supervisors or their subagents) can verify individual tasks. The close attempt then fails again with the same verification requirement — an infinite loop.

## Reproduction

1. Supervisor implements a task directly (e.g., adding two SYNC comments to files)
2. Supervisor calls `mcp__cas__task action=close id=<task> reason="..."`
3. CAS returns: "VERIFICATION REQUIRED — spawn a task-verifier subagent"
4. Supervisor spawns task-verifier agent
5. Verifier approves the work, tries to record: `mcp__cas__verification action=add task_id=<task> status=approved`
6. CAS rejects: "Supervisors can only verify epics, not individual tasks"
7. Supervisor tries to close again — still blocked by verification jail
8. Loop repeats indefinitely

## Impact

- Supervisor resorts to `mcp__cas__task action=delete` to escape the loop, losing task history
- Or leaves tasks open forever with findings in notes but never formally closed
- Wastes 2-3 turns and a subagent spawn per deadlocked task
- In today's session: cas-2f2e, cas-5712, cas-9e06, cas-58a1 all hit this

## Proposed Fix

If the supervisor is the task assignee (or there is no assignee), allow the supervisor's task-verifier subagent to record verification. The role restriction makes sense when workers exist, but when the supervisor is doing the work directly, there's no worker to delegate to.

Alternatively: allow `mcp__cas__task action=close force=true` for supervisors to skip verification on trivial tasks (comments, investigations, spikes).
