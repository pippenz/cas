---
name: cas-factory-assign-task
description: Assign a task to a worker agent. Use when supervisor needs to delegate work to workers.
argument-hint: [task-id] [worker-name]
---

# factory-assign-task

# Assign Task to Worker

Assigns a CAS task to a specific worker agent and notifies them.

## Usage

```
/factory-assign-task [task-id] [worker-name]
```

If no arguments provided, lists ready tasks and available workers.

## Workflow

1. **List available options** (no args):
   - Call `mcp__cas__task action=ready` to see actionable tasks
   - Call `mcp__cas__agent action=list` to see available workers
   - Show task IDs with titles and worker names with status

2. **Assign task** (with args):
   - Validate task exists: `mcp__cas__task action=show id={task-id}`
   - Validate worker exists: `mcp__cas__agent action=list` and find worker
   - Update task: `mcp__cas__task action=update id={task-id} assignee={worker-name}`
   - Start task: `mcp__cas__task action=start id={task-id}`
   - Notify worker via prompt queue: `mcp__cas__agent action=prompt target={worker-name} prompt="You've been assigned task {task-id}. Run /factory-start-task {task-id} to begin."`

## Example

```
# List options
/factory-assign-task

# Assign specific task
/factory-assign-task cas-1234 swift-fox
```

## Notes
- Only assign one task per worker at a time
- Check worker's current task before assigning
- Use for P1/P0 tasks that need immediate attention

## Instructions

/factory-assign-task

## Tags

factory, supervisor, tasks
