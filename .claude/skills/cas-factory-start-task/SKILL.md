---
name: cas-factory-start-task
description: Start working on an assigned task. Use when worker receives task assignment.
argument-hint: [task-id]
---

# factory-start-task

# Start Task

Claims and begins work on a CAS task. Shows task details, loads relevant skills, and sets up context.

## Usage

```
/factory-start-task [task-id]
```

If no task-id provided, shows tasks assigned to you.

## Workflow

1. **No task-id provided**
   - Call `mcp__cas__task action=mine` to see your assigned tasks
   - Show task IDs and titles

2. **With task-id**
   - Fetch task: `mcp__cas__task action=show id={task-id}`
   - Verify assignment (should be assigned to you)
   - Claim task: `mcp__cas__task action=claim id={task-id}`
   - Start task: `mcp__cas__task action=start id={task-id}`
   - Display:
     - Task title and description
     - Acceptance criteria
     - Related context/dependencies
     - Design notes (if any)

3. **Load relevant skills**
   - List available skills: `mcp__cas__skill action=list`
   - Match skills to task domain based on labels, title, and description:
     - `frontend`, `ui`, `web`, `react`, `css` → load frontend/UI skills
     - `test`, `e2e`, `testing` → load `/cas-e2e-testing`
     - API routes, data fetching → load API/data skills
     - deployment, CI/CD → load deployment skills
   - Invoke matching skills to load their instructions
   - Record loaded skills in a progress note

4. **Frontend task detection**
   - If task involves UI/frontend work, remind worker about chrome-devtools verification requirement
   - Worker must use `chrome-devtools` MCP to take screenshots and verify visual output before closing

5. **Set up context**
   - Show current git branch
   - Show clone path
   - Remind about committing with task reference

## Example

```
# See assigned tasks
/factory-start-task

# Start specific task
/factory-start-task cas-1234
```

## Output Format

```
## Starting Task: cas-1234
Title: Implement user authentication

### Description
Add JWT-based authentication to the API...

### Acceptance Criteria
- [ ] Login endpoint returns JWT token
- [ ] Protected routes require valid token
- [ ] Token refresh endpoint works

### Loaded Skills
- cas-e2e-testing (task has testing label)

### Context
- Clone: ~/cas-clones/swift-fox
- Branch: task/cas-1234-user-auth

### Getting Started
1. Read the existing auth code in src/auth/
2. Implement the criteria above
3. If this is a frontend task: verify with chrome-devtools before closing
4. Run /factory-task-done when complete
```

Invocation:
/factory-start-task

## Instructions

/factory-start-task

## Tags

factory, worker, tasks
