# Phase 3 WIP Report — per-project dirty/stash/unpushed rollup on target

**Task**: cas-5a47
**Generated**: 2026-04-11T10:07:35-04:00
**Target**: `daniel@87.99.156.244:~/projects`
**Source of truth for "expected" columns**: Phase 1 manifest `/home/pippenz/cas-src/.cas/worktrees/mighty-viper-52/migration/manifest.json`

This report confirms what WIP landed on the target after Phase 3 rsync.
Expected values come from the Phase 1 manifest; actual values come from
running `git status`, `git stash list`, `git rev-list` on the target
immediately after each project's rsync completes.

"REQUIRES HUMAN" section at the bottom flags any project where the target
state does not match the manifest or where source drift during the rsync
window suggests a re-sync.

## Per-project table

| Project | Branch | Dirty (t/m) | Untracked (t/m) | Stash (t/m) | Unpushed (t/m) | Match? |
|---|---|---|---|---|---|---|
| tooling | non-repo | 0/0 | 0/0 | 0/0 | 0/0 | yes |
| logging | non-repo | 0/0 | 0/0 | 0/0 | 0/0 | yes |
| verified-path | non-repo | 0/0 | 0/0 | 0/0 | 0/0 | yes |
| country-liberty | main | 46/46 | 2/2 | 0/0 | 0/0 | yes |
| memory-lane | non-repo | 0/0 | 0/0 | 0/0 | 0/0 | yes |
| git-mcp-server | main | 161/161 | 1/1 | 0/0 | 0/0 | yes |
| rocketship-template-new-instance | main | 18/18 | 48/47 | 0/0 | 0/0 | yes |
| full-package-media | develop | 0/0 | 2/2 | 0/0 | 0/0 | yes |
| closure-club | develop | 8/8 | 6/6 | 0/0 | 0/0 | yes |
| petra-stella-cloud | main | 8/8 | 10/9 | 0/0 | 1/1 | yes |
| fixyrs | develop | 310/310 | 2/2 | 0/0 | 0/0 | yes |
| homeschool-whisper | develop | 1/1 | 4/5 | 0/0 | 0/0 | yes |
| pulse-card | develop | 4/4 | 13/13 | 0/0 | 0/0 | yes |
| pantheon | develop | 0/0 | 12/12 | 0/0 | 0/0 | yes |
| edws | main | 201/201 | 3/3 | 0/0 | 4/4 | yes |
| memory-lane-cloud | main | 0/0 | 2/2 | 0/0 | 0/0 | yes |
| pixel-hive | feat/gemini-presigned-url | 3/3 | 3/3 | 3/3 | 0/0 | yes |
| prospect_path | main | 726/726 | 3/3 | 4/4 | 0/0 | yes |
| spaceship_template | main | 74/74 | 7/7 | 8/8 | 0/0 | yes |
| fixy-quasar | main | 664/664 | 2/2 | 0/0 | 0/0 | yes |
| abundant-mines | fix/billing-cancel-stamp-all-recurring-items | 16/16 | 10/9 | 12/12 | 0/0 | yes |
| ozer | main | 5/5 | 20/19 | 9/9 | 0/0 | yes |
| tracetix | scraper-optimizations | 233/233 | 530/530 | 0/0 | 0/0 | yes |
| domdms | cas-3c5e-dom2-toggle-ui | 9/9 | 26/25 | 5/5 | 0/0 | yes |
| gabber-studio | fix/invite-dto-import | 0/0 | 7/7 | 16/16 | 0/0 | yes |
| petrastella-aws | main | 143/143 | 247/248 | 0/0 | 0/0 | yes |

## REQUIRES HUMAN

The Phase 1 manifest found 57 stashes across 7 projects. Any "STASH DRIFT"
entry in the table above means the target did not receive those stashes —
investigate before Phase 6/8.

Any "MISSING" entry means the project directory never landed on the target.
Re-run this script with `bash migration/phase3-rsync.sh one <name>` to retry.

Any project where the "Dirty (t/m)" values are wildly different (more than
10%) is flagged above and may indicate concurrent source-side editing
during the rsync window. Document the discrepancy or schedule a re-sync
for a quieter moment.

### Notes

- Dirty count on the target is computed WITHOUT including untracked files
  (so it matches the manifest's `dirty_files`). Untracked is a separate
  column.
- Source of all "m" values is `/home/pippenz/cas-src/.cas/worktrees/mighty-viper-52/migration/manifest.json` (Phase 1).
- Source of all "t" values is `git status` / `git stash list` /
  `git rev-list` on the target, executed after Phase 3 rsync.
