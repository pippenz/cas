---
name: cas-factory-my-context
description: Show worker's current context. Use to check clone path, branch, and assigned tasks.
---

# factory-my-context

# My Context

Shows the worker's current working context including clone path, git branch, and assigned tasks.

## Usage

```
/factory-my-context
```

## Information Displayed

1. **Identity**
   - Your agent name
   - Your role (worker)
   - Session/Agent ID

2. **Working Directory**
   - Clone path (e.g., ~/cas-clones/swift-fox)
   - Current git branch
   - Uncommitted changes (if any)

3. **Assigned Tasks**
   - Current in-progress task
   - Other tasks assigned to you
   - Task details (ID, title, priority)

4. **EPIC Context**
   - Current EPIC ID and title
   - Your tasks within the EPIC
   - Overall EPIC progress

## Implementation

```
# Get factory context
mcp__cas__factory action=my_context

# Get assigned tasks
mcp__cas__task action=mine

# Get git status
git status --short
git branch --show-current
```

## Output Format

```
## Worker Context: swift-fox

### Working Directory
- Clone: /Users/you/cas-clones/swift-fox
- Branch: task/cas-1234-user-auth
- Status: 3 files modified, 1 untracked

### Current Task
- cas-1234: Implement user authentication (P1)
  Status: in_progress (claimed 2h ago)

### EPIC
- cas-91ff: Factory TUI v2
  Your tasks: 1 in-progress, 2 completed
  Overall: 8/12 tasks done

### Supervisor
- wise-eagle (active)
```

## Instructions

/factory-my-context

## Tags

factory, worker, context
