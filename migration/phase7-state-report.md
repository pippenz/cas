# Phase 7 State Report — global ~/.cas/ + selective ~/.claude/

**Task**: cas-c07f (epic cas-28d4)
**Generated**: 2026-04-11T10:22:56-04:00
**Source**: `~/.cas/` and selected `~/.claude/` on this laptop
**Target**: `daniel@87.99.156.244`
**Mode**: replication, option A snapshot-and-diverge

## cas-serve@daniel downtime timeline

| Event | Time |
|---|---|
| T_stop (systemctl stop)       | 2026-04-11T14:22:02Z |
| T_rsync_start                 | 2026-04-11T14:22:02Z |
| T_rsync_end                   | 2026-04-11T14:22:03Z |
| T_start (systemctl start)     | 2026-04-11T14:22:03Z |
| T_active (is-active=active)   | 2026-04-11T14:22:05Z |
| **Total downtime**            | **2.51 seconds** |

AC 12 requires downtime < 60 seconds.

## What was rsynced (~/.cas/)

**Source**: `~/.cas/` on laptop (`pippenz:pippenz`)
**Target**: `~/.cas/` on `daniel@87.99.156.244` (`daniel:daniel`)

**Included**: `cas.db`, `cas.db-wal` (if present), `cas.db-shm` (if present), `cloud.json`, `config.toml`, `config.yaml.bak`, `proxy_catalog.json`, `backup/`, `index/`.

**Excluded** (per-machine / ephemeral / regenerable — not a mistake):
- `*.sock` — Unix sockets for the running laptop (daemon socket + active factory sockets). Rsync cannot copy Unix sockets reliably and they would be useless on the target (they represent laptop-local IPC endpoints).
- `logs/` — per-machine daily log files (16MB on laptop, no value on target)
- `cache/` — regenerable (e.g. `update-check.json`)
- `sessions/` — per-machine session state

**Secrets hygiene**: laptop's `cloud.json` contains a Petra Stella Cloud token. Both source and target already had byte-identical `cloud.json` content before this phase (server cloud.json was populated by cas-fb43 provisioning with the same token). The rsync re-wrote it with identical bytes, a no-op overwrite. Its contents are NOT reproduced in this report or log — only md5 digests and byte counts are captured.

## What was rsynced (~/.claude/)

**Source**: `~/.claude/` on laptop
**Target**: `~/.claude/` on `daniel@87.99.156.244`

**Total size of laptop `~/.claude/projects/`** at probe time: 2.04 GB (2193571114 bytes)

**Included** (everything under `~/.claude/` except the excludes):
- `settings.json` (user-global Claude Code settings)
- `settings.local.json` (machine-local overrides — copied anyway; may need review on target)
- `agents/` — user-level agent definitions
- `commands/` — user-level slash commands
- `skills/` — user-level skills
- `hooks/` — hook scripts
- `agent-memory/` — user-level agent memory (if present)
- `projects/` — 270 per-project conversation memory + MEMORY.md files (2.1 GB)

**Excluded** (per spec's 13-item list):
| Exclude | Reason |
|---|---|
| `cache/` | Regenerable |
| `file-history/` | Large, regenerable |
| `shell-snapshots/` | Per-shell-session state |
| `paste-cache/` | Ephemeral paste cache |
| `session-env/` | Per-shell environment snapshots |
| `sessions/` | Per-machine session state |
| `ide/` | IDE-specific state |
| `downloads/` | Local downloads |
| `plugins/` | Locally built; server may have a different set |
| `backups/` | Local backup dumps |
| `teams/` | Per-machine team configs, may collide with cas-managed state |
| `mcp-needs-auth-cache.json` | Per-machine OAuth cache |
| `history.jsonl` | Per-machine REPL history |

`--partial` was passed so any interrupted run resumes per-file. rsync exit codes 23 (partial transfer) and 24 (files vanished during transfer) are tolerated in this phase because `~/.claude/projects/` is continuously written by the live factory session that ran this script. Any other non-zero exit is fatal.

## md5 integrity check (AC 9 — server files must be untouched)

| File | Pre-phase7 md5 | Post-phase7 md5 | Match |
|---|---|---|---|
| `~daniel/.config/cas/env`       | d7e98789a424ef5f4839c306bb9f0b21       | (see log) | (see log) |
| `~daniel/.config/cas/serve.env` | 113af35e8b37ca493bf0dfdbb59b07ba  | (see log) | (see log) |

Post-phase7 md5 values and match verdicts are captured in phase7-log.md under the "Step 3 — post-rsync verification" section.

## Hard-skipped paths (explicitly NOT touched by this phase)

| Path | Rationale |
|---|---|
| `~daniel/.config/cas/env`       | cas-fb43 user token vault (GH/GitHub/NEON/Vercel/Context7 tokens) |
| `~daniel/.config/cas/serve.env` | Phase 2 systemd-scoped CAS_SERVE_TOKEN env file |
| `~daniel/projects/`             | Phase 3 target — already populated with 26 Petrastella projects |
| `~/.cas/cas.db` on laptop (writes) | Read-only except for WAL checkpoint, which is SQLite-internal and not a user-observable change |
| `~/Petrastella/*` on laptop     | Read-only for the whole migration |

## REQUIRES HUMAN

See the log's AC markers (AC4, AC5, AC6, AC7, AC8, AC9_env, AC9_serveenv, AC12) for pass/fail per acceptance criterion. If any is FAIL or WARN, inspect:

- **AC5 WARN** (cas.db size delta > 1MB): expected under option A if the laptop's cas.db was being actively written during the checkpoint window. Verify the target DB integrity_check=ok separately (AC4) and consider the delta acceptable if so.
- **AC12 FAIL** (downtime > 60s): inspect the rsync stats block in the log to see where time was spent. Most likely cause is a larger-than-expected cas.db or a slow network leg.
- **AC9_env / AC9_serveenv FAIL**: the skipped files were touched somehow. Check that no exclude was missed and that no other process wrote to them during the rsync window.
- **AC7 / AC8 FAIL**: something went wrong with the ~/.claude/ rsync. Check the rsync stats block and the specific subdir that's missing.

