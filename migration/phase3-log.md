# Phase 3 rsync execution log

**Task**: cas-5a47 (epic cas-28d4)
**Started**: 2026-04-11T10:06:49-04:00
**Source**: `/home/pippenz/Petrastella` (pippenz@Soundwave)
**Target**: `daniel@87.99.156.244:~/projects`
**Mode**: replication (no `--delete`), LIVE
**Manifest**: `/home/pippenz/cas-src/.cas/worktrees/mighty-viper-52/migration/manifest.json` (@ 2026-04-11T12:32:19Z)
**Script**: `migration/phase3-rsync.sh`

## Summary

_Filled in after the run completes — see tail of this file for per-project stats._

| Metric | Value |
|---|---|
| Projects planned | 26 |
| Projects succeeded | 26 |
| Projects failed | 0 |
| Total bytes transferred | 7022948 |
| Total files transferred | 19 |
| Target disk free before | 120G |
| Target disk free after | 120G |

## Per-project sections


### tooling

- **Started**: 2026-04-11T10:06:49-04:00
- **Expected effective size** (from manifest): 45111 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 24 (reg: 19, dir: 5)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 45,111 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 818
Total bytes received: 16

sent 818 bytes  received 16 bytes  556.00 bytes/sec
total size is 45,111  speedup is 54.09
```

- **rsync_rc**: 0
- **bytes_sent**: 818
- **files_transferred**: 0
- **total_size_on_target**: 45111
- **git_head**: n/a (non-repo project)
- **target_git_status_count**: 0
- **target_git_stash_count**: 0
- **target_du_bytes**: 45111
- **expected_dirty_from_manifest**: 0
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 45111
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:50-04:00
- **OUTCOME**: OK

### logging

- **Started**: 2026-04-11T10:06:50-04:00
- **Expected effective size** (from manifest): 103260 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 63 (reg: 46, dir: 17)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 103,260 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 1,992
Total bytes received: 28

sent 1,992 bytes  received 28 bytes  4,040.00 bytes/sec
total size is 103,260  speedup is 51.12
```

- **rsync_rc**: 0
- **bytes_sent**: 1992
- **files_transferred**: 0
- **total_size_on_target**: 103260
- **git_head**: n/a (non-repo project)
- **target_git_status_count**: 0
- **target_git_stash_count**: 0
- **target_du_bytes**: 103260
- **expected_dirty_from_manifest**: 0
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 103260
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:50-04:00
- **OUTCOME**: OK

### verified-path

- **Started**: 2026-04-11T10:06:50-04:00
- **Expected effective size** (from manifest): 124906 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 17 (reg: 11, dir: 6)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 124,906 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 600
Total bytes received: 17

sent 600 bytes  received 17 bytes  1,234.00 bytes/sec
total size is 124,906  speedup is 202.44
```

- **rsync_rc**: 0
- **bytes_sent**: 600
- **files_transferred**: 0
- **total_size_on_target**: 124906
- **git_head**: n/a (non-repo project)
- **target_git_status_count**: 0
- **target_git_stash_count**: 0
- **target_du_bytes**: 124906
- **expected_dirty_from_manifest**: 0
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 124906
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:50-04:00
- **OUTCOME**: OK

### country-liberty

- **Started**: 2026-04-11T10:06:50-04:00
- **Expected effective size** (from manifest): 1110646 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 119 (reg: 83, dir: 36)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 1,110,646 bytes
Total transferred file size: 4,849 bytes
Literal data: 2,049 bytes
Matched data: 2,800 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 5,798
Total bytes received: 115

sent 5,798 bytes  received 115 bytes  11,826.00 bytes/sec
total size is 1,110,646  speedup is 187.83
```

- **rsync_rc**: 0
- **bytes_sent**: 5798
- **files_transferred**: 1
- **total_size_on_target**: 1110646
- **git_head**: ok
- **target_git_status_count**: 48
- **target_git_stash_count**: 0
- **target_du_bytes**: 1110646
- **expected_dirty_from_manifest**: 48
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 1110646
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:51-04:00
- **OUTCOME**: OK

### memory-lane

- **Started**: 2026-04-11T10:06:51-04:00
- **Expected effective size** (from manifest): 1783151 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 605 (reg: 522, dir: 83)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 1,783,151 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.014 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 15,987
Total bytes received: 102

sent 15,987 bytes  received 102 bytes  32,178.00 bytes/sec
total size is 1,783,151  speedup is 110.83
```

- **rsync_rc**: 0
- **bytes_sent**: 15987
- **files_transferred**: 0
- **total_size_on_target**: 1783151
- **git_head**: n/a (non-repo project)
- **target_git_status_count**: 0
- **target_git_stash_count**: 0
- **target_du_bytes**: 1783151
- **expected_dirty_from_manifest**: 0
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 1783151
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:51-04:00
- **OUTCOME**: OK

### git-mcp-server

- **Started**: 2026-04-11T10:06:51-04:00
- **Expected effective size** (from manifest): 2128558 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 269 (reg: 190, dir: 79)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 2,128,558 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 7,802
Total bytes received: 97

sent 7,802 bytes  received 97 bytes  15,798.00 bytes/sec
total size is 2,128,558  speedup is 269.47
```

- **rsync_rc**: 0
- **bytes_sent**: 7802
- **files_transferred**: 0
- **total_size_on_target**: 2128558
- **git_head**: ok
- **target_git_status_count**: 162
- **target_git_stash_count**: 0
- **target_du_bytes**: 2128558
- **expected_dirty_from_manifest**: 162
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 2128558
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:52-04:00
- **OUTCOME**: OK

### rocketship-template-new-instance

- **Started**: 2026-04-11T10:06:52-04:00
- **Expected effective size** (from manifest): 4759545 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 548 (reg: 366, dir: 182)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 4,759,545 bytes
Total transferred file size: 5,578 bytes
Literal data: 4,878 bytes
Matched data: 700 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 27,660
Total bytes received: 267

sent 27,660 bytes  received 267 bytes  55,854.00 bytes/sec
total size is 4,759,545  speedup is 170.43
```

- **rsync_rc**: 0
- **bytes_sent**: 27660
- **files_transferred**: 1
- **total_size_on_target**: 4759545
- **git_head**: ok
- **target_git_status_count**: 66
- **target_git_stash_count**: 0
- **target_du_bytes**: 4759545
- **expected_dirty_from_manifest**: 65
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 4759545
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:52-04:00
- **OUTCOME**: OK

### full-package-media

- **Started**: 2026-04-11T10:06:52-04:00
- **Expected effective size** (from manifest): 14940147 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 490 (reg: 324, dir: 166)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 14,940,147 bytes
Total transferred file size: 25,487 bytes
Literal data: 22,687 bytes
Matched data: 2,800 bytes
File list size: 0
File list generation time: 0.014 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 40,421
Total bytes received: 425

sent 40,421 bytes  received 425 bytes  81,692.00 bytes/sec
total size is 14,940,147  speedup is 365.77
```

- **rsync_rc**: 0
- **bytes_sent**: 40421
- **files_transferred**: 1
- **total_size_on_target**: 14940147
- **git_head**: ok
- **target_git_status_count**: 2
- **target_git_stash_count**: 0
- **target_du_bytes**: 14940147
- **expected_dirty_from_manifest**: 2
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 14940147
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:53-04:00
- **OUTCOME**: OK

### closure-club

- **Started**: 2026-04-11T10:06:53-04:00
- **Expected effective size** (from manifest): 21600774 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 2,883 (reg: 2,345, dir: 536, special: 2)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 21,568,006 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 127,883
Total bytes received: 594

sent 127,883 bytes  received 594 bytes  256,954.00 bytes/sec
total size is 21,568,006  speedup is 167.87
```

- **rsync_rc**: 0
- **bytes_sent**: 127883
- **files_transferred**: 0
- **total_size_on_target**: 21568006
- **git_head**: ok
- **target_git_status_count**: 14
- **target_git_stash_count**: 0
- **target_du_bytes**: 21568006
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 14
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 21600774
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:53-04:00
- **OUTCOME**: OK

### petra-stella-cloud

- **Started**: 2026-04-11T10:06:53-04:00
- **Expected effective size** (from manifest): 23777624 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 2,224 (reg: 1,824, dir: 399, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 23,744,856 bytes
Total transferred file size: 18,006 bytes
Literal data: 15,906 bytes
Matched data: 2,100 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 119,818
Total bytes received: 625

sent 119,818 bytes  received 625 bytes  80,295.33 bytes/sec
total size is 23,744,856  speedup is 197.15
```

- **rsync_rc**: 0
- **bytes_sent**: 119818
- **files_transferred**: 1
- **total_size_on_target**: 23744856
- **git_head**: ok
- **target_git_status_count**: 18
- **target_git_stash_count**: 0
- **target_du_bytes**: 23744856
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 17
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 23777624
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:54-04:00
- **OUTCOME**: OK

### fixyrs

- **Started**: 2026-04-11T10:06:54-04:00
- **Expected effective size** (from manifest): 24683329 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 1,867 (reg: 1,449, dir: 418)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 24,683,329 bytes
Total transferred file size: 89,899 bytes
Literal data: 66,099 bytes
Matched data: 23,800 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 138,987
Total bytes received: 1,249

sent 138,987 bytes  received 1,249 bytes  280,472.00 bytes/sec
total size is 24,683,329  speedup is 176.01
```

- **rsync_rc**: 0
- **bytes_sent**: 138987
- **files_transferred**: 1
- **total_size_on_target**: 24683329
- **git_head**: ok
- **target_git_status_count**: 312
- **target_git_stash_count**: 0
- **target_du_bytes**: 24683329
- **expected_dirty_from_manifest**: 312
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 24683329
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:55-04:00
- **OUTCOME**: OK

### homeschool-whisper

- **Started**: 2026-04-11T10:06:55-04:00
- **Expected effective size** (from manifest): 39217907 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 1,583 (reg: 1,196, dir: 386, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 39,185,139 bytes
Total transferred file size: 94,584 bytes
Literal data: 89,684 bytes
Matched data: 4,900 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 148,094
Total bytes received: 1,254

sent 148,094 bytes  received 1,254 bytes  298,696.00 bytes/sec
total size is 39,185,139  speedup is 262.37
```

- **rsync_rc**: 0
- **bytes_sent**: 148094
- **files_transferred**: 1
- **total_size_on_target**: 39185139
- **git_head**: ok
- **target_git_status_count**: 5
- **target_git_stash_count**: 0
- **target_du_bytes**: 39185139
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 6
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 39217907
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:55-04:00
- **OUTCOME**: OK

### pulse-card

- **Started**: 2026-04-11T10:06:55-04:00
- **Expected effective size** (from manifest): 39665591 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 3,415 (reg: 2,860, dir: 554, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 39,632,823 bytes
Total transferred file size: 111,962 bytes
Literal data: 107,062 bytes
Matched data: 4,900 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 258,577
Total bytes received: 1,590

sent 258,577 bytes  received 1,590 bytes  520,334.00 bytes/sec
total size is 39,632,823  speedup is 152.34
```

- **rsync_rc**: 0
- **bytes_sent**: 258577
- **files_transferred**: 1
- **total_size_on_target**: 39632823
- **git_head**: ok
- **target_git_status_count**: 17
- **target_git_stash_count**: 0
- **target_du_bytes**: 39632823
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 17
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 39665591
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:56-04:00
- **OUTCOME**: OK

### pantheon

- **Started**: 2026-04-11T10:06:56-04:00
- **Expected effective size** (from manifest): 44193730 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 3,706 (reg: 3,155, dir: 549, special: 2)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 0
Total file size: 44,160,962 bytes
Total transferred file size: 0 bytes
Literal data: 0 bytes
Matched data: 0 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 168,355
Total bytes received: 619

sent 168,355 bytes  received 619 bytes  112,649.33 bytes/sec
total size is 44,160,962  speedup is 261.35
```

- **rsync_rc**: 0
- **bytes_sent**: 168355
- **files_transferred**: 0
- **total_size_on_target**: 44160962
- **git_head**: ok
- **target_git_status_count**: 12
- **target_git_stash_count**: 0
- **target_du_bytes**: 44160962
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 12
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 44193730
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:57-04:00
- **OUTCOME**: OK

### edws

- **Started**: 2026-04-11T10:06:57-04:00
- **Expected effective size** (from manifest): 50859497 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 579 (reg: 405, dir: 174)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 50,859,497 bytes
Total transferred file size: 42,614 bytes
Literal data: 25,814 bytes
Matched data: 16,800 bytes
File list size: 0
File list generation time: 0.014 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 43,879
Total bytes received: 577

sent 43,879 bytes  received 577 bytes  88,912.00 bytes/sec
total size is 50,859,497  speedup is 1,144.04
```

- **rsync_rc**: 0
- **bytes_sent**: 43879
- **files_transferred**: 1
- **total_size_on_target**: 50859497
- **git_head**: ok
- **target_git_status_count**: 204
- **target_git_stash_count**: 0
- **target_du_bytes**: 50859497
- **expected_dirty_from_manifest**: 204
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 50859497
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:58-04:00
- **OUTCOME**: OK

### memory-lane-cloud

- **Started**: 2026-04-11T10:06:58-04:00
- **Expected effective size** (from manifest): 72660552 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 1,408 (reg: 1,013, dir: 395)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 72,660,552 bytes
Total transferred file size: 18,872 bytes
Literal data: 17,472 bytes
Matched data: 1,400 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 77,399
Total bytes received: 610

sent 77,399 bytes  received 610 bytes  156,018.00 bytes/sec
total size is 72,660,552  speedup is 931.44
```

- **rsync_rc**: 0
- **bytes_sent**: 77399
- **files_transferred**: 1
- **total_size_on_target**: 72660552
- **git_head**: ok
- **target_git_status_count**: 2
- **target_git_stash_count**: 0
- **target_du_bytes**: 72660552
- **expected_dirty_from_manifest**: 2
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 72660552
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:58-04:00
- **OUTCOME**: OK

### pixel-hive

- **Started**: 2026-04-11T10:06:58-04:00
- **Expected effective size** (from manifest): 83627898 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 5,579 (reg: 4,989, dir: 590)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 83,627,898 bytes
Total transferred file size: 160,856 bytes
Literal data: 152,456 bytes
Matched data: 8,400 bytes
File list size: 0
File list generation time: 0.015 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 414,720
Total bytes received: 2,075

sent 414,720 bytes  received 2,075 bytes  277,863.33 bytes/sec
total size is 83,627,898  speedup is 200.65
```

- **rsync_rc**: 0
- **bytes_sent**: 414720
- **files_transferred**: 1
- **total_size_on_target**: 83627898
- **git_head**: ok
- **target_git_status_count**: 6
- **target_git_stash_count**: 3
- **target_du_bytes**: 83627898
- **expected_dirty_from_manifest**: 6
- **expected_stash_from_manifest**: 3
- **expected_size_from_manifest**: 83627898
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:06:59-04:00
- **OUTCOME**: OK

### prospect_path

- **Started**: 2026-04-11T10:06:59-04:00
- **Expected effective size** (from manifest): 87112922 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 4,143 (reg: 3,221, dir: 922)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 87,112,922 bytes
Total transferred file size: 141,797 bytes
Literal data: 94,897 bytes
Matched data: 46,900 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 244,962
Total bytes received: 2,213

sent 244,962 bytes  received 2,213 bytes  164,783.33 bytes/sec
total size is 87,112,922  speedup is 352.43
```

- **rsync_rc**: 0
- **bytes_sent**: 244962
- **files_transferred**: 1
- **total_size_on_target**: 87112922
- **git_head**: ok
- **target_git_status_count**: 729
- **target_git_stash_count**: 4
- **target_du_bytes**: 87112922
- **expected_dirty_from_manifest**: 729
- **expected_stash_from_manifest**: 4
- **expected_size_from_manifest**: 87112922
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:00-04:00
- **OUTCOME**: OK

### spaceship_template

- **Started**: 2026-04-11T10:07:00-04:00
- **Expected effective size** (from manifest): 103684184 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 6,873 (reg: 5,939, dir: 933, link: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 103,684,276 bytes
Total transferred file size: 94,049 bytes
Literal data: 84,949 bytes
Matched data: 9,100 bytes
File list size: 0
File list generation time: 0.014 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 385,935
Total bytes received: 1,868

sent 385,935 bytes  received 1,868 bytes  258,535.33 bytes/sec
total size is 103,684,276  speedup is 267.36
```

- **rsync_rc**: 0
- **bytes_sent**: 385935
- **files_transferred**: 1
- **total_size_on_target**: 103684276
- **git_head**: ok
- **target_git_status_count**: 81
- **target_git_stash_count**: 8
- **target_du_bytes**: 103684276
- **expected_dirty_from_manifest**: 81
- **expected_stash_from_manifest**: 8
- **expected_size_from_manifest**: 103684184
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:01-04:00
- **OUTCOME**: OK

### fixy-quasar

- **Started**: 2026-04-11T10:07:01-04:00
- **Expected effective size** (from manifest): 144576927 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 1,228 (reg: 920, dir: 308)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 144,576,927 bytes
Total transferred file size: 97,518 bytes
Literal data: 26,118 bytes
Matched data: 71,400 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 66,396
Total bytes received: 1,193

sent 66,396 bytes  received 1,193 bytes  45,059.33 bytes/sec
total size is 144,576,927  speedup is 2,139.06
```

- **rsync_rc**: 0
- **bytes_sent**: 66396
- **files_transferred**: 1
- **total_size_on_target**: 144576927
- **git_head**: ok
- **target_git_status_count**: 666
- **target_git_stash_count**: 0
- **target_du_bytes**: 144576927
- **expected_dirty_from_manifest**: 666
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 144576927
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:02-04:00
- **OUTCOME**: OK

### abundant-mines

- **Started**: 2026-04-11T10:07:02-04:00
- **Expected effective size** (from manifest): 167593183 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 11,606 (reg: 10,911, dir: 694, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 167,560,415 bytes
Total transferred file size: 191,433 bytes
Literal data: 182,333 bytes
Matched data: 9,100 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 783,339
Total bytes received: 2,484

sent 783,339 bytes  received 2,484 bytes  523,882.00 bytes/sec
total size is 167,560,415  speedup is 213.23
```

- **rsync_rc**: 0
- **bytes_sent**: 783339
- **files_transferred**: 1
- **total_size_on_target**: 167560415
- **git_head**: ok
- **target_git_status_count**: 26
- **target_git_stash_count**: 12
- **target_du_bytes**: 167560415
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 25
- **expected_stash_from_manifest**: 12
- **expected_size_from_manifest**: 167593183
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:03-04:00
- **OUTCOME**: OK

### ozer

- **Started**: 2026-04-11T10:07:03-04:00
- **Expected effective size** (from manifest): 214169066 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 9,969 (reg: 9,179, dir: 788, link: 1, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 214,136,358 bytes
Total transferred file size: 132,262 bytes
Literal data: 122,462 bytes
Matched data: 9,800 bytes
File list size: 65,510
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 620,567
Total bytes received: 2,040

sent 620,567 bytes  received 2,040 bytes  415,071.33 bytes/sec
total size is 214,136,358  speedup is 343.94
```

- **rsync_rc**: 0
- **bytes_sent**: 620567
- **files_transferred**: 1
- **total_size_on_target**: 214136358
- **git_head**: ok
- **target_git_status_count**: 25
- **target_git_stash_count**: 9
- **target_du_bytes**: 214136358
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 24
- **expected_stash_from_manifest**: 9
- **expected_size_from_manifest**: 214169066
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:05-04:00
- **OUTCOME**: OK

### tracetix

- **Started**: 2026-04-11T10:07:05-04:00
- **Expected effective size** (from manifest): 233338622 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: skip (no .cas/cas.db)

- **rsync stats**:

```
Number of files: 2,882 (reg: 2,468, dir: 414)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 233,338,622 bytes
Total transferred file size: 79,876 bytes
Literal data: 62,376 bytes
Matched data: 17,500 bytes
File list size: 65,440
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 248,032
Total bytes received: 1,173

sent 248,032 bytes  received 1,173 bytes  498,410.00 bytes/sec
total size is 233,338,622  speedup is 936.33
```

- **rsync_rc**: 0
- **bytes_sent**: 248032
- **files_transferred**: 1
- **total_size_on_target**: 233338622
- **git_head**: ok
- **target_git_status_count**: 763
- **target_git_stash_count**: 0
- **target_du_bytes**: 233338622
- **expected_dirty_from_manifest**: 763
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 233338622
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:05-04:00
- **OUTCOME**: OK

### domdms

- **Started**: 2026-04-11T10:07:05-04:00
- **Expected effective size** (from manifest): 263843157 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 7,334 (reg: 6,657, dir: 676, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 263,810,389 bytes
Total transferred file size: 170,522 bytes
Literal data: 159,322 bytes
Matched data: 11,200 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 519,501
Total bytes received: 2,274

sent 519,501 bytes  received 2,274 bytes  347,850.00 bytes/sec
total size is 263,810,389  speedup is 505.60
```

- **rsync_rc**: 0
- **bytes_sent**: 519501
- **files_transferred**: 1
- **total_size_on_target**: 263810389
- **git_head**: ok
- **target_git_status_count**: 35
- **target_git_stash_count**: 5
- **target_du_bytes**: 263810389
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 34
- **expected_stash_from_manifest**: 5
- **expected_size_from_manifest**: 263843157
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:07-04:00
- **OUTCOME**: OK

### gabber-studio

- **Started**: 2026-04-11T10:07:07-04:00
- **Expected effective size** (from manifest): 539143034 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 8,940 (reg: 8,141, dir: 795, link: 3, special: 1)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 539,110,266 bytes
Total transferred file size: 280,885 bytes
Literal data: 269,685 bytes
Matched data: 11,200 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 672,506
Total bytes received: 3,321

sent 672,506 bytes  received 3,321 bytes  1,351,654.00 bytes/sec
total size is 539,110,266  speedup is 797.70
```

- **rsync_rc**: 0
- **bytes_sent**: 672506
- **files_transferred**: 1
- **total_size_on_target**: 539110266
- **git_head**: ok
- **target_git_status_count**: 7
- **target_git_stash_count**: 16
- **target_du_bytes**: 539110266
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 7
- **expected_stash_from_manifest**: 16
- **expected_size_from_manifest**: 539143034
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:08-04:00
- **OUTCOME**: OK

### petrastella-aws

- **Started**: 2026-04-11T10:07:08-04:00
- **Expected effective size** (from manifest): 3603098602 bytes
- **target_disk_free_before_gb**: 120
- **wal_checkpoint**: ok: (0, 0, 0)

- **rsync stats**:

```
Number of files: 38,018 (reg: 33,020, dir: 4,996, special: 2)
Number of created files: 0
Number of deleted files: 0
Number of regular files transferred: 1
Total file size: 3,603,065,834 bytes
Total transferred file size: 550,229 bytes
Literal data: 514,901 bytes
Matched data: 35,328 bytes
File list size: 0
File list generation time: 0.001 seconds
File list transfer time: 0.000 seconds
Total bytes sent: 1,882,920
Total bytes received: 9,820

sent 1,882,920 bytes  received 9,820 bytes  757,096.00 bytes/sec
total size is 3,603,065,834  speedup is 1,903.62
```

- **rsync_rc**: 0
- **bytes_sent**: 1882920
- **files_transferred**: 1
- **total_size_on_target**: 3603065834
- **git_head**: ok
- **target_git_status_count**: 390
- **target_git_stash_count**: 0
- **target_du_bytes**: 3603065834
- **cas_db_integrity**: ok
- **expected_dirty_from_manifest**: 391
- **expected_stash_from_manifest**: 0
- **expected_size_from_manifest**: 3603098602
- **WARNINGS**: 0
- **completed_at**: 2026-04-11T10:07:35-04:00
- **OUTCOME**: OK
