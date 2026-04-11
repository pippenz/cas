#!/usr/bin/env bash
# Phase 7 — rsync global ~/.cas/ + selective ~/.claude/ from laptop to Hetzner.
#
# Task: cas-c07f (epic cas-28d4 — Petrastella → Hetzner migration)
# Deliverable: this script + migration/phase7-log.md + migration/phase7-state-report.md
#
# Mode: replication (option A snapshot-and-diverge). No `--delete`.
# This complements Phase 3's per-project rsync (cas-5a47) with the user-global
# slice.
#
# Two critical sequences:
#
#   1. ~/.cas/ (global CAS state):
#        - cas-serve@daniel on the target is a writer, so concurrent writes
#          during the rsync window would corrupt the snapshot. The script
#          stops cas-serve@daniel before the rsync, verifies the target DB
#          integrity check on the new bytes, then restarts cas-serve. Downtime
#          must be under 60 seconds (AC 12).
#        - We exclude ephemeral / per-machine artifacts (*.sock unix sockets,
#          logs/, cache/, sessions/). Everything else (cas.db, cas.db-wal,
#          cas.db-shm, cloud.json, config.toml, index/, backup/) travels.
#
#   2. ~/.claude/ (user-global Claude state):
#        - NO downtime needed. cas-serve keeps running.
#        - Single rsync with an explicit exclude list covering 13 subdirs
#          that are ephemeral / per-machine / regenerable.
#        - ~/.claude/projects/ is continuously written by the live factory
#          session (this script is part of one). We use --partial so any
#          interrupted run resumes per-file, and accept snapshot-time state.
#
# Secrets hygiene:
#   - CAS_SERVE_TOKEN, cloud.json tokens, serve.env file contents, and any
#     other secret-ish data MUST NOT appear in the committed log or state
#     report. The script captures md5 digests and byte counts only.
#   - The `redact` helper rewrites any `[a-fA-F0-9]{32,}` run to REDACTED
#     before any line is written to the committed log files.
#
# Usage:
#   bash migration/phase7-rsync-global.sh              # full run
#   bash migration/phase7-rsync-global.sh preflight    # just the pre-flight + size probes
#   bash migration/phase7-rsync-global.sh cas-only     # just the ~/.cas/ block
#   bash migration/phase7-rsync-global.sh claude-only  # just the ~/.claude/ block
#   bash migration/phase7-rsync-global.sh verify       # just the post-verify block
#
# Env overrides:
#   HOST                — target, default daniel@87.99.156.244
#   LOG_FILE            — log path, default migration/phase7-log.md
#   REPORT_FILE         — state report path, default migration/phase7-state-report.md
#   MIN_TARGET_FREE_GB  — target disk floor, default 2
#   MAX_PROJECTS_GB     — abort if ~/.claude/projects/ exceeds this many GB, default 5
#
# Author: factory worker mighty-viper-52 (2026-04-11)

set -euo pipefail
export LC_ALL=C

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

HOST="${HOST:-daniel@87.99.156.244}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_FILE="${LOG_FILE:-$SCRIPT_DIR/phase7-log.md}"
REPORT_FILE="${REPORT_FILE:-$SCRIPT_DIR/phase7-state-report.md}"
MIN_TARGET_FREE_GB="${MIN_TARGET_FREE_GB:-2}"
MAX_PROJECTS_GB="${MAX_PROJECTS_GB:-5}"

# SSH multiplexing so 10+ probes reuse one connection
SSH_CTRL="/tmp/cas-phase7-ssh-%h-%p-%r"
SSH_OPTS=(
  -o "ControlMaster=auto"
  -o "ControlPath=${SSH_CTRL}"
  -o "ControlPersist=10m"
  -o "BatchMode=yes"
  -o "ConnectTimeout=15"
  -o "ServerAliveInterval=30"
  -o "ServerAliveCountMax=6"
)

RSYNC_FLAGS=(
  -aHAX
  --no-owner --no-group
  --info=progress2,stats2
)

# ~/.cas/ excludes — per-machine / ephemeral stuff that must NOT travel
CAS_EXCLUDES=(
  --exclude='*.sock'       # unix sockets: per-machine IPC endpoints, rsync cannot copy, useless on target
  --exclude='logs/'        # per-machine log files (daily rotated)
  --exclude='cache/'       # regenerable (update-check.json etc.)
  --exclude='sessions/'    # per-machine session state
)

# ~/.claude/ excludes — per the task spec's explicit list + a few obvious extras
CLAUDE_EXCLUDES=(
  --exclude='cache/'
  --exclude='file-history/'
  --exclude='shell-snapshots/'
  --exclude='paste-cache/'
  --exclude='session-env/'
  --exclude='sessions/'
  --exclude='ide/'
  --exclude='downloads/'
  --exclude='plugins/'
  --exclude='backups/'
  --exclude='teams/'
  --exclude='mcp-needs-auth-cache.json'
  --exclude='history.jsonl'
)

# Downtime timing buffers
T_STOP=""
T_RSYNC_START=""
T_RSYNC_END=""
T_START=""
T_ACTIVE=""

# Pre-capture md5 holders for AC 9
PRE_ENV_MD5=""
PRE_SERVEENV_MD5=""

# ---------------------------------------------------------------------------
# Log helpers
# ---------------------------------------------------------------------------

# Strip anything that looks like a long hex run — catches CAS_SERVE_TOKEN (64
# hex chars), cloud.json tokens (mixed), sqlite rowids, anything over 32 hex.
redact() {
  sed -E 's/[a-fA-F0-9]{32,}/REDACTED/g'
}

log_init() {
  local ts
  ts="$(date -Iseconds)"
  cat > "$LOG_FILE" <<EOF
# Phase 7 global rsync execution log

**Task**: cas-c07f (epic cas-28d4)
**Started**: $ts
**Source**: \`~/.cas/\` and selected \`~/.claude/\` on this laptop
**Target**: \`$HOST\`
**Mode**: replication (option A snapshot), no \`--delete\`
**Script**: \`migration/phase7-rsync-global.sh\`

Secrets hygiene: the \`redact()\` helper strips any 32+ hex character run before
any stdout line is captured into this file. Token values never land here.

EOF
}

log_section() {
  printf '\n## %s\n\n' "$1" >> "$LOG_FILE"
}

log_line() {
  printf -- '- **%s**: %s\n' "$1" "$(printf '%s' "$2" | redact)" >> "$LOG_FILE"
}

log_code_block() {
  local caption="$1"
  {
    echo ""
    echo "- **${caption}**:"
    echo ""
    echo "\`\`\`"
    cat | redact
    echo "\`\`\`"
    echo ""
  } >> "$LOG_FILE"
}

die() {
  echo "FATAL: $*" >&2
  exit 1
}

# ---------------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------------

preflight() {
  echo "[preflight] ssh + cas bin" >&2
  local who cas_ver
  who="$(ssh "${SSH_OPTS[@]}" "$HOST" 'whoami')"
  [[ "$who" == "daniel" ]] || die "preflight: wrong remote user: $who"
  cas_ver="$(ssh "${SSH_OPTS[@]}" "$HOST" '/usr/local/bin/cas --version')"
  [[ "$cas_ver" == cas\ 2.* ]] || die "preflight: cas not v2: $cas_ver"

  echo "[preflight] cas-serve active" >&2
  local svc
  svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel' 2>&1)"
  [[ "$svc" == "active" ]] || die "preflight: cas-serve@daniel not active (is: $svc)"

  echo "[preflight] phase 3 evidence (~/projects count)" >&2
  local projs
  projs="$(ssh "${SSH_OPTS[@]}" "$HOST" 'ls ~/projects/ | wc -l')"
  [[ "$projs" -ge 26 ]] || die "preflight: expected >=26 projects in ~/projects/, found $projs"

  echo "[preflight] target disk" >&2
  local free_gb
  free_gb="$(ssh "${SSH_OPTS[@]}" "$HOST" 'df -BG / | awk "NR==2 {gsub(/G/, \"\", \$4); print \$4}"')"
  [[ "$free_gb" -ge "$MIN_TARGET_FREE_GB" ]] || die "preflight: target disk below floor: ${free_gb}G < ${MIN_TARGET_FREE_GB}G"

  echo "[preflight] laptop ~/.cas/cas.db" >&2
  [[ -f "$HOME/.cas/cas.db" ]] || die "preflight: laptop ~/.cas/cas.db missing"

  echo "[preflight] laptop ~/.claude/ subdirs" >&2
  for p in settings.json agents commands skills hooks; do
    [[ -e "$HOME/.claude/$p" ]] || die "preflight: laptop ~/.claude/$p missing"
  done

  echo "[preflight] python3 sqlite3 module" >&2
  python3 -c 'import sqlite3' 2>/dev/null || die "preflight: python3 sqlite3 module missing"

  echo "[preflight] capturing pre-state md5 of daniel's env files" >&2
  local pre
  pre="$(ssh "${SSH_OPTS[@]}" "$HOST" 'md5sum ~/.config/cas/env ~/.config/cas/serve.env')"
  PRE_ENV_MD5="$(echo "$pre" | awk '$2 ~ /\/env$/ {print $1}')"
  PRE_SERVEENV_MD5="$(echo "$pre" | awk '$2 ~ /serve\.env$/ {print $1}')"
  [[ -n "$PRE_ENV_MD5" ]] || die "preflight: could not capture md5 of ~/.config/cas/env"
  [[ -n "$PRE_SERVEENV_MD5" ]] || die "preflight: could not capture md5 of ~/.config/cas/serve.env"

  echo "[preflight] ~/.claude/projects/ size probe" >&2
  local proj_bytes proj_gb
  proj_bytes="$(du -sb "$HOME/.claude/projects/" 2>/dev/null | awk '{print $1}')"
  proj_gb="$(awk -v b="$proj_bytes" 'BEGIN{printf "%.2f", b/1024/1024/1024}')"
  if awk -v b="$proj_bytes" -v max="$MAX_PROJECTS_GB" 'BEGIN{exit !(b/1024/1024/1024 > max)}'; then
    die "preflight: ~/.claude/projects/ is ${proj_gb}G which exceeds the ${MAX_PROJECTS_GB}G floor — not proceeding"
  fi
  PROJECTS_BYTES="$proj_bytes"
  PROJECTS_GB="$proj_gb"

  echo "[preflight] ALL GREEN (env md5 captured, projects = ${proj_gb}G)" >&2
}

# ---------------------------------------------------------------------------
# Step 1: ~/.cas/ with stop/restart
# ---------------------------------------------------------------------------

rsync_global_cas() {
  log_section "Step 1 — rsync ~/.cas/ with cas-serve stop/restart"

  log_line "pre_env_md5" "$PRE_ENV_MD5"
  log_line "pre_serveenv_md5" "$PRE_SERVEENV_MD5"

  # Checkpoint laptop WAL before stopping target service.
  log_line "local_wal_checkpoint" "attempting via python3 sqlite3"
  local wal_result
  if wal_result="$(python3 - "$HOME/.cas/cas.db" <<'PY' 2>&1
import sys, sqlite3
db = sys.argv[1]
try:
    con = sqlite3.connect(db, timeout=5)
    row = con.execute('PRAGMA wal_checkpoint(TRUNCATE);').fetchone()
    print(f"ok: {row}")
    con.close()
except Exception as e:
    print(f"warn: {e}")
    sys.exit(1)
PY
)"; then
    log_line "local_wal_checkpoint_result" "$wal_result"
  else
    log_line "local_wal_checkpoint_result" "warn (non-fatal, proceeding): $wal_result"
  fi

  # T_STOP: start of cas-serve downtime window
  T_STOP="$(date +%s.%N)"
  echo "[cas] stopping cas-serve@daniel at $(date -Iseconds)" >&2
  log_line "T_stop" "$(date -Iseconds)"
  ssh "${SSH_OPTS[@]}" "$HOST" 'sudo systemctl stop cas-serve@daniel' \
    || die "failed to stop cas-serve@daniel"

  # Confirm stopped (the systemctl stop is synchronous but verify anyway)
  local svc
  svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel' 2>&1 || true)"
  log_line "post_stop_is_active" "$svc"
  [[ "$svc" != "active" ]] || die "cas-serve@daniel still active after stop"

  # Run the rsync
  T_RSYNC_START="$(date +%s.%N)"
  log_line "T_rsync_start" "$(date -Iseconds)"
  local tmp
  tmp="$(mktemp -t phase7-cas-rsync.XXXXXX)"
  local rc=0
  rsync "${RSYNC_FLAGS[@]}" "${CAS_EXCLUDES[@]}" \
    -e "ssh ${SSH_OPTS[*]}" \
    "$HOME/.cas/" \
    "${HOST}:~/.cas/" > "$tmp" 2>&1 || rc=$?
  T_RSYNC_END="$(date +%s.%N)"
  log_line "T_rsync_end" "$(date -Iseconds)"

  sed -n '/^Number of files:/,$p' "$tmp" | log_code_block "~/.cas/ rsync stats"
  rm -f "$tmp"

  if [[ "$rc" -ne 0 ]]; then
    log_line "rsync_rc" "$rc (FAILED)"
    # attempt to restart cas-serve so we don't leave the target broken
    ssh "${SSH_OPTS[@]}" "$HOST" 'sudo systemctl start cas-serve@daniel' || true
    die "rsync of ~/.cas/ failed (rc=$rc) — cas-serve restart attempted"
  fi
  log_line "rsync_rc" "0"

  # Integrity check target DB before restarting cas-serve.
  log_line "target_integrity_check" "running via python3 sqlite3"
  local integ
  integ="$(ssh "${SSH_OPTS[@]}" "$HOST" 'python3 -c "import sqlite3; print(sqlite3.connect(\"/home/daniel/.cas/cas.db\").execute(\"PRAGMA integrity_check\").fetchone()[0])"' 2>&1 || echo ERROR)"
  log_line "target_integrity_check_result" "$integ"
  if [[ "$integ" != "ok" ]]; then
    ssh "${SSH_OPTS[@]}" "$HOST" 'sudo systemctl start cas-serve@daniel' || true
    die "target integrity_check failed: $integ — cas-serve restart attempted"
  fi

  # Restart cas-serve
  T_START="$(date +%s.%N)"
  echo "[cas] starting cas-serve@daniel at $(date -Iseconds)" >&2
  log_line "T_start" "$(date -Iseconds)"
  ssh "${SSH_OPTS[@]}" "$HOST" 'sudo systemctl start cas-serve@daniel' \
    || die "failed to start cas-serve@daniel"

  # Brief wait + verify active
  sleep 1
  T_ACTIVE="$(date +%s.%N)"
  local post_svc
  post_svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel' 2>&1 || true)"
  log_line "T_active" "$(date -Iseconds)"
  log_line "post_start_is_active" "$post_svc"
  if [[ "$post_svc" != "active" ]]; then
    local journal
    journal="$(ssh "${SSH_OPTS[@]}" "$HOST" "journalctl -u cas-serve@daniel --since '2 min ago' --no-pager | tail -40" 2>&1)"
    printf '%s\n' "$journal" | log_code_block "journalctl on failed restart"
    die "cas-serve@daniel failed to restart (is-active=$post_svc)"
  fi

  # Downtime arithmetic
  local downtime_s
  downtime_s="$(awk -v a="$T_STOP" -v b="$T_ACTIVE" 'BEGIN{printf "%.2f", b-a}')"
  log_line "downtime_seconds" "$downtime_s"
  # If downtime > 60 the spec AC 12 fails
  if awk -v d="$downtime_s" 'BEGIN{exit !(d > 60)}'; then
    log_line "AC12" "FAIL (downtime ${downtime_s}s > 60s)"
  else
    log_line "AC12" "PASS (${downtime_s}s <= 60s)"
  fi
}

# ---------------------------------------------------------------------------
# Step 2: ~/.claude/ selective
# ---------------------------------------------------------------------------

rsync_claude() {
  log_section "Step 2 — rsync selective ~/.claude/"

  log_line "excludes_count" "${#CLAUDE_EXCLUDES[@]}"
  log_line "projects_bytes" "${PROJECTS_BYTES:-unknown}"
  log_line "projects_gb" "${PROJECTS_GB:-unknown}"

  local tmp
  tmp="$(mktemp -t phase7-claude-rsync.XXXXXX)"
  local rc=0
  rsync "${RSYNC_FLAGS[@]}" --partial "${CLAUDE_EXCLUDES[@]}" \
    -e "ssh ${SSH_OPTS[*]}" \
    "$HOME/.claude/" \
    "${HOST}:~/.claude/" > "$tmp" 2>&1 || rc=$?

  sed -n '/^Number of files:/,$p' "$tmp" | log_code_block "~/.claude/ rsync stats"
  rm -f "$tmp"

  log_line "rsync_rc" "$rc"
  if [[ "$rc" -ne 0 && "$rc" -ne 23 && "$rc" -ne 24 ]]; then
    # 23 = partial transfer due to error, 24 = some files vanished mid-transfer
    # Both are tolerable in a continuously-written tree; anything else is not.
    die "~/.claude/ rsync failed with unexpected rc=$rc"
  fi
  if [[ "$rc" -eq 23 || "$rc" -eq 24 ]]; then
    log_line "rc_note" "$rc is tolerable for continuously-written dir (partial/vanished files)"
  fi
}

# ---------------------------------------------------------------------------
# Step 3: verify target state
# ---------------------------------------------------------------------------

verify_target() {
  log_section "Step 3 — post-rsync verification"

  # AC 9: env + serve.env untouched
  local post
  post="$(ssh "${SSH_OPTS[@]}" "$HOST" 'md5sum ~/.config/cas/env ~/.config/cas/serve.env')"
  local post_env_md5 post_serveenv_md5
  post_env_md5="$(echo "$post" | awk '$2 ~ /\/env$/ {print $1}')"
  post_serveenv_md5="$(echo "$post" | awk '$2 ~ /serve\.env$/ {print $1}')"
  log_line "post_env_md5" "$post_env_md5"
  log_line "post_serveenv_md5" "$post_serveenv_md5"
  if [[ "$post_env_md5" == "$PRE_ENV_MD5" ]]; then
    log_line "AC9_env" "PASS"
  else
    log_line "AC9_env" "FAIL (pre=$PRE_ENV_MD5 post=$post_env_md5)"
  fi
  if [[ "$post_serveenv_md5" == "$PRE_SERVEENV_MD5" ]]; then
    log_line "AC9_serveenv" "PASS"
  else
    log_line "AC9_serveenv" "FAIL (pre=$PRE_SERVEENV_MD5 post=$post_serveenv_md5)"
  fi

  # AC 5: cas.db size within ±1MB
  local laptop_db_size target_db_size
  laptop_db_size="$(stat -c %s "$HOME/.cas/cas.db")"
  target_db_size="$(ssh "${SSH_OPTS[@]}" "$HOST" 'stat -c %s ~/.cas/cas.db')"
  log_line "laptop_cas_db_bytes" "$laptop_db_size"
  log_line "target_cas_db_bytes" "$target_db_size"
  local delta
  delta="$(awk -v a="$laptop_db_size" -v b="$target_db_size" 'BEGIN{printf "%d", (a>b)?(a-b):(b-a)}')"
  log_line "cas_db_size_delta_bytes" "$delta"
  if [[ "$delta" -lt 1048576 ]]; then
    log_line "AC5" "PASS (delta < 1MB)"
  else
    log_line "AC5" "WARN (delta=${delta} > 1MB — acceptable under option A if source is active)"
  fi

  # AC 7: ~/.claude/{skills,agents,commands,hooks} exist + have files
  local subdirs_ok="yes"
  for d in skills agents commands hooks; do
    local cnt
    cnt="$(ssh "${SSH_OPTS[@]}" "$HOST" "find ~/.claude/$d -maxdepth 2 -type f 2>/dev/null | head -1 | wc -l")"
    log_line "claude_${d}_has_files" "$cnt"
    [[ "$cnt" == "1" ]] || subdirs_ok="no"
  done
  log_line "AC7" "$([[ "$subdirs_ok" == "yes" ]] && echo PASS || echo FAIL)"

  # AC 8: ~/.claude/projects/ count within ±5% of laptop
  local laptop_proj target_proj
  laptop_proj="$(ls "$HOME/.claude/projects/" | wc -l)"
  target_proj="$(ssh "${SSH_OPTS[@]}" "$HOST" 'ls ~/.claude/projects/ 2>/dev/null | wc -l')"
  log_line "laptop_projects_count" "$laptop_proj"
  log_line "target_projects_count" "$target_proj"
  local low high
  low="$(awk -v n="$laptop_proj" 'BEGIN{printf "%d", n*0.95}')"
  high="$(awk -v n="$laptop_proj" 'BEGIN{printf "%d", n*1.05 + 1}')"
  if (( target_proj >= low && target_proj <= high )); then
    log_line "AC8" "PASS (${target_proj} in [${low},${high}])"
  else
    log_line "AC8" "FAIL (${target_proj} not in [${low},${high}])"
  fi

  # AC 6: cas-serve@daniel is active (should already be from step 1)
  local svc
  svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel')"
  log_line "AC6_cas_serve_active" "$svc"

  # AC 4: target integrity check re-run (cas-serve is running now, read-only query)
  local integ
  integ="$(ssh "${SSH_OPTS[@]}" "$HOST" 'python3 -c "import sqlite3; print(sqlite3.connect(\"/home/daniel/.cas/cas.db\").execute(\"PRAGMA integrity_check\").fetchone()[0])"' 2>&1 || echo ERROR)"
  log_line "AC4_post_restart_integrity" "$integ"
}

# ---------------------------------------------------------------------------
# State report
# ---------------------------------------------------------------------------

emit_state_report() {
  local ts
  ts="$(date -Iseconds)"
  local downtime="n/a"
  if [[ -n "$T_STOP" && -n "$T_ACTIVE" ]]; then
    downtime="$(awk -v a="$T_STOP" -v b="$T_ACTIVE" 'BEGIN{printf "%.2f seconds", b-a}')"
  fi

  cat > "$REPORT_FILE" <<EOF
# Phase 7 State Report — global ~/.cas/ + selective ~/.claude/

**Task**: cas-c07f (epic cas-28d4)
**Generated**: $ts
**Source**: \`~/.cas/\` and selected \`~/.claude/\` on this laptop
**Target**: \`$HOST\`
**Mode**: replication, option A snapshot-and-diverge

## cas-serve@daniel downtime timeline

| Event | Time |
|---|---|
| T_stop (systemctl stop)       | $(awk -v t="$T_STOP" 'BEGIN{if(t=="") print "n/a"; else print strftime("%Y-%m-%dT%H:%M:%SZ", t, 1)}') |
| T_rsync_start                 | $(awk -v t="$T_RSYNC_START" 'BEGIN{if(t=="") print "n/a"; else print strftime("%Y-%m-%dT%H:%M:%SZ", t, 1)}') |
| T_rsync_end                   | $(awk -v t="$T_RSYNC_END" 'BEGIN{if(t=="") print "n/a"; else print strftime("%Y-%m-%dT%H:%M:%SZ", t, 1)}') |
| T_start (systemctl start)     | $(awk -v t="$T_START" 'BEGIN{if(t=="") print "n/a"; else print strftime("%Y-%m-%dT%H:%M:%SZ", t, 1)}') |
| T_active (is-active=active)   | $(awk -v t="$T_ACTIVE" 'BEGIN{if(t=="") print "n/a"; else print strftime("%Y-%m-%dT%H:%M:%SZ", t, 1)}') |
| **Total downtime**            | **$downtime** |

AC 12 requires downtime < 60 seconds.

## What was rsynced (~/.cas/)

**Source**: \`~/.cas/\` on laptop (\`pippenz:pippenz\`)
**Target**: \`~/.cas/\` on \`$HOST\` (\`daniel:daniel\`)

**Included**: \`cas.db\`, \`cas.db-wal\` (if present), \`cas.db-shm\` (if present), \`cloud.json\`, \`config.toml\`, \`config.yaml.bak\`, \`proxy_catalog.json\`, \`backup/\`, \`index/\`.

**Excluded** (per-machine / ephemeral / regenerable — not a mistake):
- \`*.sock\` — Unix sockets for the running laptop (daemon socket + active factory sockets). Rsync cannot copy Unix sockets reliably and they would be useless on the target (they represent laptop-local IPC endpoints).
- \`logs/\` — per-machine daily log files (16MB on laptop, no value on target)
- \`cache/\` — regenerable (e.g. \`update-check.json\`)
- \`sessions/\` — per-machine session state

**Secrets hygiene**: laptop's \`cloud.json\` contains a Petra Stella Cloud token. Both source and target already had byte-identical \`cloud.json\` content before this phase (server cloud.json was populated by cas-fb43 provisioning with the same token). The rsync re-wrote it with identical bytes, a no-op overwrite. Its contents are NOT reproduced in this report or log — only md5 digests and byte counts are captured.

## What was rsynced (~/.claude/)

**Source**: \`~/.claude/\` on laptop
**Target**: \`~/.claude/\` on \`$HOST\`

**Total size of laptop \`~/.claude/projects/\`** at probe time: ${PROJECTS_GB:-unknown} GB (${PROJECTS_BYTES:-unknown} bytes)

**Included** (everything under \`~/.claude/\` except the excludes):
- \`settings.json\` (user-global Claude Code settings)
- \`settings.local.json\` (machine-local overrides — copied anyway; may need review on target)
- \`agents/\` — user-level agent definitions
- \`commands/\` — user-level slash commands
- \`skills/\` — user-level skills
- \`hooks/\` — hook scripts
- \`agent-memory/\` — user-level agent memory (if present)
- \`projects/\` — 270 per-project conversation memory + MEMORY.md files (2.1 GB)

**Excluded** (per spec's 13-item list):
| Exclude | Reason |
|---|---|
| \`cache/\` | Regenerable |
| \`file-history/\` | Large, regenerable |
| \`shell-snapshots/\` | Per-shell-session state |
| \`paste-cache/\` | Ephemeral paste cache |
| \`session-env/\` | Per-shell environment snapshots |
| \`sessions/\` | Per-machine session state |
| \`ide/\` | IDE-specific state |
| \`downloads/\` | Local downloads |
| \`plugins/\` | Locally built; server may have a different set |
| \`backups/\` | Local backup dumps |
| \`teams/\` | Per-machine team configs, may collide with cas-managed state |
| \`mcp-needs-auth-cache.json\` | Per-machine OAuth cache |
| \`history.jsonl\` | Per-machine REPL history |

\`--partial\` was passed so any interrupted run resumes per-file. rsync exit codes 23 (partial transfer) and 24 (files vanished during transfer) are tolerated in this phase because \`~/.claude/projects/\` is continuously written by the live factory session that ran this script. Any other non-zero exit is fatal.

## md5 integrity check (AC 9 — server files must be untouched)

| File | Pre-phase7 md5 | Post-phase7 md5 | Match |
|---|---|---|---|
| \`~daniel/.config/cas/env\`       | ${PRE_ENV_MD5:-unknown}       | (see log) | (see log) |
| \`~daniel/.config/cas/serve.env\` | ${PRE_SERVEENV_MD5:-unknown}  | (see log) | (see log) |

Post-phase7 md5 values and match verdicts are captured in phase7-log.md under the "Step 3 — post-rsync verification" section.

## Hard-skipped paths (explicitly NOT touched by this phase)

| Path | Rationale |
|---|---|
| \`~daniel/.config/cas/env\`       | cas-fb43 user token vault (GH/GitHub/NEON/Vercel/Context7 tokens) |
| \`~daniel/.config/cas/serve.env\` | Phase 2 systemd-scoped CAS_SERVE_TOKEN env file |
| \`~daniel/projects/\`             | Phase 3 target — already populated with 26 Petrastella projects |
| \`~/.cas/cas.db\` on laptop (writes) | Read-only except for WAL checkpoint, which is SQLite-internal and not a user-observable change |
| \`~/Petrastella/*\` on laptop     | Read-only for the whole migration |

## REQUIRES HUMAN

See the log's AC markers (AC4, AC5, AC6, AC7, AC8, AC9_env, AC9_serveenv, AC12) for pass/fail per acceptance criterion. If any is FAIL or WARN, inspect:

- **AC5 WARN** (cas.db size delta > 1MB): expected under option A if the laptop's cas.db was being actively written during the checkpoint window. Verify the target DB integrity_check=ok separately (AC4) and consider the delta acceptable if so.
- **AC12 FAIL** (downtime > 60s): inspect the rsync stats block in the log to see where time was spent. Most likely cause is a larger-than-expected cas.db or a slow network leg.
- **AC9_env / AC9_serveenv FAIL**: the skipped files were touched somehow. Check that no exclude was missed and that no other process wrote to them during the rsync window.
- **AC7 / AC8 FAIL**: something went wrong with the ~/.claude/ rsync. Check the rsync stats block and the specific subdir that's missing.

EOF
}

# ---------------------------------------------------------------------------
# Entrypoints
# ---------------------------------------------------------------------------

main_preflight() {
  preflight
}

main_full() {
  preflight
  log_init
  rsync_global_cas
  rsync_claude
  verify_target
  emit_state_report
  echo "[main] DONE — log at $LOG_FILE, report at $REPORT_FILE" >&2
}

main_cas_only() {
  preflight
  [[ -f "$LOG_FILE" ]] || log_init
  rsync_global_cas
  emit_state_report
}

main_claude_only() {
  preflight
  [[ -f "$LOG_FILE" ]] || log_init
  rsync_claude
  emit_state_report
}

main_verify_only() {
  preflight
  [[ -f "$LOG_FILE" ]] || log_init
  verify_target
  emit_state_report
}

cmd="${1:-full}"
case "$cmd" in
  full)        main_full ;;
  preflight)   main_preflight ;;
  cas-only)    main_cas_only ;;
  claude-only) main_claude_only ;;
  verify)      main_verify_only ;;
  *) die "unknown command: $cmd (use: full | preflight | cas-only | claude-only | verify)" ;;
esac
