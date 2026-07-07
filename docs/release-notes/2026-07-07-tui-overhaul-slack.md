# Slack draft — factory TUI overhaul (post after merge to main + push)

Channel: #cas-internal (C0B44GUKDK2) — two distinct top-level posts.

---

## Post 1 — User

The factory screen used to be a wall of text that kept its secrets: links you couldn't click, no idea what branch anything was on, half the sidebar sitting empty while the activity log cut every line short. Now the terminal works like you'd expect — and tells you what's happening at a glance.

- Links printed by any agent are now ctrl+clickable, straight in your terminal — no copy-paste dance. Text selection still works exactly as before.
- You can see git branches everywhere they matter: the bottom bar shows the branch of whatever pane you're focused on, every worker pane's title shows its branch, and the sidebar shows the epic's branch with how far ahead/behind it is.
- A new header line answers "where am I": session name, the epic in focus, its branch, how long the session has been running, and how many workers are active.
- Empty sidebar sections shrink to one line and give their space to the activity feed, which now wraps recent entries instead of chopping them, color-codes event types, and keeps its timestamps live.
- Each worker row shows what task it's actually working on — "who is doing what" is one glance.

## Post 2 — Dev

The attach TUI's custom render stack used to strip terminal hyperlinks, shell out for nothing, and hard-truncate everything; the sidecar allocated fixed heights regardless of content. This release rebuilds the information surface.

- OSC 8 passthrough: the VT layer's per-cell hyperlink metadata is recorded per frame (keyed by final host-terminal coordinates, offset-corrected per pane) and re-emitted by the buffer backend bracketing contiguous linked runs. No mouse capture — native drag-select untouched. URIs are ESC/BEL-sanitized; overlay-covered cells are pruned so dialogs never inherit stale links; a feed-time introducer gate (correct across arbitrarily split chunks) skips the scan for link-free panes; full and compact render pipelines keep separate maps.
- Branch visibility: one background-thread cache (fs-locked snapshot swap, in-flight dedup, TTL) feeds all three surfaces — status bar, pane titles, epic branch with ahead/behind — with zero git subprocesses on the daemon loop or render path, and ahead/behind parse orientation pinned by scratch-repo tests.
- Sidecar density: empty sections collapse to header lines with freed height flowing to content; per-worker task chips resolve via current-task id with assignee fallback (agent id or display name); the identity header reserves a layout row consistently across the render and PTY-resize paths.
- Activity feed: recent-K wrapping, theme-derived category styles with per-agent attribution colors preserved, and render-time relative timestamps (injected-clock tested).
- Verified end-to-end: the integrated branch's full test suite is failure-identical to main's baseline; every feature carries render-level regression tests (TestBackend snapshots for the foreign-epic placeholder, OSC 8 byte streams, adaptive collapse, identity header).
