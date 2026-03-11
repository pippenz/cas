---
name: cas-factory-task-done
description: Mark current task as completed. Use when worker finishes assigned work.
---

# factory-task-done

# Task Done

Marks the current task as completed, commits changes, and notifies the supervisor.

## Usage

```
/factory-task-done
```

## Workflow

1. **Get current task**
   - Call `mcp__cas__task action=mine` to find your in-progress task
   - If no task in progress, show error

2. **Verify completion**
   - Review acceptance criteria from task
   - Ensure all criteria are met
   - Run relevant tests if applicable

3. **Commit changes**
   - Stage changes: `git add -A`
   - Commit with task reference: `git commit -m "feat({task-id}): {summary}"`
   - Include task ID in commit for traceability

4. **Close task**
   - Add completion note: `mcp__cas__task action=notes id={task-id} notes="Implementation complete" note_type=progress`
   - Close task: `mcp__cas__task action=close id={task-id} reason="All acceptance criteria met"`

5. **Notify supervisor**
   - Send prompt: `mcp__cas__agent action=prompt target=supervisor prompt="Worker {name} completed {task-id}: {title}"`

## Checklist Before Completing
- [ ] All acceptance criteria implemented
- [ ] Code compiles/builds without errors
- [ ] Tests pass (if applicable)
- [ ] Changes committed with task reference
- [ ] No debug code or console.logs left

## Notes
- Don't mark done until ALL criteria are met
- If partially complete, add progress notes instead
- Supervisor may assign next task after completion
- Your changes stay in your clone branch until EPIC merge

## Instructions

/factory-task-done

## Tags

factory, worker, tasks
