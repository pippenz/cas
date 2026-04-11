#!/usr/bin/env bash
# Phase 8 — final verification pass + smoke test for the Petrastella → Hetzner migration.
#
# Task: cas-dece (epic cas-28d4)
# Deliverables:
#   - migration/phase8-verification.sh  (this)
#   - migration/phase8-verification-log.md  (per-project 6-check matrix)
#   - migration/phase8-env-audit.md         (env file audit, absorbs Phase 4)
#   - migration/phase8-completion-report.md (THE final report — the user reads this)
#
# This script consolidates what would have been Phase 4 (env audit), Phase 6
# (rebuild smoke test) and Phase 8 (verification) into a single lightweight
# end-of-pipeline pass. Under the locked decisions (replication + option A
# snapshot-and-diverge + R1 no-docker-on-server) each of those original phases
# collapsed to a few checks.
#
# Usage:
#   bash migration/phase8-verification.sh              # full run
#   bash migration/phase8-verification.sh preflight    # just pre-flight
#   bash migration/phase8-verification.sh matrix       # just the per-project matrix
#   bash migration/phase8-verification.sh env-audit    # just the env audit
#   bash migration/phase8-verification.sh smoke        # just the gabber-studio smoke test
#
# Author: factory worker mighty-viper-52 (2026-04-11)

set -euo pipefail
export LC_ALL=C

# ---------------------------------------------------------------------------
# Config
# ---------------------------------------------------------------------------

HOST="${HOST:-daniel@87.99.156.244}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LOG_FILE="${LOG_FILE:-$SCRIPT_DIR/phase8-verification-log.md}"
ENV_AUDIT_FILE="${ENV_AUDIT_FILE:-$SCRIPT_DIR/phase8-env-audit.md}"
MANIFEST="${MANIFEST:-$SCRIPT_DIR/manifest.json}"
SMOKE_PROJECT="${SMOKE_PROJECT:-gabber-studio}"

SSH_CTRL="/tmp/cas-phase8-ssh-%h-%p-%r"
SSH_OPTS=(
  -o "ControlMaster=auto"
  -o "ControlPath=${SSH_CTRL}"
  -o "ControlPersist=10m"
  -o "BatchMode=yes"
  -o "ConnectTimeout=15"
)

die() { echo "FATAL: $*" >&2; exit 1; }

# Strip any 32+ hex char run before writing to committed files.
redact() { sed -E 's/[a-fA-F0-9]{32,}/REDACTED/g'; }

# ---------------------------------------------------------------------------
# Log helpers
# ---------------------------------------------------------------------------

log_init() {
  local ts
  ts="$(date -Iseconds)"
  cat > "$LOG_FILE" <<EOF
# Phase 8 verification log

**Task**: cas-dece (epic cas-28d4)
**Started**: $ts
**Target**: \`$HOST\`
**Source manifest**: \`$MANIFEST\`
**Script**: \`migration/phase8-verification.sh\`

Secrets hygiene: any 32+ hex character run is stripped via \`redact()\` before
being written to this file.

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

# ---------------------------------------------------------------------------
# Pre-flight
# ---------------------------------------------------------------------------

preflight() {
  echo "[preflight] cas + cas-serve" >&2
  local cas_ver
  cas_ver="$(ssh "${SSH_OPTS[@]}" "$HOST" '/usr/local/bin/cas --version')"
  [[ "$cas_ver" == cas\ 2.* ]] || die "preflight: cas not v2: $cas_ver"
  local svc
  svc="$(ssh "${SSH_OPTS[@]}" "$HOST" 'systemctl is-active cas-serve@daniel')"
  [[ "$svc" == "active" ]] || die "preflight: cas-serve@daniel not active: $svc"

  echo "[preflight] laptop ~/Petrastella still intact" >&2
  local lap_count
  lap_count="$(ls "$HOME/Petrastella/" 2>/dev/null | grep -vE '^doc_links_IMPORTANT.md$' | wc -l)"
  [[ "$lap_count" == "26" ]] || die "preflight: laptop ~/Petrastella has $lap_count projects, expected 26"

  echo "[preflight] target ~/projects count" >&2
  local tgt_count
  tgt_count="$(ssh "${SSH_OPTS[@]}" "$HOST" 'ls ~/projects/ | wc -l')"
  # 26 Petrastella + 1 pre-existing cas repo = 27
  [[ "$tgt_count" -ge 26 ]] || die "preflight: target ~/projects has $tgt_count, expected >=26"

  echo "[preflight] manifest present" >&2
  [[ -f "$MANIFEST" ]] || die "preflight: manifest missing: $MANIFEST"

  echo "[preflight] ALL GREEN" >&2
}

# ---------------------------------------------------------------------------
# Step 2: per-project verification matrix (26 × 6 checks)
# ---------------------------------------------------------------------------

list_projects() {
  jq -r '.projects[] | .name' "$MANIFEST"
}

project_manifest_field() {
  local name="$1" field="$2"
  jq -r --arg n "$name" --arg f "$field" '.projects[] | select(.name == $n) | getpath($f | split("."))' "$MANIFEST"
}

run_matrix() {
  log_section "Step 2 — per-project verification matrix (26 projects × 6 checks)"

  # Header row
  {
    echo ""
    echo "| Project | Presence | Git HEAD | Stashes (t/m) | cas.db integrity | Task count (t/m) | Size drift | Overall |"
    echo "|---|---|---|---|---|---|---|---|"
  } >> "$LOG_FILE"

  local pass_count=0 warn_count=0 fail_count=0
  local stash_total_target=0 stash_total_manifest=0
  local casdb_ok_count=0 casdb_check_count=0

  local name
  while read -r name; do
    local rpath="$HOME/projects/$name"

    # Manifest values
    local m_is_repo m_stash m_size m_has_casdb m_tasks_open m_tasks_closed
    m_is_repo=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.is_repo // false' "$MANIFEST")
    m_stash=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .git.stash_count // 0' "$MANIFEST")
    m_size=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .size_bytes_excluding_regenerable' "$MANIFEST")
    m_has_casdb=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .cas_db.present // false' "$MANIFEST")
    m_tasks_open=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .cas_db.task_counts.open // 0' "$MANIFEST")
    m_tasks_closed=$(jq -r --arg n "$name" '.projects[] | select(.name == $n) | .cas_db.task_counts.closed // 0' "$MANIFEST")
    local m_tasks=$(( m_tasks_open + m_tasks_closed ))

    # Run all 6 checks on the target in a single SSH hop for speed.
    local row_data
    row_data=$(ssh "${SSH_OPTS[@]}" "$HOST" "bash -s" <<REMOTE || true
set -e
proj="\$HOME/projects/$name"
cd "\$proj" 2>/dev/null || { echo "PRESENCE=MISSING|||||||"; exit 0; }

# presence
presence=\$(test -d .git -o ! -d .git && echo OK || echo MISSING)
if [[ "$m_is_repo" == "true" ]]; then
  presence=\$(test -d .git && echo OK || echo MISSING)
fi

# git HEAD
if [[ -d .git ]]; then
  head_sha=\$(git rev-parse HEAD 2>/dev/null || echo "-")
else
  head_sha=non-repo
fi

# stash count
if [[ -d .git ]]; then
  stash=\$(git stash list 2>/dev/null | wc -l)
else
  stash=0
fi

# cas.db integrity + task counts
integrity=n/a
tasks=n/a
if [[ -f .cas/cas.db ]]; then
  integrity=\$(python3 -c "import sqlite3; print(sqlite3.connect('.cas/cas.db', timeout=5).execute('PRAGMA integrity_check').fetchone()[0])" 2>&1 || echo ERROR)
  tasks=\$(python3 -c "import sqlite3; c=sqlite3.connect('.cas/cas.db', timeout=5); r=c.execute('SELECT COUNT(*) FROM tasks').fetchone(); print(r[0])" 2>&1 || echo ERROR)
fi

# du
du_b=\$(du -sb . 2>/dev/null | awk '{print \$1}')

echo "PRESENCE=\$presence|HEAD=\$head_sha|STASH=\$stash|INTEGRITY=\$integrity|TASKS=\$tasks|DU=\$du_b"
REMOTE
)

    # Parse row_data pipe-delimited
    local t_presence t_head t_stash t_integrity t_tasks t_du
    t_presence=$(echo "$row_data" | sed -E 's/.*PRESENCE=([^|]*).*/\1/')
    t_head=$(echo "$row_data" | sed -E 's/.*HEAD=([^|]*).*/\1/')
    t_stash=$(echo "$row_data" | sed -E 's/.*STASH=([^|]*).*/\1/')
    t_integrity=$(echo "$row_data" | sed -E 's/.*INTEGRITY=([^|]*).*/\1/')
    t_tasks=$(echo "$row_data" | sed -E 's/.*TASKS=([^|]*).*/\1/')
    t_du=$(echo "$row_data" | sed -E 's/.*DU=([^|]*).*/\1/')

    # Compute laptop git HEAD if is_repo
    local lap_head="n/a"
    if [[ "$m_is_repo" == "true" ]]; then
      lap_head=$(git -C "$HOME/Petrastella/$name" rev-parse HEAD 2>/dev/null || echo "-")
    fi

    # Verdicts per column
    local v_presence="FAIL" v_head="FAIL" v_stash="FAIL" v_integ="n/a" v_tasks="n/a" v_size="FAIL"
    [[ "$t_presence" == "OK" ]] && v_presence="PASS"

    if [[ "$m_is_repo" != "true" ]]; then
      v_head="n/a"
    elif [[ "$t_head" == "$lap_head" ]]; then
      v_head="PASS"
    elif [[ -n "$t_head" && "$t_head" != "-" ]]; then
      v_head="WARN"  # different commits, but both present → source drift, not failure
    fi

    if [[ "$t_stash" == "$m_stash" ]]; then v_stash="PASS"; else v_stash="FAIL"; fi

    if [[ "$m_has_casdb" == "true" ]]; then
      casdb_check_count=$((casdb_check_count + 1))
      if [[ "$t_integrity" == "ok" ]]; then
        v_integ="PASS"
        casdb_ok_count=$((casdb_ok_count + 1))
      else
        v_integ="FAIL"
      fi
      # task count parity ±5
      if [[ "$t_tasks" =~ ^[0-9]+$ ]] && [[ "$m_tasks" -gt 0 ]]; then
        local delta
        delta=$(( t_tasks > m_tasks ? t_tasks - m_tasks : m_tasks - t_tasks ))
        if (( delta <= 5 )); then
          v_tasks="PASS"
        else
          v_tasks="WARN"
        fi
      else
        v_tasks="n/a"
      fi
    fi

    # Size ±10%
    if [[ "$t_du" =~ ^[0-9]+$ ]] && [[ "$m_size" -gt 0 ]]; then
      local low high
      low=$(awk -v e="$m_size" 'BEGIN{printf "%d", e*0.9}')
      high=$(awk -v e="$m_size" 'BEGIN{printf "%d", e*1.1 + 1000}')
      if (( t_du >= low && t_du <= high )); then
        v_size="PASS"
      else
        v_size="WARN"
      fi
    fi

    # Overall verdict
    local overall="PASS"
    for v in "$v_presence" "$v_head" "$v_stash" "$v_integ" "$v_tasks" "$v_size"; do
      case "$v" in
        FAIL) overall="FAIL"; break ;;
        WARN) [[ "$overall" != "FAIL" ]] && overall="WARN" ;;
      esac
    done
    case "$overall" in
      PASS) pass_count=$((pass_count + 1)) ;;
      WARN) warn_count=$((warn_count + 1)) ;;
      FAIL) fail_count=$((fail_count + 1)) ;;
    esac

    stash_total_target=$((stash_total_target + t_stash))
    stash_total_manifest=$((stash_total_manifest + m_stash))

    # Emit row
    printf '| %s | %s | %s | %s/%s | %s | %s/%s | %s | **%s** |\n' \
      "$name" "$v_presence" "$v_head" \
      "$t_stash" "$m_stash" "$v_integ" \
      "$t_tasks" "$m_tasks" "$v_size" "$overall" \
      >> "$LOG_FILE"
  done < <(list_projects)

  {
    echo ""
    echo "**Totals**: $pass_count PASS, $warn_count WARN, $fail_count FAIL of 26"
    echo ""
    echo "**Stash total**: target=$stash_total_target manifest=$stash_total_manifest (57 expected from Phase 1)"
    echo ""
    echo "**CAS DB integrity**: $casdb_ok_count/$casdb_check_count projects PASS"
    echo ""
  } >> "$LOG_FILE"

  # Export for completion report
  MATRIX_PASS=$pass_count
  MATRIX_WARN=$warn_count
  MATRIX_FAIL=$fail_count
  STASH_TARGET=$stash_total_target
  STASH_MANIFEST=$stash_total_manifest
  CASDB_OK=$casdb_ok_count
  CASDB_TOTAL=$casdb_check_count
}

# ---------------------------------------------------------------------------
# Step 3: env audit (absorbs Phase 4)
# ---------------------------------------------------------------------------

run_env_audit() {
  log_section "Step 3 — env audit running"

  local ts
  ts="$(date -Iseconds)"
  cat > "$ENV_AUDIT_FILE" <<EOF
# Phase 8 env audit — per-project \`.env*\` files on target

**Task**: cas-dece (absorbs Phase 4)
**Generated**: $ts
**Target**: \`$HOST:~/projects/\`

This audit inventories every \`.env*\` file that traveled in Phase 3, reports
KEY names only (never values), flags localhost/127.0.0.1/pippenz references,
and classifies each file as \`benign\` (references to local services the app
doesn't actually reach at runtime — e.g., docker postgres overridden by a
cloud endpoint in \`.env.local\`) or \`actionable\` (references the app
actually uses, which will break on the server).

Classification logic: for each file with localhost refs, we check whether
another \`.env*\` in the same project (higher in the load order —
\`.env.local\` typically wins) overrides the localhost-referenced key with a
non-localhost value. If yes, benign. If no, actionable.

## Summary table

| Project | env files (count) | Localhost refs total | Benign | Actionable |
|---|---|---|---|---|
EOF

  # For each project, for each env file, for each localhost-referenced key,
  # determine benign vs actionable.
  declare -A PROJECT_BENIGN
  declare -A PROJECT_ACTIONABLE
  declare -A PROJECT_ENVFILES
  declare -A PROJECT_LOCALHOST_REFS
  declare -a ACTIONABLE_DETAILS

  local name
  while read -r name; do
    local proj_rpath="~/projects/$name"
    local env_count=0 lh_total=0 benign=0 actionable=0

    # Remote: list all .env* files and for each, extract keys + localhost refs
    local project_env_data
    project_env_data=$(ssh "${SSH_OPTS[@]}" "$HOST" "bash -s" <<REMOTE || true
set -e
cd "\$HOME/projects/$name" 2>/dev/null || exit 0
# Find all .env* files (not .env.example; not .env.sample but include them if they contain real refs)
find . -maxdepth 4 -type f \\( -name '.env' -o -name '.env.*' \\) -not -path '*/node_modules/*' -not -path '*/.git/*' 2>/dev/null | while read -r f; do
  # KEY=localhost-ref lines (no value capture — just the key name)
  lh_keys=\$(grep -E '^\\s*(export\\s+)?[A-Z_][A-Z0-9_]*\\s*=' "\$f" 2>/dev/null \\
    | grep -E '(localhost|127\\.0\\.0\\.1|0\\.0\\.0\\.0|pippenz)' \\
    | sed -E 's/^\\s*(export\\s+)?([A-Z_][A-Z0-9_]*)\\s*=.*/\\2/' \\
    | sort -u | tr '\\n' ',' | sed 's/,$//')
  total_keys=\$(grep -cE '^\\s*(export\\s+)?[A-Z_][A-Z0-9_]*\\s*=' "\$f" 2>/dev/null || echo 0)
  size=\$(stat -c %s "\$f" 2>/dev/null || echo 0)
  mode=\$(stat -c %a "\$f" 2>/dev/null || echo '-')
  echo "FILE|\$f|\$size|\$mode|\$total_keys|\$lh_keys"
done
REMOTE
)

    # Build an associative list of (file, localhost_keys) so we can cross-check
    declare -A FILE_LH_KEYS=()
    declare -A FILE_META=()
    local file_list=()
    while IFS='|' read -r tag fpath size mode total_keys lh_keys; do
      [[ "$tag" == "FILE" ]] || continue
      env_count=$((env_count + 1))
      FILE_META[$fpath]="$size $mode $total_keys"
      FILE_LH_KEYS[$fpath]="$lh_keys"
      file_list+=("$fpath")
    done <<<"$project_env_data"

    # For each localhost-referenced key in each file, check if overridden by
    # a higher-priority .env file. The standard dotenv load order (Node ecosystems):
    #   .env.local > .env.<NODE_ENV>.local > .env.<NODE_ENV> > .env
    # For simplicity, we treat ".env.local" as the highest-priority override
    # and check if it contains the SAME key with a non-localhost value.
    local override_file=".env.local"
    local override_keys_non_lh="(none)"
    if [[ -n "${FILE_META[./$override_file]+x}" ]]; then
      # Fetch keys in override file whose value does NOT match localhost regex
      override_keys_non_lh=$(ssh "${SSH_OPTS[@]}" "$HOST" "bash -s" <<REMOTE2 || true
cd "\$HOME/projects/$name" 2>/dev/null || exit 0
grep -E '^\\s*(export\\s+)?[A-Z_][A-Z0-9_]*\\s*=' "./$override_file" 2>/dev/null \\
  | grep -vE '(localhost|127\\.0\\.0\\.1|0\\.0\\.0\\.0|pippenz)' \\
  | sed -E 's/^\\s*(export\\s+)?([A-Z_][A-Z0-9_]*)\\s*=.*/\\2/' \\
  | sort -u | tr '\\n' ',' | sed 's/,$//'
REMOTE2
)
    fi

    # Iterate, classify
    for fpath in "${file_list[@]}"; do
      local lh_keys="${FILE_LH_KEYS[$fpath]}"
      [[ -z "$lh_keys" ]] && continue
      IFS=',' read -ra key_arr <<<"$lh_keys"
      for key in "${key_arr[@]}"; do
        [[ -z "$key" ]] && continue
        lh_total=$((lh_total + 1))
        # benign if key appears in override_keys_non_lh
        if [[ ",$override_keys_non_lh," == *",$key,"* ]]; then
          benign=$((benign + 1))
        else
          actionable=$((actionable + 1))
          ACTIONABLE_DETAILS+=("$name $fpath $key")
        fi
      done
    done

    printf '| %s | %s | %s | %s | %s |\n' "$name" "$env_count" "$lh_total" "$benign" "$actionable" >> "$ENV_AUDIT_FILE"

    PROJECT_BENIGN[$name]=$benign
    PROJECT_ACTIONABLE[$name]=$actionable
    PROJECT_ENVFILES[$name]=$env_count
    PROJECT_LOCALHOST_REFS[$name]=$lh_total
  done < <(list_projects)

  {
    echo ""
    echo "## REQUIRES HUMAN — actionable localhost refs"
    echo ""
    if [[ "${#ACTIONABLE_DETAILS[@]}" -eq 0 ]]; then
      echo "None. Every localhost-referenced env key is overridden by \`.env.local\` with a non-localhost value. No manual rewrite required for the server copies."
    else
      echo "The following env keys point at localhost in their primary \`.env\` and are NOT overridden by \`.env.local\`. If the server runs these apps, these need rewriting to the target's networked values:"
      echo ""
      echo "| Project | File | Key |"
      echo "|---|---|---|"
      local row
      for row in "${ACTIONABLE_DETAILS[@]}"; do
        local p f k
        read -r p f k <<<"$row"
        printf '| %s | %s | %s |\n' "$p" "$f" "$k"
      done
    fi
    echo ""
    echo "## Classification methodology"
    echo ""
    echo "1. For each project, list all \`.env\` + \`.env.*\` files (excluding \`node_modules\`, \`.git\`)."
    echo "2. For each file, extract KEY names (no values) of lines whose value matches localhost / 127.0.0.1 / 0.0.0.0 / pippenz."
    echo "3. For each such key, check if \`.env.local\` (the highest-priority loader in Node ecosystems) defines the same key with a non-localhost value. If yes → **benign**. If no → **actionable**."
    echo "4. Values are never captured or logged. Only keys, file paths, counts, sizes, and modes."
    echo ""
    echo "## Caveats"
    echo ""
    echo "- Projects that use env loaders other than the standard Node dotenv chain may have different override semantics. The spec assumes the common case."
    echo "- Python projects and Next.js 14 projects use \`.env.local\` similarly, so the heuristic generalizes."
    echo "- The heuristic does NOT detect runtime-constant overrides (e.g., \`DATABASE_URL\` hardcoded in \`prisma.config.ts\`). Any such cases would appear in the actionable list and require human review."
  } >> "$ENV_AUDIT_FILE"

  log_line "env_audit_written" "$ENV_AUDIT_FILE"
  log_line "total_env_files_audited" "$(grep -c '^| [a-z]' "$ENV_AUDIT_FILE" || echo 0)"
  log_line "total_actionable_keys" "${#ACTIONABLE_DETAILS[@]}"
  ENV_ACTIONABLE_COUNT="${#ACTIONABLE_DETAILS[@]}"
}

# ---------------------------------------------------------------------------
# Step 4: smoke test on one project (absorbs Phase 6)
# ---------------------------------------------------------------------------

run_smoke_test() {
  log_section "Step 4 — smoke test on \`$SMOKE_PROJECT\` (pnpm install + typecheck)"

  log_line "smoke_project" "$SMOKE_PROJECT"
  log_line "smoke_policy" "DO NOT install any other project; DO NOT run test suites; typecheck is sufficient"

  # pnpm install --frozen-lockfile
  log_line "smoke_step" "pnpm install --frozen-lockfile"
  local install_tmp
  install_tmp="$(mktemp -t phase8-install.XXXXXX)"
  local install_rc=0
  ssh "${SSH_OPTS[@]}" "$HOST" "cd ~/projects/$SMOKE_PROJECT && pnpm install --frozen-lockfile 2>&1" > "$install_tmp" 2>&1 || install_rc=$?
  log_line "install_rc" "$install_rc"
  tail -30 "$install_tmp" | log_code_block "pnpm install tail"
  SMOKE_INSTALL_RC=$install_rc
  SMOKE_INSTALL_TAIL_PATH="$install_tmp"

  if [[ "$install_rc" -ne 0 ]]; then
    log_line "smoke_status" "INSTALL FAILED — typecheck skipped"
    SMOKE_TYPECHECK_RC="skipped"
    return 0
  fi

  # pnpm typecheck
  log_line "smoke_step" "pnpm typecheck"
  local tc_tmp
  tc_tmp="$(mktemp -t phase8-typecheck.XXXXXX)"
  local tc_rc=0
  ssh "${SSH_OPTS[@]}" "$HOST" "cd ~/projects/$SMOKE_PROJECT && pnpm typecheck 2>&1" > "$tc_tmp" 2>&1 || tc_rc=$?
  log_line "typecheck_rc" "$tc_rc"
  tail -30 "$tc_tmp" | log_code_block "pnpm typecheck tail"
  SMOKE_TYPECHECK_RC=$tc_rc
  rm -f "$tc_tmp"

  if [[ "$tc_rc" -eq 0 ]]; then
    log_line "smoke_status" "PASS (install rc=0, typecheck rc=0)"
  else
    log_line "smoke_status" "FAIL (install rc=0, typecheck rc=$tc_rc)"
  fi
}

# ---------------------------------------------------------------------------
# Entrypoints
# ---------------------------------------------------------------------------

MATRIX_PASS=0
MATRIX_WARN=0
MATRIX_FAIL=0
STASH_TARGET=0
STASH_MANIFEST=0
CASDB_OK=0
CASDB_TOTAL=0
ENV_ACTIONABLE_COUNT=0
SMOKE_INSTALL_RC=""
SMOKE_TYPECHECK_RC=""

main_full() {
  preflight
  log_init
  run_matrix
  run_env_audit
  run_smoke_test
  echo "[main] Phase 8 verification pass complete" >&2
  echo "[main] matrix: $MATRIX_PASS PASS, $MATRIX_WARN WARN, $MATRIX_FAIL FAIL of 26" >&2
  echo "[main] stashes: target=$STASH_TARGET manifest=$STASH_MANIFEST" >&2
  echo "[main] cas.db integrity: $CASDB_OK/$CASDB_TOTAL" >&2
  echo "[main] env audit: $ENV_ACTIONABLE_COUNT actionable localhost refs" >&2
  echo "[main] smoke: install_rc=$SMOKE_INSTALL_RC typecheck_rc=$SMOKE_TYPECHECK_RC" >&2
}

main_preflight() { preflight; }
main_matrix() { preflight; [[ -f "$LOG_FILE" ]] || log_init; run_matrix; }
main_env_audit() { preflight; [[ -f "$LOG_FILE" ]] || log_init; run_env_audit; }
main_smoke() { preflight; [[ -f "$LOG_FILE" ]] || log_init; run_smoke_test; }

cmd="${1:-full}"
case "$cmd" in
  full)      main_full ;;
  preflight) main_preflight ;;
  matrix)    main_matrix ;;
  env-audit) main_env_audit ;;
  smoke)     main_smoke ;;
  *) die "unknown command: $cmd" ;;
esac
