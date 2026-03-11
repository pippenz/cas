---
name: cas-factory-review-worker
description: Review changes made by a specific worker. Use to inspect worker branches.
argument-hint: [worker-name]
---

# factory-review-worker

# Review Worker

Shows the changes made by a specific worker including git diffs and completed tasks.

## Usage

```
/factory-review-worker [worker-name]
```

If no worker specified, lists all workers with summary.

## Information Displayed

1. **Worker Overview**
   - Worker name and status
   - Clone path and current branch
   - Time active in session

2. **Completed Tasks**
   - List of tasks this worker completed
   - Task IDs, titles, and completion time

3. **Current Task**
   - Task currently assigned (if any)
   - Progress notes

4. **Git Changes**
   - Files modified/added/deleted
   - Diff summary (lines added/removed)
   - Detailed diff (optional with --full)

5. **Commits**
   - Recent commits by this worker
   - Commit messages and timestamps

## Example

```
# List all workers
/factory-review-worker

# Review specific worker
/factory-review-worker swift-fox

# Show full diff
/factory-review-worker swift-fox --full
```

## Output Format

```
## Worker: swift-fox

### Status
- Clone: ~/cas-clones/swift-fox
- Branch: task/cas-1234-user-auth
- Active: 2h 15m

### Completed Tasks (2)
- cas-1111: Add login endpoint (45m ago)
- cas-2222: Add token validation (1h 30m ago)

### Current Task
- cas-1234: Implement user authentication
  Status: in_progress (claimed 30m ago)
  Notes: Working on refresh token logic

### Changes
- src/auth/login.rs (+89 -12)
- src/auth/token.rs (+156 -0) [new]
- tests/auth_test.rs (+45 -0) [new]

Total: +290 -12 (3 files)

### Recent Commits
- abc1234 feat(cas-1234): Add token refresh endpoint
- def5678 feat(cas-2222): Implement token validation
- 789abcd feat(cas-1111): Add login endpoint
```

## Instructions

/factory-review-worker

## Tags

factory, supervisor, review
