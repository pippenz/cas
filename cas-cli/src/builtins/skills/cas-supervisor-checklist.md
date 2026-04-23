---
name: cas-supervisor-checklist
description: Quick startup checklist for factory supervisors. Use at the beginning of a factory session to load context, check EPICs, and confirm worker availability.
managed_by: cas
---

# Supervisor Checklist

## Session Start

1. Identify yourself: `mcp__cas__coordination action=whoami`
2. Load EPIC/task context:
   ```
   mcp__cas__task action=list task_type=epic
   mcp__cas__task action=ready
   mcp__cas__task action=list status=blocked
   ```
3. Pull relevant memories and rules:
   ```
   mcp__cas__search action=search query="<keywords>" doc_type=entry limit=5
   ```
4. Check codemap freshness:
   - If `.claude/CODEMAP.md` is missing → run `/codemap` to generate it.
   - If it exists but is stale (structural changes since last update) → run `/codemap` to refresh.
   - Workers reference CODEMAP for codebase orientation — ensure it's current before spawning them.
5. Check worker availability: `mcp__cas__coordination action=worker_status`
6. **Session hygiene triage** — check for leftover WIP from prior factory sessions:
   ```
   mcp__cas__coordination action=gc_report
   ```
   The report's "Prior-factory WIP candidates" section lists uncommitted files
   in the main worktree. Files often survive when a prior session died mid-task
   without committing. Decide salvage / commit / discard **before** spawning
   workers — otherwise a cherry-pick into `develop` will abort later.

   For the full history of what prior sessions left behind, see
   `.cas/logs/factory-session-{YYYY-MM-DD}.log` (written automatically on
   `SessionEnd`; each block records session id, agent, worktree, and a
   `git status --porcelain` snapshot).

## Intake Gate (Before Planning)

- [ ] "What does done look like?" has a measurable answer
- [ ] No vague terms — "better/faster/cleaner" replaced with testable criteria
- [ ] All assumptions stated and confirmed
- [ ] Scope broken into discrete chunks if sprawling
- [ ] No conflicts with existing architecture or prior decisions
- [ ] User override logged if any challenge was overridden

## During Coordination

Record decisions as you go:
```
mcp__cas__memory action=remember title="..." content="..." tags="decision"
```

## Epic Planning Checklist

- Every subtask has a `demo_statement` (if not, it may be a horizontal slice — restructure)
- Investigation tasks use `task_type=spike` with question-based acceptance criteria
- When multiple approaches exist, a spike with a fit check comparison in `design_notes` precedes implementation tasks

## Review Gate (Per Task Completion)

- [ ] Tests exist and pass (including failure paths)
- [ ] No DRY violations or SRP violations
- [ ] No work outside declared layer boundary
- [ ] Output matches declared interface
- [ ] No magic numbers that should be configurable
- [ ] Obvious SOLID violations flagged with specifics

## Before Closing an EPIC

- Verify all worker branches are merged into the epic branch
- Confirm task deliverables exist on the epic branch
- Run full test suite on epic branch

## Session End

Store a short summary memory tagged `summary`.
