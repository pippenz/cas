---
name: cas-factory-report-blocked
description: Report that current task is blocked. Use when worker encounters a blocker.
argument-hint: [reason]
---

# factory-report-blocked

# Report Blocked

Updates current task status to blocked and notifies the supervisor.

## Usage

```
/factory-report-blocked [reason]
```

Reason is required - describe what's blocking progress.

## Workflow

1. **Get current task**
   - Call `mcp__cas__task action=mine` to find your in-progress task
   - If no task in progress, show error

2. **Update task status**
   - Add blocker note: `mcp__cas__task action=notes id={task-id} notes="{reason}" note_type=blocker`
   - Update status: `mcp__cas__task action=update id={task-id} status=blocked`

3. **Notify supervisor**
   - Send prompt: `mcp__cas__agent action=prompt target=supervisor prompt="Worker {name} is BLOCKED on {task-id}: {reason}"`

## Example

```
/factory-report-blocked Waiting for API credentials from DevOps team
/factory-report-blocked Tests failing due to missing test fixtures
/factory-report-blocked Need clarification on acceptance criteria #3
```

## Common Blockers
- Missing dependencies or credentials
- Unclear requirements
- Waiting on another task
- Test environment issues
- Need code review/approval

## Notes
- Be specific about what's blocking
- Supervisor will see notification immediately
- Task remains assigned to you while blocked
- Use /factory-start-task again when unblocked

## Instructions

/factory-report-blocked

## Tags

factory, worker, status
