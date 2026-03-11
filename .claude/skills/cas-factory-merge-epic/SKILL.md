---
name: cas-factory-merge-epic
description: Merge all worker branches after EPIC completion. Use when all tasks are done.
argument-hint: [--dry-run]
---

# factory-merge-epic

# Merge EPIC

Merges all worker branches back to the main branch after EPIC completion.

## Usage

```
/factory-merge-epic [--dry-run]
```

Use --dry-run to preview without actually merging.

## Prerequisites
- All EPIC tasks must be completed
- All workers must have committed their changes
- No merge conflicts (or resolve them first)

## Workflow

1. **Verify EPIC completion**
   - Check all subtasks are closed
   - Verify no blocked or in-progress tasks remain

2. **Gather worker branches**
   - List all worker clones
   - Get current branch for each worker
   - Show uncommitted changes (if any)

3. **Preview changes (dry-run)**
   - Show diff summary for each worker branch
   - List files changed per worker
   - Identify potential conflicts

4. **Run tests**
   - Merge branches to temp branch
   - Run test suite
   - Report any failures

5. **Merge to main**
   - Merge each worker branch sequentially
   - Resolve conflicts if needed
   - Push to remote

6. **Cleanup**
   - Close EPIC task
   - Archive worker branches (optional)
   - Shutdown workers (optional)

## Example

```
# Preview merge
/factory-merge-epic --dry-run

# Execute merge
/factory-merge-epic
```

## Output Format

```
## Merging EPIC: cas-91ff

### Worker Branches
- swift-fox: task/cas-1234 (+142 -23, 5 files)
- calm-owl: task/cas-5678 (+89 -12, 3 files)
- bold-eagle: task/cas-9abc (+56 -8, 2 files)

### Conflict Check
✓ No conflicts detected

### Test Results
✓ All tests passing

### Merge Status
✓ swift-fox merged
✓ calm-owl merged  
✓ bold-eagle merged

### EPIC Closed
cas-91ff marked as completed
```

## Instructions

/factory-merge-epic

## Tags

factory, supervisor, merge
