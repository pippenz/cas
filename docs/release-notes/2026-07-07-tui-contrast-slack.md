# Slack draft — 2026-07-07 TUI contrast pass (main 6a24862)

Channel: #cas-internal (C0B44GUKDK2). Two top-level posts per rubric.

## User post

**Live on production — User**
The factory screen is readable again, everywhere. Was: the mode badge in the corner was light-on-light mush, the status line's detail text faded into the background, and the keyboard shortcut hints were dim enough to miss. Now: every label, badge, and hint on screen meets accessibility contrast standards in both the dark and light themes — and an automated check keeps it that way, so a future color tweak can't quietly make anything unreadable again.

## Dev post

**Live on production — Dev**
Systematic WCAG contrast pass over the TUI theme. Was: `text_primary` (near-white in dark mode) doubled as chip-badge foreground — 1.69:1 on the mode chips — and the status colors were dual-role tokens whose light-mode text uses measured as low as 1.54:1 on the keybind hints. Now: a dedicated near-black `chip_fg` token across all 13 chip/badge sites, targeted lightness fixes for the muted status-bar segment, and a `hint_*` foreground family (dark mode aliases unchanged; light mode darkened same-hue variants solved per color to ≥4.5:1). A 6-test `contrast_guard` module computes relative-luminance ratios over the declared token pairings in both themes and was hand-falsified against the pre-fix palette — palette regressions now fail CI instead of shipping.
