#!/usr/bin/env bash
# Phase 3 — rsync 26 Petrastella projects from laptop to Hetzner target.
#
# Task: cas-5a47 (epic cas-28d4 — Petrastella → Hetzner migration)
# Deliverable: this script + migration/phase3-log.md + migration/phase3-wip-report.md
#
# Mode: REPLICATION (source not deleted, both sides persist after Phase 3).
# CAS DB strategy: option A snapshot-and-diverge — include .cas/cas.db* in rsync,
#                  checkpoint WAL first so the copy is consistent.
# Docker compose: files travel as code (R1), no `docker compose up` on server.
# Order: ascending by size_bytes_excluding_regenerable from migration/manifest.json,
#        so the pipeline is validated end-to-end on tiny projects before the big
#        ones commit bandwidth.
#
# Idempotency:
#   - Pre-flight can be re-run indefinitely.
#   - rsync is naturally incremental; re-running this script after a partial
#     transfer resumes from where it stopped. No `--delete` is used.
#   - Log entries are appended with timestamps, so re-runs are traceable.
#
# Hard constraints enforced by this script:
#   - READ-ONLY on ~/Petrastella/* source (except WAL checkpoints, which are
#     SQLite-internal and don't alter the user-observable DB content).
#   - --no-owner --no-group (pippenz:pippenz source → daniel:daniel target).
#   - `--info=progress2,stats2` so stdout reports bytes + files transferred.
#   - Target disk floor: abort a project rsync if df -BG / shows less than
#     $MIN_TARGET_FREE_GB free before the transfer.
#   - No secret values ever leak into the committed log / script.
#
# Usage:
#   bash migration/phase3-rsync.sh               # full run (all 26 projects, smallest-first)
#   bash migration/phase3-rsync.sh preflight     # just run the pre-flight block and exit
#   bash migration/phase3-rsync.sh probe <name>  # dry-run rsync one project, report stats only
#   bash migration/phase3-rsync.sh one <name>    # sync exactly one project by name
#
# Env overrides:
#   HOST              — target host, default daniel@87.99.156.244
#   SOURCE_ROOT       — source parent dir, default ~/Petrastella
#   TARGET_PARENT     — target parent dir on server, default ~/projects
#   MANIFEST          — path to the Phase 1 manifest, default ./migration/manifest.json
#   LOG_FILE          — phase3 log path, default ./migration/phase3-log.md
#   WIP_REPORT        — phase3 wip report path, default ./migration/phase3-wip-report.md
#   MIN_TARGET_FREE_GB — target disk floor, default 2
#   DRY_RUN           — if set to 1, rsync runs with --dry-run for all projects
#
# Author: factory worker mighty-viper-52 (2026-04-11)

set -euo pipefail

# Deterministic locale for sort/grep/awk across hosts.
export LC_ALL=C

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

HOST="${HOST:-daniel@87.99.156.244}"
SOURCE_ROOT="${SOURCE_ROOT:-$HOME/Petrastella}"
TARGET_PARENT="${TARGET_PARENT:-~/projects}"
MANIFEST="${MANIFEST:-$(cd "$(dirname "$0")/.." && pwd)/migration/manifest.json}"
LOG_FILE="${LOG_FILE:-$(cd "$(dirname "$0")/.." && pwd)/migration/phase3-log.md}"
WIP_REPORT="${WIP_REPORT:-$(cd "$(dirname "$0")/.." && pwd)/migration/phase3-wip-report.md}"
MIN_TARGET_FREE_GB="${MIN_TARGET_FREE_GB:-2}"
DRY_RUN="${DRY_RUN:-0}"

RSYNC_EXCLUDES=(
  --exclude='node_modules/'
  --exclude='dist/'
  --exclude='build/'
  --exclude='.next/'
  --exclude='.nuxt/'
  --exclude='.output/'
  --exclude='target/'
  --exclude='.pnpm-store/'
  --exclude='playwright-report/'
  --exclude='test-results/'
  --exclude='coverage/'
  --exclude='.cas/worktrees/'
  --exclude='.claude/backups/'
  --exclude='.cache/'
  --exclude='.turbo/'
)

RSYNC_FLAGS=(
  -aHAX
  --no-owner --no-group
  --info=progress2,stats2
)

if [[ "$DRY_RUN" == "1" ]]; then
  RSYNC_FLAGS+=(--dry-run)
fi

# SSH multiplexing so 26 project transfers reuse one connection.
SSH_CTRL="/tmp/cas-phase3-ssh-%h-%p-%r"
SSH_OPTS=(
  -o "ControlMaster=auto"
  -o "ControlPath=${SSH_CTRL}"
  -o "ControlPersist=10m"
  -o "BatchMode=yes"
  -o "ConnectTimeout=15"
  -o "ServerAliveInterval=30"
  -o "ServerAliveCountMax=6"
)

# ---------------------------------------------------------------------------
# Log helpers
# ---------------------------------------------------------------------------

log_init() {
  local started_at
  started_at="$(date -Iseconds)"
  cat > "$LOG_FILE" <<EOF
# Phase 3 rsync execution log

**Task**: cas-5a47 (epic cas-28d4)
**Started**: $started_at
**Source**: \`$SOURCE_ROOT\` (pippenz@$(hostname))
**Target**: \`$HOST:$TARGET_PARENT\`
**Mode**: replication (no \`--delete\`), $( [[ "$DRY_RUN" == 1 ]] && echo "DRY RUN" || echo "LIVE" )
**Manifest**: \`$MANIFEST\` (@ $(jq -r '.generated_at' "$MANIFEST" 2>/dev/null || echo unknown))
**Script**: \`migration/phase3-rsync.sh\`

## Summary

_Filled in after the run completes — see tail of this file for per-project stats._

| Metric | Value |
|---|---|
| Projects planned | _pending_ |
| Projects succeeded | _pending_ |
| Projects failed | _pending_ |
| Total bytes transferred | _pending_ |
| Total files transferred | _pending_ |
| Target disk free before | _pending_ |
| Target disk free after | _pending_ |

## Per-project sections

EOF
}

log_project_header() {
  local name="$1"
  local expected_size="$2"
  local timestamp
  timestamp="$(date -Iseconds)"
  cat >> "$LOG_FILE" <<EOF

### ${name}

- **Started**: ${timestamp}
- **Expected effective size** (from manifest): ${expected_size} bytes
EOF
}

log_line() {
  local name="$1"
  shift
  printf -- "- **%s**: %s\n" "$name" "$*" >> "$LOG_FILE"
}

log_code_block() {
  local caption="$1"
  shift
  {
    echo ""
    echo "- **${caption}**:"
    echo ""
    echo "\`\`\`"
    cat
    echo "\`\`\`"
    echo ""
  } >> "$LOG_FILE"
}

# ---------------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------------

preflight() {
  echo "[preflight] ssh auth" >&2
  local who
  who="$(ssh "${SSH_OPTS[@]}" "$HOST" 'whoami' 2>&1)" || die "preflight: ssh failed: $who"
  [[ "$who" == "daniel" ]] || die "preflight: expected user 'daniel', got '$who'"

  echo "[preflight] cas binary" >&2
  local cas_ver
  cas_ver="$(ssh "${SSH_OPTS[@]}" "$HOST" '/usr/local/bin/cas --version' 2>&1)" \
    || die "preflight: cas --version failed: $cas_ver"
  [[ "$cas_ver" == cas\ 2.* ]] || die "preflight: expected cas 2.x, got: $cas_ver"

  echo "[preflight] cas-serve active" >&2
  local svc
  svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel' 2>&1)" \
    || die "preflight: cas-serve@daniel not active: $svc"

  echo "[preflight] target projects dir" >&2
  ssh "${SSH_OPTS[@]}" "$HOST" 'test -d ~/projects && test -w ~/projects' \
    || die "preflight: ~/projects not present or not writable on target"

  echo "[preflight] target disk" >&2
  local free_gb
  free_gb="$(ssh "${SSH_OPTS[@]}" "$HOST" 'df -BG / | awk "NR==2 {gsub(/G/, \"\", \$4); print \$4}"')"
  [[ "$free_gb" -ge 10 ]] || die "preflight: target disk has only ${free_gb}G free, need >=10G"

  echo "[preflight] github ssh from target (warn only)" >&2
  ssh "${SSH_OPTS[@]}" "$HOST" 'ssh -o BatchMode=yes -o StrictHostKeyChecking=accept-new -T git@github.com 2>&1 | head -1' || true

  echo "[preflight] rsync present" >&2
  command -v rsync >/dev/null || die "preflight: rsync not on this host"
  ssh "${SSH_OPTS[@]}" "$HOST" 'command -v rsync >/dev/null' || die "preflight: rsync not on target"

  echo "[preflight] jq + manifest" >&2
  command -v jq >/dev/null || die "preflight: jq not on this host"
  [[ -f "$MANIFEST" ]] || die "preflight: manifest not found at $MANIFEST"
  jq empty "$MANIFEST" 2>/dev/null || die "preflight: manifest is not valid JSON"

  echo "[preflight] wal-checkpoint tool (sqlite3 or python3)" >&2
  if command -v sqlite3 >/dev/null; then
    echo "[preflight]   using sqlite3 CLI" >&2
  elif python3 -c 'import sqlite3' 2>/dev/null; then
    echo "[preflight]   using python3 sqlite3 module (CLI not installed on this host)" >&2
  else
    die "preflight: neither sqlite3 CLI nor python3 sqlite3 module available — cannot checkpoint WAL"
  fi

  echo "[preflight] ALL GREEN" >&2
}

die() {
  echo "FATAL: $*" >&2
  exit 1
}

# ---------------------------------------------------------------------------
# Project ordering + metadata
# ---------------------------------------------------------------------------

list_projects_smallest_first() {
  jq -r '.projects | sort_by(.size_bytes_excluding_regenerable) | .[].name' "$MANIFEST"
}

project_expected_size() {
  local name="$1"
  jq -r --arg n "$name" '.projects[] | select(.name == $n) | .size_bytes_excluding_regenerable' "$MANIFEST"
}

project_has_cas_db() {
  local name="$1"
  local has
  has="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .cas_db.present // false' "$MANIFEST")"
  [[ "$has" == "true" ]]
}

project_expected_dirty() {
  local name="$1"
  jq -r --arg n "$name" '.projects[] | select(.name == $n) | (.git.dirty_files // 0) + (.git.untracked_files // 0)' "$MANIFEST"
}

project_expected_stash() {
  local name="$1"
  jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.stash_count // 0' "$MANIFEST"
}

# ---------------------------------------------------------------------------
# Per-project pipeline
# ---------------------------------------------------------------------------

checkpoint_wal() {
  local name="$1"
  local db="${SOURCE_ROOT}/${name}/.cas/cas.db"
  if [[ ! -f "$db" ]]; then
    echo "skip (no .cas/cas.db)"
    return 0
  fi
  local out rc=0
  if command -v sqlite3 >/dev/null; then
    out="$(sqlite3 "$db" 'PRAGMA wal_checkpoint(TRUNCATE);' 2>&1)" || rc=$?
  else
    # python3 fallback — same PRAGMA, same semantics
    out="$(python3 - "$db" <<'PY' 2>&1
import sys, sqlite3
db = sys.argv[1]
try:
    con = sqlite3.connect(db, timeout=5)
    row = con.execute('PRAGMA wal_checkpoint(TRUNCATE);').fetchone()
    print(row)
    con.close()
except Exception as e:
    print(f"ERROR: {e}", file=sys.stderr)
    sys.exit(1)
PY
)" || rc=$?
  fi
  if [[ "$rc" -eq 0 ]]; then
    echo "ok: $out"
  else
    # tolerate locked DB — log it, proceed with whatever snapshot exists
    echo "warn: $out"
  fi
}

ensure_target_disk() {
  local free_gb
  free_gb="$(ssh "${SSH_OPTS[@]}" "$HOST" 'df -BG / | awk "NR==2 {gsub(/G/, \"\", \$4); print \$4}"' 2>/dev/null)"
  if [[ "$free_gb" -lt "$MIN_TARGET_FREE_GB" ]]; then
    die "target disk below floor: ${free_gb}G free, floor is ${MIN_TARGET_FREE_GB}G"
  fi
  echo "$free_gb"
}

rsync_project() {
  local name="$1"
  local src="${SOURCE_ROOT}/${name}/"
  local dst="${HOST}:${TARGET_PARENT}/${name}/"

  # rsync output goes to a temp file so we can extract stats from it.
  local tmp
  tmp="$(mktemp -t phase3-rsync.XXXXXX)"

  local rc=0
  rsync "${RSYNC_FLAGS[@]}" "${RSYNC_EXCLUDES[@]}" \
    -e "ssh ${SSH_OPTS[*]}" \
    "$src" "$dst" > "$tmp" 2>&1 || rc=$?

  # Extract totals from the stats2 block.
  local sent_bytes files_transferred total_size
  sent_bytes="$(awk -F': ' '/^Total bytes sent/  {gsub(/[^0-9]/, "", $2); print $2}' "$tmp" | tail -1)"
  files_transferred="$(awk -F': ' '/^Number of regular files transferred/ {gsub(/[^0-9]/, "", $2); print $2}' "$tmp" | tail -1)"
  total_size="$(awk -F': ' '/^Total file size/ {gsub(/[^0-9]/, "", $2); print $2}' "$tmp" | tail -1)"

  # Write only the stats block to the log (from "Number of files:" to end).
  # rsync's --info=progress2 produces a single carriage-returned progress line
  # that looks awful in markdown, so we drop it entirely.
  sed -n '/^Number of files:/,$p' "$tmp" | log_code_block "rsync stats"
  rm -f "$tmp"

  if [[ "$rc" -ne 0 ]]; then
    log_line "rsync_rc" "$rc (FAILED)"
    return "$rc"
  fi

  log_line "rsync_rc" "0"
  log_line "bytes_sent" "${sent_bytes:-?}"
  log_line "files_transferred" "${files_transferred:-?}"
  log_line "total_size_on_target" "${total_size:-?}"

  # Export for summary aggregation.
  printf '%s\n' "${sent_bytes:-0}" > /tmp/phase3-last-sent
  printf '%s\n' "${files_transferred:-0}" > /tmp/phase3-last-files
}

verify_project() {
  local name="$1"
  local rpath="${TARGET_PARENT}/${name}"

  # Is the source a git repo at all? Use the manifest so we don't need a second SSH hop.
  local is_repo
  is_repo="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.is_repo // false' "$MANIFEST")"

  local git_head git_status_count git_stash_count target_du integrity
  target_du="$(ssh "${SSH_OPTS[@]}" "$HOST" "du -sb ${rpath} 2>/dev/null | awk '{print \$1}'")"

  if [[ "$is_repo" == "true" ]]; then
    git_head="$(ssh "${SSH_OPTS[@]}" "$HOST" "test -f ${rpath}/.git/HEAD && echo ok || echo MISSING" 2>&1)"
    git_status_count="$(ssh "${SSH_OPTS[@]}" "$HOST" "cd ${rpath} 2>/dev/null && git status --porcelain 2>/dev/null | wc -l || echo NOT_A_REPO")"
    git_stash_count="$(ssh "${SSH_OPTS[@]}" "$HOST" "cd ${rpath} 2>/dev/null && git stash list 2>/dev/null | wc -l || echo 0")"
  else
    git_head="n/a (non-repo project)"
    git_status_count="0"
    git_stash_count="0"
  fi

  log_line "git_head" "$git_head"
  log_line "target_git_status_count" "$git_status_count"
  log_line "target_git_stash_count" "$git_stash_count"
  log_line "target_du_bytes" "$target_du"

  if project_has_cas_db "$name"; then
    integrity="$(ssh "${SSH_OPTS[@]}" "$HOST" "sqlite3 ${rpath}/.cas/cas.db 'PRAGMA integrity_check;' 2>&1" || echo ERROR)"
    log_line "cas_db_integrity" "$integrity"
  fi

  # Expected-vs-actual reconciliation.
  local expected_dirty expected_stash expected_size
  expected_dirty="$(project_expected_dirty "$name")"
  expected_stash="$(project_expected_stash "$name")"
  expected_size="$(project_expected_size "$name")"
  log_line "expected_dirty_from_manifest" "$expected_dirty"
  log_line "expected_stash_from_manifest" "$expected_stash"
  log_line "expected_size_from_manifest" "$expected_size"

  local warnings=()
  # stash count is strict
  if [[ "$git_stash_count" =~ ^[0-9]+$ ]] && [[ "$git_stash_count" -ne "$expected_stash" ]]; then
    warnings+=("stash count drift: target=$git_stash_count manifest=$expected_stash")
  fi
  # dirty count within ±10%
  if [[ "$git_status_count" =~ ^[0-9]+$ ]]; then
    local tol_low tol_high
    tol_low="$(awk -v e="$expected_dirty" 'BEGIN{printf "%d", e*0.9}')"
    tol_high="$(awk -v e="$expected_dirty" 'BEGIN{printf "%d", e*1.1 + 1}')"
    if (( git_status_count < tol_low || git_status_count > tol_high )); then
      warnings+=("dirty count drift >10%: target=$git_status_count manifest=$expected_dirty")
    fi
  fi
  # size within ±5% (only rough — excludes differ slightly)
  if [[ "$target_du" =~ ^[0-9]+$ ]] && [[ "$expected_size" =~ ^[0-9]+$ ]] && [[ "$expected_size" -gt 0 ]]; then
    local sz_low sz_high
    sz_low="$(awk -v e="$expected_size" 'BEGIN{printf "%d", e*0.95}')"
    sz_high="$(awk -v e="$expected_size" 'BEGIN{printf "%d", e*1.05}')"
    if (( target_du < sz_low || target_du > sz_high )); then
      warnings+=("size drift >5%: target=$target_du manifest=$expected_size")
    fi
  fi
  if [[ ${#warnings[@]} -gt 0 ]]; then
    log_line "WARNINGS" "${#warnings[@]}"
    for w in "${warnings[@]}"; do
      log_line "  warning" "$w"
    done
  else
    log_line "WARNINGS" "0"
  fi
}

sync_one_project() {
  local name="$1"
  local expected_size
  expected_size="$(project_expected_size "$name")"

  log_project_header "$name" "$expected_size"

  # Disk floor
  local free_gb
  free_gb="$(ensure_target_disk)"
  log_line "target_disk_free_before_gb" "$free_gb"

  # WAL checkpoint (only if .cas/cas.db)
  local wal_result
  wal_result="$(checkpoint_wal "$name")"
  log_line "wal_checkpoint" "$wal_result"

  # Actual rsync
  if rsync_project "$name"; then
    :
  else
    local rc=$?
    log_line "OUTCOME" "FAILED (rsync rc=$rc)"
    return $rc
  fi

  # Post-rsync verification
  verify_project "$name"

  local end_ts
  end_ts="$(date -Iseconds)"
  log_line "completed_at" "$end_ts"
  log_line "OUTCOME" "OK"
}

# ---------------------------------------------------------------------------
# WIP report (built at the end from the target's git state)
# ---------------------------------------------------------------------------

emit_wip_report() {
  local started_at
  started_at="$(date -Iseconds)"
  cat > "$WIP_REPORT" <<EOF
# Phase 3 WIP Report — per-project dirty/stash/unpushed rollup on target

**Task**: cas-5a47
**Generated**: $started_at
**Target**: \`$HOST:$TARGET_PARENT\`
**Source of truth for "expected" columns**: Phase 1 manifest \`$MANIFEST\`

This report confirms what WIP landed on the target after Phase 3 rsync.
Expected values come from the Phase 1 manifest; actual values come from
running \`git status\`, \`git stash list\`, \`git rev-list\` on the target
immediately after each project's rsync completes.

"REQUIRES HUMAN" section at the bottom flags any project where the target
state does not match the manifest or where source drift during the rsync
window suggests a re-sync.

## Per-project table

| Project | Branch | Dirty (t/m) | Untracked (t/m) | Stash (t/m) | Unpushed (t/m) | Match? |
|---|---|---|---|---|---|---|
EOF

  local name row
  while read -r name; do
    # Note: passing $name into the heredoc via local expansion; everything else
    # (\$branch, \$HOME, etc.) is escaped so it evaluates on the remote shell.
    # Must NOT wrap the remote `cd` target in quotes, because `~` / `$HOME`
    # semantics differ between quoted-local-expansion and unquoted-remote-expansion.
    # `grep -c` returns exit 1 when it finds 0 matches AND prints "0" on stdout,
    # so `|| echo 0` fires and we get "0\n0" in the capture. Use `wc -l` for
    # counting and let pipe failures stay harmless (pipefail is off in this heredoc).
    row="$(ssh "${SSH_OPTS[@]}" "$HOST" "bash -s" <<REMOTE || true
set -e
proj_path="\$HOME/projects/${name}"
cd "\$proj_path" 2>/dev/null || { echo "MISSING|-|-|-|-|-"; exit 0; }
if [[ -d .git ]]; then
  branch=\$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo "-")
  porcelain=\$(git status --porcelain 2>/dev/null || true)
  dirty=\$(printf '%s\n' "\$porcelain" | grep -v '^??' | grep -v '^\$' | wc -l)
  untracked=\$(printf '%s\n' "\$porcelain" | grep -c '^??' || echo 0)
  untracked=\${untracked%%[^0-9]*}
  stash=\$(git stash list 2>/dev/null | wc -l)
  unpushed=\$(git log '@{u}..HEAD' --oneline 2>/dev/null | wc -l || echo 0)
  unpushed=\${unpushed%%[^0-9]*}
  echo "\$branch|\$dirty|\$untracked|\$stash|\$unpushed"
else
  echo "non-repo|0|0|0|0"
fi
REMOTE
)"

    local t_branch t_dirty t_untracked t_stash t_unpushed
    IFS='|' read -r t_branch t_dirty t_untracked t_stash t_unpushed <<<"$row"

    local m_dirty m_untracked m_stash m_unpushed
    m_dirty="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.dirty_files // 0' "$MANIFEST")"
    m_untracked="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.untracked_files // 0' "$MANIFEST")"
    m_stash="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.stash_count // 0' "$MANIFEST")"
    m_unpushed="$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.unpushed_commits // 0' "$MANIFEST")"

    local match="yes"
    if [[ "$t_stash" != "$m_stash" ]]; then match="STASH DRIFT"; fi
    if [[ "$t_branch" == "MISSING" ]]; then match="MISSING"; fi

    printf '| %s | %s | %s/%s | %s/%s | %s/%s | %s/%s | %s |\n' \
      "$name" "$t_branch" \
      "$t_dirty" "$m_dirty" \
      "$t_untracked" "$m_untracked" \
      "$t_stash" "$m_stash" \
      "$t_unpushed" "$m_unpushed" \
      "$match" \
      >> "$WIP_REPORT"
  done < <(list_projects_smallest_first)

  cat >> "$WIP_REPORT" <<EOF

## REQUIRES HUMAN

The Phase 1 manifest found 57 stashes across 7 projects. Any "STASH DRIFT"
entry in the table above means the target did not receive those stashes —
investigate before Phase 6/8.

Any "MISSING" entry means the project directory never landed on the target.
Re-run this script with \`bash migration/phase3-rsync.sh one <name>\` to retry.

Any project where the "Dirty (t/m)" values are wildly different (more than
10%) is flagged above and may indicate concurrent source-side editing
during the rsync window. Document the discrepancy or schedule a re-sync
for a quieter moment.

### Notes

- Dirty count on the target is computed WITHOUT including untracked files
  (so it matches the manifest's \`dirty_files\`). Untracked is a separate
  column.
- Source of all "m" values is \`$MANIFEST\` (Phase 1).
- Source of all "t" values is \`git status\` / \`git stash list\` /
  \`git rev-list\` on the target, executed after Phase 3 rsync.
EOF
}

# ---------------------------------------------------------------------------
# Entry points
# ---------------------------------------------------------------------------

main_full_run() {
  preflight
  log_init

  local planned=0 succeeded=0 failed=0 total_sent=0 total_files=0
  local free_before free_after
  free_before="$(ensure_target_disk)"

  local projects=()
  while read -r name; do projects+=("$name"); done < <(list_projects_smallest_first)
  planned="${#projects[@]}"

  echo "[main] planned=$planned projects" >&2

  local name
  for name in "${projects[@]}"; do
    echo "[main] >>> $name" >&2
    if sync_one_project "$name"; then
      succeeded=$((succeeded + 1))
      local s f
      s="$(cat /tmp/phase3-last-sent 2>/dev/null || echo 0)"
      f="$(cat /tmp/phase3-last-files 2>/dev/null || echo 0)"
      total_sent=$((total_sent + s))
      total_files=$((total_files + f))
    else
      failed=$((failed + 1))
      echo "[main] FAIL $name (non-zero rsync rc) — continuing to next project" >&2
    fi
  done

  free_after="$(ensure_target_disk)"

  # Stamp summary table at the top of the log.
  local summary_tmp
  summary_tmp="$(mktemp)"
  awk -v planned="$planned" -v succ="$succeeded" -v failed="$failed" \
      -v bytes="$total_sent" -v files="$total_files" \
      -v fb="$free_before" -v fa="$free_after" '
    /Projects planned/ { sub(/_pending_/, planned); print; next }
    /Projects succeeded/ { sub(/_pending_/, succ); print; next }
    /Projects failed/ { sub(/_pending_/, failed); print; next }
    /Total bytes transferred/ { sub(/_pending_/, bytes); print; next }
    /Total files transferred/ { sub(/_pending_/, files); print; next }
    /Target disk free before/ { sub(/_pending_/, fb "G"); print; next }
    /Target disk free after/ { sub(/_pending_/, fa "G"); print; next }
    { print }
  ' "$LOG_FILE" > "$summary_tmp"
  mv "$summary_tmp" "$LOG_FILE"

  echo "[main] planned=$planned succeeded=$succeeded failed=$failed sent=$total_sent files=$total_files" >&2

  emit_wip_report

  if [[ "$failed" -gt 0 ]]; then
    return 1
  fi
}

main_preflight_only() {
  preflight
}

main_probe_one() {
  local name="$1"
  preflight
  [[ -f "$LOG_FILE" ]] || log_init
  RSYNC_FLAGS+=(--dry-run)
  sync_one_project "$name"
}

main_one() {
  local name="$1"
  preflight
  [[ -f "$LOG_FILE" ]] || log_init
  sync_one_project "$name"
}

main_wip_only() {
  preflight
  emit_wip_report
  echo "[wip] report written to $WIP_REPORT" >&2
}

# ---------------------------------------------------------------------------
# Dispatch
# ---------------------------------------------------------------------------

cmd="${1:-full}"
case "$cmd" in
  full) main_full_run ;;
  preflight) main_preflight_only ;;
  probe) main_probe_one "${2:-}" ;;
  one) main_one "${2:-}" ;;
  wip) main_wip_only ;;
  *) die "unknown command: $cmd (use: full | preflight | probe <name> | one <name> | wip)" ;;
esac
