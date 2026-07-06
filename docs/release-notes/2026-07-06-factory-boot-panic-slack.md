# Slack draft — factory boot panic hotfix (2026-07-06)

Channel: #cas-internal (`C0B44GUKDK2`) — two top-level posts.

---

## Post 1 — User

The factory could get into a state where it simply wouldn't open: splash screen, "loading…", then straight back to your shell prompt with nothing on screen and no error. It only hit some projects — the same install worked fine everywhere else — which made it look like your project was corrupted. It wasn't, and it's fixed.

- The crash was triggered by ordinary punctuation (an em-dash "—") in a pending reminder message; if a reminder happened to contain one at just the wrong position for your terminal width, every boot in that project died.
- Now the factory boots normally regardless of what characters your reminders contain — long messages are shortened cleanly instead of crashing the whole app.
- Nothing to clean up: no database changes, just update the binary.

## Post 2 — Dev

The reminders panel truncated messages with a raw byte slice (`&msg[..n]`), which panics when the cut lands inside a multi-byte UTF-8 character. A pending reminder with an em-dash at the fatal byte offset made the TUI panic during first render — splash, then exit to the prompt, per-project (only stores containing such a reminder), and terminal-width dependent.

- Fix: the panel now routes through the existing char-boundary-safe `ui::widgets::truncate` helper, same as the sibling task/worker panels already did.
- Regression test sweeps the exact offending message across every truncation width, so any future byte-boundary regression in this path fails CI instead of bricking boots.
- No schema or config impact — binary update only.
