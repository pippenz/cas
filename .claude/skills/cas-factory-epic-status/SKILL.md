---
name: cas-factory-epic-status
description: Show current EPIC progress and worker status. Use for supervisor oversight.
---

# factory-epic-status

# EPIC Status Dashboard

Shows comprehensive status of the current EPIC including task progress and worker assignments.

## Usage

```
/factory-epic-status
```

## Information Displayed

1. **EPIC Overview**
   - EPIC ID and title
   - Overall progress (X/Y tasks complete)
   - Time since EPIC started

2. **Task Breakdown**
   - In-progress tasks with assignees
   - Blocked tasks with blockers
   - Ready tasks (unassigned)
   - Recently completed tasks

3. **Worker Status**
   - Each worker's current task
   - Idle workers (no assigned task)
   - Worker activity (time since last action)

4. **Suggested Actions**
   - Unassigned ready tasks to assign
   - Blocked tasks needing attention
   - Idle workers to assign work to

## Implementation

```
# Get EPIC tasks
mcp__cas__task action=show id={epic-id} with_deps=true

# Get worker status
mcp__cas__agent action=list

# Get in-progress tasks
mcp__cas__task action=list status=in_progress

# Get blocked tasks
mcp__cas__task action=blocked

# Get ready tasks
mcp__cas__task action=ready
```

## Output Format

```
## EPIC: {title} ({id})
Progress: 8/12 tasks (67%)

### In Progress (3)
- cas-1234: Task title [swift-fox] (2h)
- cas-5678: Another task [calm-owl] (45m)

### Blocked (1)
- cas-9abc: Blocked task [bold-eagle]
  └─ Blocker: Waiting for API access

### Ready to Assign (2)
- cas-def0: Ready task (P1)
- cas-1111: Another ready (P2)

### Workers
- swift-fox: Working on cas-1234
- calm-owl: Working on cas-5678
- bold-eagle: BLOCKED on cas-9abc
- wise-owl: IDLE (5m)

### Suggested Actions
1. Assign cas-def0 to wise-owl (idle)
2. Unblock cas-9abc (bold-eagle waiting)
```

## Instructions

/factory-epic-status

## Tags

factory, supervisor, status
