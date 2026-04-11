# Phase 7 global rsync execution log

**Task**: cas-c07f (epic cas-28d4)
**Started**: 2026-04-11T10:22:02-04:00
**Source**: `~/.cas/` and selected `~/.claude/` on this laptop
**Target**: `daniel@87.99.156.244`
**Mode**: replication (option A snapshot), no `--delete`
**Script**: `migration/phase7-rsync-global.sh`

Secrets hygiene: the `redact()` helper strips any 32+ hex character run before
any stdout line is captured into this file. Token values never land here.


## Step 1 — rsync ~/.cas/ with cas-serve stop/restart

- **pre_env_md5**: REDACTED
- **pre_serveenv_md5**: REDACTED
- **local_wal_checkpoint**: attempting via python3 sqlite3
- **local_wal_checkpoint_result**: ok: (0, 0, 0)
- **T_stop**: 2026-04-11T10:22:02-04:00
- **post_stop_is_active**: inactive
- **T_rsync_start**: 2026-04-11T10:22:02-04:00
- **T_rsync_end**: 2026-04-11T10:22:03-04:00

- **~/.cas/ rsync stats**:

```
Number of files: 126 (reg: 122, dir: 4)
Number of created files: 124 (reg: 121, dir: 3)
Number of deleted files: 0
Number of regular files transferred: 122
Total file size: 17,528,735 bytes
Total transferred file size: 17,528,735 bytes
Literal data: 17,528,735 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 17,544,442
Total bytes received: 2,363

sent 17,544,442 bytes  received 2,363 bytes  11,697,870.00 bytes/sec
total size is 17,528,735  speedup is 1.00
```

- **rsync_rc**: 0
- **target_integrity_check**: running via python3 sqlite3
- **target_integrity_check_result**: ok
- **T_start**: 2026-04-11T10:22:03-04:00
- **T_active**: 2026-04-11T10:22:05-04:00
- **post_start_is_active**: active
- **downtime_seconds**: 2.51
- **AC12**: PASS (2.51s <= 60s)

## Step 2 — rsync selective ~/.claude/

- **excludes_count**: 13
- **projects_bytes**: 2193571114
- **projects_gb**: 2.04

- **~/.claude/ rsync stats**:

```
Number of files: 6,706 (reg: 5,016, dir: 1,690)
Number of created files: 6,705 (reg: 5,016, dir: 1,689)
Number of deleted files: 0
Number of regular files transferred: 5,016
Total file size: 2,194,635,438 bytes
Total transferred file size: 2,194,635,438 bytes
Literal data: 2,194,635,438 bytes
Matched data: 0 bytes
File list size: 327,593
File list generation time: 0.014 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 2,195,701,923
Total bytes received: 106,609

sent 2,195,701,923 bytes  received 106,609 bytes  42,637,058.87 bytes/sec
total size is 2,194,635,438  speedup is 1.00
```

- **rsync_rc**: 0

## Step 3 — post-rsync verification

- **post_env_md5**: REDACTED
- **post_serveenv_md5**: REDACTED
- **AC9_env**: PASS
- **AC9_serveenv**: PASS
- **laptop_cas_db_bytes**: 17043456
- **target_cas_db_bytes**: 17043456
- **cas_db_size_delta_bytes**: 0
- **AC5**: PASS (delta < 1MB)
- **claude_skills_has_files**: 1
- **claude_agents_has_files**: 1
- **claude_commands_has_files**: 1
- **claude_hooks_has_files**: 1
- **AC7**: PASS
- **laptop_projects_count**: 270
- **target_projects_count**: 270
- **AC8**: PASS (270 in [256,284])
- **AC6_cas_serve_active**: active
- **AC4_post_restart_integrity**: ok
