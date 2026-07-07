---
date: 2026-07-06
topic: factory-epic-focus-pinning
---

# Factory Epic/Task Focus Pinning

## Problem Frame

The FACTORY and TASKS panels decide what to display by state inference: `cas-cli/src/ui/factory/director/factory_radar.rs` picks the first epic with `InProgress` status, **or falls back to the first epic in the list**. Any stale in-progress epic in the DB — or simply list ordering — makes the panel show an epic unrelated to what this factory session is doing. The operator sees "EPIC: cas-2eb3" while the session works on something else entirely, which destroys trust in the panel. Display focus should be declared, not guessed.

## Requirements

- R1. Default focus is session-scoped: the panels show the epic tied to **this** factory session (factory sessions are already session-scoped entities), not a global state-based guess.
- R2. The supervisor can explicitly pin the displayed epic/task focus for the session via an MCP action, overriding the default.
- R3. If the session has no associated epic and nothing is pinned, the panels show an explicit empty/unfocused state — never an unrelated epic. The first-in-list fallback is removed.
- R4. The TASKS panel scopes to the focused epic's tasks (plus their assignees/status), consistent with the FACTORY section.
- R5. Pinned focus survives TUI restarts/reattach within the same factory session.

## Success Criteria

- Starting a fresh factory session never displays an epic from a previous or concurrent session.
- The supervisor can switch the displayed epic with one MCP call and the panels follow immediately.
- An operator can trust that whatever the FACTORY panel names is what this session is actually working on.

## Scope Boundaries

- No changes to task *state* semantics (what InProgress means, close gates, etc.) — this is display focus only.
- No per-worker focus pinning; focus is one epic per factory session.
- Visual redesign of the panels is the separate TUI-overhaul epic (`docs/brainstorms/2026-07-06-factory-tui-overhaul-requirements.md`).

## Key Decisions

- **Session-scoped default + supervisor pin** (over supervisor-set-only and over scoped state inference): automatic correct behavior in the common case, explicit control when the supervisor needs it, and a hard guarantee against unrelated epics via removal of the fallback.

## Dependencies / Assumptions

- Factory session isolation (m203/m204, session-scoped agents/spawns/messages) provides the session identity to hang focus on — verified to exist.
- Assumption: an association between a factory session and its epic either exists or is derivable at spawn/assignment time; if absent, planning must add the linkage. [Verify during planning]

## Outstanding Questions

### Resolve Before Planning

(none)

### Deferred to Planning

- [Affects R2][Technical] Which MCP surface carries the pin action (`coordination` vs `task` vs a factory-specific action) and where the pin is persisted.
- [Affects R1][Technical] How the session→epic default association is established (supervisor's assigned epic at spawn vs first epic the supervisor starts).

## Next Steps

→ Hand off to planning (cas-supervisor or /plan)
