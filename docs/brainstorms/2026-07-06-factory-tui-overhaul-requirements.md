---
date: 2026-07-06
topic: factory-tui-overhaul
---

# Factory TUI Visual & Information Overhaul

## Problem Frame

The factory TUI (`cas factory --attach`) multiplexes agent panes through a custom terminal-emulation stack, and that indirection loses capabilities a plain terminal has for free — most painfully clickable links. It also under-uses the screen: no git context anywhere, fixed-height sidecar sections that sit empty while the activity feed truncates, and no at-a-glance answer to "who is working on what". This epic is a combined interaction + information + polish upgrade for the operator watching a factory session.

## Requirements

**Clickable links**

- R1. Hyperlinks rendered inside panes must be clickable in the host terminal via OSC 8 passthrough: hyperlink metadata already tracked by the VT layer (`crates/ghostty_vt/src/lib.rs` `hyperlink_at`) is re-emitted to the outer terminal so Konsole (and any OSC 8-capable terminal) handles ctrl+click natively.
- R2. No mouse capture may be introduced for links — native drag-select/copy behavior must be preserved exactly as today.
- R3. Plain-text URLs need no in-TUI treatment; the host terminal's own URL detection covers them once pane text reaches the outer terminal unmangled. Verify this holds through the render path.

**Branch visibility**

- R4. Status bar shows the git branch of the focused pane's working directory (supervisor cwd branch, or the focused worker's worktree branch).
- R5. Each worker pane's border title shows that worker's worktree branch alongside the worker name.
- R6. The sidecar FACTORY section shows the epic integration branch (`factory/<name>`) with ahead/behind counts vs `main`.

**Sidecar information density**

- R7. Adaptive sidecar layout: sections with no content (e.g., CHANGES at 0 items) collapse to a single header line; reclaimed space flows to content-heavy sections (e.g., ACTIVITY).
- R8. Per-worker current task chips in the FACTORY section: each agent row shows its in-progress task id + truncated title inline, not just an idle/active state dot.
- R9. Activity feed upgrades: entries wrap or expand instead of hard one-line truncation with `…`; event types are color-coded; relative timestamps stay live.

**Identity header**

- R10. A top identity line shows: factory session name, focused epic, epic branch, elapsed session time, and worker count — the single-glance "where am I" summary.

## Success Criteria

- Ctrl+clicking a link printed by any agent pane opens it in the browser, and drag-select copy still works without entering any special mode.
- The operator can answer "what branch am I on / is each worker on" without running `git branch` anywhere.
- With one worker active and nothing else happening, the sidecar shows no large empty boxes — activity history fills the freed space.
- "Who is doing what" is answerable from the FACTORY section alone, without opening the tasks view or dashboard.

## Scope Boundaries

- No in-TUI mouse click handling and no keyboard link-picker mode (deliberately rejected in favor of OSC 8 passthrough; can be a follow-on if non-OSC-8 terminals ever matter).
- No general theming/config system for colors — polish within the existing theme palette.
- Which epic/tasks the panels display is **not** this epic — that is the separate epic-focus-pinning epic (`docs/brainstorms/2026-07-06-factory-epic-focus-pinning-requirements.md`).
- Mission Control dashboard redesign beyond what R7-R9 imply is out of scope.

## Key Decisions

- **OSC 8 passthrough over in-TUI click handling**: mouse capture is deliberately disabled in the TUI to keep native drag-select; passthrough gives clickable links with zero capture cost, and Konsole (the primary terminal) supports it natively.
- **All three branch surfaces** (status bar, pane titles, FACTORY section) — they answer different questions (where am I / where is each worker / where does work land) and are individually cheap.
- **Adaptive layout over more sections**: the sidecar's problem is dead space, not missing panels.

## Dependencies / Assumptions

- ghostty_vt exposes hyperlink data per cell (`hyperlink_at`, verified present); assumption: the OSC 8 open/close sequences can be reconstructed per-run at render time — verify in planning.
- ratatui has no first-class OSC 8 span support; passthrough likely needs a post-render pass or backend-level injection. [Needs research during planning]
- Konsole is the primary host terminal; AlternateScrolling quirk already documented separately.

## Outstanding Questions

### Resolve Before Planning

(none)

### Deferred to Planning

- [Affects R1][Needs research] Where in the render path to inject OSC 8 sequences (ratatui backend wrapper vs post-frame diff writer), and how to avoid corrupting the diffing.
- [Affects R3][Technical] Confirm pane text reaches the host terminal byte-clean enough for Konsole's plain-URL detection, or whether line-wrapping in ratatui breaks URL continuity.
- [Affects R4-R6][Technical] Branch lookups must be cheap/cached (no git subprocess per frame).
- [Affects R9][Technical] Activity entry expansion interaction (auto-wrap vs Enter-to-expand) — pick during planning based on vertical budget.

## Next Steps

→ Hand off to planning (cas-supervisor or /plan)
