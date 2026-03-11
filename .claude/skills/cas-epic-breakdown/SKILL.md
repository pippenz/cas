---
name: cas-epic-breakdown
description: Break down an epic into subtasks based on its spec. Use after /epic-spec to generate task list.
argument-hint: [epic-id]
---

# epic-breakdown

# epic-breakdown

# Epic Breakdown

Analyzes an epic's specification and generates well-structured subtasks with proper dependencies and acceptance criteria.

## Usage

```
/epic-breakdown [epic-id]
```

## Workflow

### 1. Load Epic and Spec
First, fetch the epic task:
```
mcp__cas__task action=show id={epic-id}
```

Then load the linked spec:
```
mcp__cas__spec action=get_for_task task_id={epic-id}
```

This returns specs linked to the epic. Use the most recent approved spec, or if none approved, the latest draft.

**Backward Compatibility:** If no spec is found, fall back to reading the epic's `design` and `acceptance_criteria` fields (legacy format). Suggest running `cas spec import --from-task {epic-id}` to migrate.

### 2. Analyze Requirements
Parse the spec to identify:
- **Goals** - High-level objectives to achieve
- **In Scope** - Features and deliverables to implement
- **Technical Requirements** - Constraints and integration points
- **Acceptance Criteria** - Verification requirements
- **Users** - Who benefits (helps prioritize)

If using legacy task fields, parse the `design` field for goals/scope and `acceptance_criteria` for criteria.

### 3. Generate Subtasks
For each identified work item, create a subtask:
```
mcp__cas__task action=create title="{task_title}" description="{task_desc}" acceptance_criteria="{task_criteria}" epic={epic-id} priority={priority}
```

### 4. Set Dependencies
Identify task dependencies and create them:
```
mcp__cas__task action=dep_add id={task-id} to_id={depends-on-id} dep_type=blocks
```

## Task Structure Guidelines

**Task Sizing**
- Each task should be completable in 1-2 hours
- Break larger items into multiple tasks
- Group related small items if appropriate

**Priority Assignment**
- P0: Blocking others, critical path
- P1: Core functionality
- P2: Important but not blocking
- P3: Nice-to-have, polish

**Acceptance Criteria**
- Each task gets specific, verifiable criteria
- Derived from spec's `acceptance_criteria` array
- Include relevant tests to run

**Dependencies**
- Infrastructure before features
- Core before extensions
- Tests can run in parallel

## Mapping Spec Fields to Tasks

| Spec Field | Task Derivation |
|------------|-----------------|
| `goals` | High-level milestones or epic phases |
| `in_scope` | Individual feature tasks |
| `technical_requirements` | Infrastructure/setup tasks |
| `acceptance_criteria` | Verification tasks, test tasks |
| `users` | Prioritization (user-facing = higher priority) |

## Example Output

```
Loaded spec spec-a1b2 for epic cas-1234

Created 8 subtasks:

Phase 1: Foundation
- cas-a001 [P0] Set up database schema
- cas-a002 [P0] Create API types (blocked by a001)

Phase 2: Core Features
- cas-a003 [P1] Implement user creation endpoint
- cas-a004 [P1] Implement login endpoint
- cas-a005 [P1] Add JWT token generation

Phase 3: Integration
- cas-a006 [P2] Add middleware validation
- cas-a007 [P2] Integrate with frontend

Phase 4: Testing
- cas-a008 [P1] Write E2E auth tests
```

## Notes
- Review generated tasks before starting work
- Adjust priorities based on team availability
- Add/remove tasks as scope becomes clearer
- If spec is in `draft` status, remind user to get it approved

## Tags

epic, planning, tasks

## Instructions

/epic-breakdown

## Tags

epic, planning, tasks
