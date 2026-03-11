---
name: cas-factory-spawn-workers
description: Request spawning of worker agents. Use after EPIC planning to scale up.
argument-hint: [count]
---

# factory-spawn-workers

# Spawn Workers

Requests the factory TUI to spawn additional worker agents.

## Usage

```
/factory-spawn-workers [count]
```

Default count is 2 if not specified.

## Workflow

1. **Validate EPIC is active**
   - Check that an EPIC is in progress
   - Warn if no EPIC (workers need tasks)

2. **Check current workers**
   - List existing workers: `mcp__cas__agent action=list`
   - Show how many workers are currently active

3. **Request spawn**
   - Call: `mcp__cas__factory action=spawn count={count}`
   - The factory TUI polls for spawn requests and creates workers

4. **Wait for confirmation**
   - Workers appear in agent list after spawning
   - Each worker gets their own git clone

## Example

```
# Spawn 2 workers (default)
/factory-spawn-workers

# Spawn specific count
/factory-spawn-workers 3
```

## Notes
- Maximum 6 workers supported
- Each worker gets isolated clone directory
- Workers register with CAS automatically
- Use after EPIC planning when tasks are ready

## Instructions

/factory-spawn-workers

## Tags

factory, supervisor, workers
