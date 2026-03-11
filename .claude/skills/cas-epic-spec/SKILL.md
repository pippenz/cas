---
name: cas-epic-spec
description: Gather detailed specification for an epic through structured questions. Use after creating an epic to define requirements.
argument-hint: [epic-id]
---

# epic-spec

# epic-spec

# Epic Specification

Conducts a structured interview to gather detailed requirements for an epic, then creates a native Spec linked to the epic task.

## Usage

```
/epic-spec [epic-id]
```

## Workflow

### 1. Identify Epic
If no epic-id provided, show in-progress epics to choose from:
```
mcp__cas__task action=list task_type=epic status=in_progress
```

### 2. Gather Requirements
Use AskUserQuestion to conduct structured interview:

**Summary**
- What is this epic about in one paragraph?

**Goals**
- What is the primary goal of this epic?
- What are the key deliverables?

**Scope**
- What is explicitly in scope?
- What is explicitly out of scope?

**Users & Stakeholders**
- Who are the target users?
- What are their main pain points?
- What does success look like for them?

**Technical Requirements**
- What are the technical constraints?
- What systems/APIs need integration?
- What are the performance requirements?

**Acceptance Criteria**
- How will we know when this is done?
- What are the must-have vs nice-to-have features?
- What tests need to pass?

### 3. Create Native Spec
Create a Spec linked to the epic using gathered information:
```
mcp__cas__spec action=create task_id={epic-id} title="{epic_title}" spec_type=epic summary="{summary}" goals="{goals_json}" in_scope="{in_scope_json}" out_of_scope="{out_scope_json}" users="{users_json}" technical_requirements="{tech_req_json}" acceptance_criteria="{criteria_json}" design_notes="{notes}" status=under_review
```

Field mappings:
- `summary` - One paragraph overview
- `goals` - JSON array of goal strings
- `in_scope` - JSON array of in-scope items
- `out_of_scope` - JSON array of out-of-scope items
- `users` - JSON array of user personas/descriptions
- `technical_requirements` - JSON array of technical constraints
- `acceptance_criteria` - JSON array of criteria
- `design_notes` - Free-form text for additional context

### 4. Link and Confirm
The spec is automatically linked via `task_id`. Confirm creation:
```
Spec {spec-id} created and linked to epic {epic-id}.
Status: under_review

The spec will be synced to .cas/specs/{spec-id}.md when approved.
Run `/epic-breakdown {epic-id}` to generate subtasks from this spec.
```

## Spec Fields Reference

| Interview Section | Spec Field |
|------------------|------------|
| Summary paragraph | `summary` |
| Primary goal, deliverables | `goals` (array) |
| In scope items | `in_scope` (array) |
| Out of scope items | `out_of_scope` (array) |
| Target users, pain points | `users` (array) |
| Constraints, integrations, performance | `technical_requirements` (array) |
| Done criteria, must-haves, tests | `acceptance_criteria` (array) |
| Additional context | `design_notes` (text) |

## After Spec
Run `/epic-breakdown {epic-id}` to auto-generate subtasks from the spec.

## Migration from Legacy
If an epic has spec data in task.design/acceptance_criteria fields, migrate it:
```
cas spec import --from-task {epic-id}
```

## Tags

epic, spec, planning

## Instructions

/epic-spec

## Tags

epic, spec, planning
