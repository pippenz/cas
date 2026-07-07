# Slack draft — epic focus pinning (post after merge to main + push)

Channel: #cas-internal (C0B44GUKDK2) — two distinct top-level posts.

---

## Post 1 — User

The factory dashboard used to guess which epic to show you — and sometimes guessed wrong, proudly displaying an epic from last week or a different project while your real work was invisible. Now it shows the epic *your session* is actually working on, and you can override it explicitly.

- The FACTORY and TASKS panels are tied to your current factory session: fresh sessions never inherit someone else's epic.
- You can pin any epic front-and-center with one command, and clear the pin just as easily — the pin survives detaching and reattaching the TUI.
- When nothing is in focus, the panel says so plainly ("No focused epic") with a hint on how to pin one — no more mystery epics.
- The task list shows only the focused epic's tasks plus your session's own standalone work, so "what is everyone doing" is one glance, not a scavenger hunt.

## Post 2 — Dev

Panel focus used to be inferred from global task state (first in-progress epic, else literally the first epic in the list). Now display focus is declared: session-scoped by default, supervisor-pinnable via MCP, with the guessing fallback deleted.

- Focus resolution precedence: explicit pin > session's own epic (persisted in session metadata, restored on reattach) > conservative inference > explicit empty state. The first-in-list fallback is gone.
- New coordination action `focus_epic` (`id=<epic>` to pin, `clear=true` to unpin) — validates the target exists, is an Epic, and isn't Closed; every pin/clear is recorded as an activity event.
- Session metadata writes are now serialized: both writer processes (TUI daemon and MCP server) go through one fs2-locked, temp-file + atomic-rename update helper, killing a lost-update race between epic transitions and pins.
- The TASKS sidebar renders through a scoped view (focused epic's groups + session-agents' standalone tasks) shared with the selection/count logic, so clicks, counts, and rendering can't drift apart.
- Test surface grew accordingly: precedence, reattach roundtrip, concurrent-writer interleaving, closed/nonexistent/non-epic rejection, render-level regression for the foreign-epic symptom, and backward-compat parsing of pre-existing session metadata.
