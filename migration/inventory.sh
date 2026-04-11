#!/usr/bin/env bash
# migration/inventory.sh
#
# Phase 1 Petrastella → Hetzner inventory generator.
#
# READ-ONLY on ~/Petrastella. Writes `migration/manifest.json` next to this script.
# Idempotent: running twice produces byte-identical output modulo `generated_at`.
#
# Requires: bash 4+, jq, python3 (with pyyaml), GNU coreutils (du, stat), find, grep,
# git (optional — per-project repo introspection), docker (optional — runtime state).
# sqlite3 CLI is NOT required; cas.db probing uses python3's stdlib sqlite3.
#
# Usage:
#   ./migration/inventory.sh                       # default SOURCE_ROOT=~/Petrastella
#   SOURCE_ROOT=/some/other ./migration/inventory.sh
#   OUTPUT=/tmp/manifest.json ./migration/inventory.sh
#
# Exit codes:
#   0  success
#   1  precondition failure (missing tool, SOURCE_ROOT missing, etc.)

set -euo pipefail

# Force deterministic collation for sort / grep / awk across hosts. Without this
# the idempotency claim ("two runs produce byte-identical output") only holds
# within one locale; a second run on a host with a different LC_ALL would diff.
export LC_ALL=C

###############################################################################
# Config
###############################################################################

SOURCE_ROOT="${SOURCE_ROOT:-$HOME/Petrastella}"
TARGET_ROOT="${TARGET_ROOT:-daniel@87.99.156.244:~/projects}"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
OUTPUT="${OUTPUT:-$SCRIPT_DIR/manifest.json}"

# Regenerable directories whose size is reported separately and subtracted from
# "size_bytes_excluding_regenerable". Kept sorted alphabetically for determinism.
REGENERABLE_DIRS=(
  .cas/worktrees
  .next
  .nuxt
  .output
  .pnpm-store
  .svelte-kit
  .turbo
  build
  coverage
  dist
  node_modules
  playwright-report
  target
  test-results
)

###############################################################################
# Preconditions
###############################################################################

die() { printf 'inventory.sh: %s\n' "$*" >&2; exit 1; }

command -v jq >/dev/null      || die "jq is required"
command -v python3 >/dev/null  || die "python3 is required"
python3 -c 'import yaml' 2>/dev/null || die "python3 pyyaml module is required"
command -v du >/dev/null       || die "GNU coreutils du is required"

[ -d "$SOURCE_ROOT" ] || die "SOURCE_ROOT does not exist: $SOURCE_ROOT"

mkdir -p "$(dirname "$OUTPUT")"

###############################################################################
# Docker pre-flight (optional)
###############################################################################

DOCKER_AVAILABLE=false
DOCKER_PS_JSON='[]'
DOCKER_VOLUME_JSON='[]'
DOCKER_SYSTEM_DF_JSON='{}'

if command -v docker >/dev/null 2>&1 && docker info >/dev/null 2>&1; then
  DOCKER_AVAILABLE=true
  # Gather runtime state ONCE, globally. Per-project filters below just select
  # matching rows from these caches. Keeps idempotency checks tractable.
  DOCKER_PS_JSON=$(docker ps --all --format '{{json .}}' 2>/dev/null \
    | python3 -c '
import json, sys
rows = []
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        rows.append(json.loads(line))
    except Exception:
        pass
# Keep only stable fields. Drop CreatedAt/Status/RunningFor which embed deltas.
keep = ["Names", "State", "Image", "Labels"]
rows = [{k: r.get(k, "") for k in keep} for r in rows]
rows.sort(key=lambda r: r["Names"])
print(json.dumps(rows))
' 2>/dev/null || echo '[]')

  DOCKER_VOLUME_JSON=$(docker volume ls --format '{{json .}}' 2>/dev/null \
    | python3 -c '
import json, sys
rows = []
for line in sys.stdin:
    line = line.strip()
    if not line: continue
    try:
        rows.append(json.loads(line))
    except Exception:
        pass
rows.sort(key=lambda r: r.get("Name",""))
print(json.dumps(rows))
' 2>/dev/null || echo '[]')

  # docker system df -v gives per-volume Size in human format. Parse carefully.
  DOCKER_SYSTEM_DF_JSON=$(docker system df -v --format '{{json .Volumes}}' 2>/dev/null \
    | python3 -c '
import json, sys
data = sys.stdin.read().strip()
if not data:
    print("{}"); sys.exit(0)
try:
    rows = json.loads(data)
except Exception:
    print("{}"); sys.exit(0)
def parse_size(s):
    # Handles "1.234GB", "567MB", "12kB", "0B", "123B"
    if not isinstance(s, str) or not s: return 0
    s = s.strip()
    mult = 1
    for suf, m in (("GB", 1000**3), ("MB", 1000**2), ("kB", 1000), ("B", 1)):
        if s.endswith(suf):
            try:
                return int(float(s[:-len(suf)]) * m)
            except Exception:
                return 0
    try:
        return int(s)
    except Exception:
        return 0
out = {}
for r in rows or []:
    name = r.get("Name") or r.get("VolumeName") or ""
    if not name: continue
    out[name] = parse_size(r.get("Size", "0B"))
print(json.dumps(out, sort_keys=True))
' 2>/dev/null || echo '{}')
fi

###############################################################################
# Helpers
###############################################################################

# du -sb with consistent error handling. Always prints a bare integer.
safe_du_b() {
  local p="$1"
  if [ ! -e "$p" ]; then echo 0; return; fi
  du -sb "$p" 2>/dev/null | awk 'NR==1 {print $1+0}' || echo 0
}

# Extract KEY names from a KEY=VALUE file. NEVER emits values.
# Handles:
#   KEY=value
#   export KEY=value
#   leading whitespace
# Ignores:
#   # comment lines
#   blank lines
#   lines without `=`
env_key_names() {
  local f="$1"
  grep -E '^[[:space:]]*(export[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=' "$f" 2>/dev/null \
    | sed -E 's/^[[:space:]]*(export[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*=.*/\2/' \
    | sort -u
}

# Keys whose VALUE matched a localhost/bind-host pattern. Emits key names only.
# Two-stage filter:
#   1. Select real KEY=VALUE assignment lines (same shape as env_key_names above)
#      — this drops comments, blank lines, and stray text.
#   2. Of those, keep only lines whose VALUE portion matches the localhost
#      regex. Extract the KEY name.
# Stage 1 is essential because the old single-stage grep would match a comment
# like `# DB_HOST=localhost` and emit `DB_HOST` as localhost-referencing, even
# though the line is inactive.
env_localhost_keys() {
  local f="$1"
  grep -E '^[[:space:]]*(export[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*[[:space:]]*=.*(localhost|127\.0\.0\.1|0\.0\.0\.0|://pippenz)' "$f" 2>/dev/null \
    | sed -E 's/^[[:space:]]*(export[[:space:]]+)?([A-Za-z_][A-Za-z0-9_]*)[[:space:]]*=.*/\2/' \
    | sort -u
}

redact_remote_url() {
  # Redact credentials embedded in `git remote -v` URLs so tokens never land
  # in manifest.json (which is committed).
  #
  # Handles three forms, across any RFC-3986 scheme:
  #   1. <scheme>://user:token@host/…   → <scheme>://***:***@host/…
  #   2. <scheme>://token@host/…        → <scheme>://***@host/…    (bearer)
  #   3. SCP-style git@host:path.git    → unchanged (user name `git` is public)
  #
  # The bearer-form pass MUST run after the user:pass pass so credentials with
  # a colon aren't double-substituted into "***@".
  sed -E '
    s#([a-zA-Z][a-zA-Z0-9+.-]*://)[^@/:[:space:]]+:[^@[:space:]]+@#\1***:***@#g
    s#([a-zA-Z][a-zA-Z0-9+.-]*://)[^@/:[:space:]]+@#\1***@#g
  '
}

###############################################################################
# Per-project collection
###############################################################################

collect_project_json() {
  local path="$1"
  local name
  name="$(basename "$path")"

  # --- sizes ---
  # Top-level project size (one du invocation — hard links deduped once).
  local total_bytes
  total_bytes=$(safe_du_b "$path")

  # size_bytes_excluding_regenerable is computed with a SINGLE `du --exclude`
  # pass so hard links are counted consistently with the total. Subtracting
  # per-child du's is incorrect because `.cas/worktrees` and `.git` share
  # hardlinked blobs, so Σ(children) often exceeds the parent total.
  local excl_args=()
  local d
  for d in "${REGENERABLE_DIRS[@]}"; do
    excl_args+=(--exclude="$d")
  done
  local size_excl=0
  if [ -e "$path" ]; then
    size_excl=$(du -sb "${excl_args[@]}" "$path" 2>/dev/null | awk 'NR==1 {print $1+0}' || echo 0)
  fi
  [ -z "$size_excl" ] && size_excl=0
  [ "$size_excl" -gt "$total_bytes" ] && size_excl="$total_bytes"

  # Regenerable aggregate is derived so it matches total - excl exactly.
  local regenerable_total=$((total_bytes - size_excl))
  [ "$regenerable_total" -lt 0 ] && regenerable_total=0

  # Per-dir sizes are still recorded for visibility, but are NOT summed — they
  # double-count hard links across dirs so we explicitly flag that in the key
  # name. Each entry is a standalone du of that dir.
  local regenerable_json='{}'
  for d in "${REGENERABLE_DIRS[@]}"; do
    local sz=0
    if [ -e "$path/$d" ]; then
      sz=$(safe_du_b "$path/$d")
    fi
    regenerable_json=$(jq --arg k "$d" --argjson v "$sz" '. + {($k): $v}' <<<"$regenerable_json")
  done

  # --- git ---
  local git_json='{"is_repo": false}'
  if [ -d "$path/.git" ] || [ -f "$path/.git" ]; then
    local branch head_sha porcelain dirty untracked stash unpushed summary
    local remotes_json

    branch=$(git -C "$path" rev-parse --abbrev-ref HEAD 2>/dev/null || echo "")
    head_sha=$(git -C "$path" rev-parse HEAD 2>/dev/null || echo "")

    local remotes
    remotes=$(git -C "$path" remote -v 2>/dev/null | awk '{print $2}' | sort -u | redact_remote_url)
    if [ -n "$remotes" ]; then
      remotes_json=$(printf '%s\n' "$remotes" | jq -R . | jq -s '.')
    else
      remotes_json='[]'
    fi

    porcelain=$(git -C "$path" status --porcelain 2>/dev/null || true)
    dirty=$(printf '%s\n' "$porcelain" | awk 'NF && $1!="??" {c++} END {print c+0}')
    untracked=$(printf '%s\n' "$porcelain" | awk 'NF && $1=="??" {c++} END {print c+0}')
    stash=$(git -C "$path" stash list 2>/dev/null | wc -l | awk '{print $1+0}')
    # A branch without a configured upstream makes `git log @{u}..HEAD` exit
    # 128. Under `set -o pipefail` + `set -e` that would otherwise abort the
    # entire inventory mid-scan. A branch with no upstream genuinely has 0
    # pushable commits relative to any upstream, so swallowing the failure
    # with `|| echo 0` is semantically correct.
    unpushed=$(git -C "$path" log '@{u}..HEAD' --oneline 2>/dev/null | wc -l | awk '{print $1+0}' || echo 0)
    # Safety net: if the pipeline still produced non-numeric output for any
    # reason (empty string, garbage), default to 0 so later jq / arithmetic
    # cannot fail on an empty integer.
    [[ "$unpushed" =~ ^[0-9]+$ ]] || unpushed=0

    # NOTE: we deliberately do NOT record the `git status --porcelain` output
    # as a `status_summary` field. Two reasons:
    #   1. It can contain filenames that happen to match the secret-scan
    #      regex `sk-|pk_|xoxb-|AKIA|...` (e.g. `.claude/skills/cas-task-
    #      tracking/` → "sk-tracking") and blow up the P0 secrets grep with
    #      false positives.
    #   2. The dirty_files / untracked_files / stash_count / unpushed_commits
    #      counts already give Phase 0 decision-makers the signal they need.
    #      Anyone who wants file-level detail can cd into the project and run
    #      `git status` directly — this manifest is committed, status output
    #      is not.

    git_json=$(jq -n \
      --arg branch "$branch" \
      --arg head_sha "$head_sha" \
      --argjson remote_urls "$remotes_json" \
      --argjson dirty_files "$dirty" \
      --argjson untracked_files "$untracked" \
      --argjson stash_count "$stash" \
      --argjson unpushed_commits "$unpushed" \
      '{
        is_repo: true,
        current_branch: $branch,
        head_sha: $head_sha,
        remote_urls: $remote_urls,
        dirty_files: $dirty_files,
        untracked_files: $untracked_files,
        stash_count: $stash_count,
        unpushed_commits: $unpushed_commits
      }')
  fi

  # --- .cas/cas.db ---
  local cas_db_json='{"present": false}'
  local cas_db_path="$path/.cas/cas.db"
  if [ -f "$cas_db_path" ]; then
    local db_size wal_present shm_present probe
    db_size=$(stat -c%s "$cas_db_path" 2>/dev/null || echo 0)
    wal_present=$([ -f "${cas_db_path}-wal" ] && echo true || echo false)
    shm_present=$([ -f "${cas_db_path}-shm" ] && echo true || echo false)

    probe=$(python3 - "$cas_db_path" <<'PY' 2>/dev/null || echo '{"probe_error":"python_failed"}'
import json, sqlite3, sys
p = sys.argv[1]
out = {"tables": [], "task_counts": {}, "integrity_check": "unknown"}
try:
    con = sqlite3.connect(f"file:{p}?mode=ro", uri=True, timeout=2)
    cur = con.cursor()
    cur.execute("SELECT name FROM sqlite_master WHERE type='table'")
    out["tables"] = sorted([r[0] for r in cur.fetchall()])
    if "tasks" in out["tables"]:
        try:
            cur.execute("SELECT status, COUNT(*) FROM tasks GROUP BY status")
            out["task_counts"] = {str(r[0]): int(r[1]) for r in cur.fetchall()}
        except Exception as e:
            out["task_counts_error"] = str(e)
    try:
        cur.execute("PRAGMA integrity_check")
        out["integrity_check"] = str(cur.fetchone()[0])
    except Exception as e:
        out["integrity_check"] = f"error: {e}"
    con.close()
except Exception as e:
    out["probe_error"] = str(e)
print(json.dumps(out, sort_keys=True))
PY
)
    cas_db_json=$(jq -n \
      --argjson size_bytes "$db_size" \
      --argjson wal_present "$wal_present" \
      --argjson shm_present "$shm_present" \
      --argjson probe "$probe" \
      '{present: true, path: ".cas/cas.db", size_bytes: $size_bytes, wal_present: $wal_present, shm_present: $shm_present} + $probe')
  fi

  # --- docker ---
  local compose_file=""
  local f
  for f in docker-compose.yml docker-compose.yaml compose.yml compose.yaml; do
    if [ -f "$path/$f" ]; then compose_file="$f"; break; fi
  done

  local docker_json
  if [ -z "$compose_file" ]; then
    docker_json='{"compose_file": null, "services": [], "images": [], "named_volumes": [], "bind_mounts": [], "running_containers": [], "volume_sizes": {}}'
  else
    # Parse compose file statically — NEVER execute `docker compose config`
    # (that would interpolate .env values into the manifest).
    local compose_parsed
    compose_parsed=$(python3 - "$path/$compose_file" <<'PY' 2>/dev/null || echo '{"parse_error": "python_failed"}'
import json, os, sys
try:
    import yaml
except Exception as e:
    print(json.dumps({"parse_error": f"yaml import: {e}"})); sys.exit(0)
p = sys.argv[1]
try:
    with open(p, "r") as fh:
        doc = yaml.safe_load(fh) or {}
    if not isinstance(doc, dict):
        print(json.dumps({"parse_error": "compose root is not a mapping"})); sys.exit(0)
    services_map = doc.get("services") or {}
    if not isinstance(services_map, dict): services_map = {}
    services = sorted(services_map.keys())
    images = []
    bind_mounts = []
    # Named volumes referenced from services.*.volumes. Compose permits
    # service-scoped named volumes without a matching top-level `volumes:`
    # key (older compose v2 files do this). Missing them would mean the
    # `docker_named_volumes` risk flag never fires for such a project and
    # Phase 0 plans no dump/restore — silent data loss at cutover.
    service_scoped_named_volumes = set()
    for sname, svc in services_map.items():
        if not isinstance(svc, dict): continue
        img = svc.get("image")
        if isinstance(img, str) and img:
            images.append(img.strip())
        for v in (svc.get("volumes") or []):
            if isinstance(v, str):
                # e.g. "./data:/var/lib/postgres" or "$PWD/data:/..." or "vol:/..."
                src = v.split(":", 1)[0]
                if src.startswith(("/", ".", "~", "$")):
                    bind_mounts.append(src)
                elif src:
                    # Named volume reference (no leading path sigil).
                    service_scoped_named_volumes.add(src)
            elif isinstance(v, dict):
                if v.get("type") == "bind":
                    src = v.get("source", "")
                    if src: bind_mounts.append(str(src))
                elif v.get("type") == "volume":
                    src = v.get("source", "")
                    if src: service_scoped_named_volumes.add(str(src))
    vol_map = doc.get("volumes") or {}
    if not isinstance(vol_map, dict): vol_map = {}
    # Merge top-level volume declarations with service-scoped references so
    # any named volume a service actually uses appears in the manifest,
    # whether or not it was declared at the top.
    named_volumes = sorted(set(vol_map.keys()) | service_scoped_named_volumes)
    out = {
        "compose_file": os.path.basename(p),
        "services": services,
        "images": sorted(set(images)),
        "named_volumes": named_volumes,
        "bind_mounts": sorted(set(bind_mounts)),
    }
    print(json.dumps(out, sort_keys=True))
except Exception as e:
    print(json.dumps({"parse_error": str(e)}))
PY
)

    # Match running containers + volumes by docker-compose project label.
    # docker-compose project name defaults to basename lowercased with
    # [^a-z0-9] stripped.
    local proj_label
    proj_label=$(printf '%s' "$name" | tr '[:upper:]' '[:lower:]' | tr -cd 'a-z0-9_-')

    local running_containers='[]'
    local volume_sizes='{}'
    if [ "$DOCKER_AVAILABLE" = "true" ]; then
      # IMPORTANT: pass the JSON blobs via environment variables, NOT via
      # shell interpolation into a triple-quoted Python string literal. The
      # earlier approach `json.loads('''$DOCKER_PS_JSON''')` broke whenever a
      # docker label value contained `'''` or `\` and — worse — allowed a
      # malicious container name to inject Python. Quoted heredoc (<<'PY')
      # suppresses shell expansion; Python reads the blobs from os.environ.
      running_containers=$(DOCKER_PS_JSON="$DOCKER_PS_JSON" python3 - "$proj_label" <<'PY'
import json, os, sys
proj = sys.argv[1]
rows = json.loads(os.environ.get("DOCKER_PS_JSON", "[]"))
out = []
for r in rows:
    lbls = r.get("Labels", "") or ""
    name = r.get("Names", "")
    match = False
    if f"com.docker.compose.project={proj}" in lbls: match = True
    elif name.startswith(f"{proj}-") or name.startswith(f"{proj}_"): match = True
    if match:
        out.append({"name": name, "state": r.get("State",""), "image": r.get("Image","")})
out.sort(key=lambda x: x["name"])
print(json.dumps(out))
PY
)
      volume_sizes=$(DOCKER_VOLUME_JSON="$DOCKER_VOLUME_JSON" DOCKER_SYSTEM_DF_JSON="$DOCKER_SYSTEM_DF_JSON" python3 - "$proj_label" <<'PY'
import json, os, sys
proj = sys.argv[1]
rows = json.loads(os.environ.get("DOCKER_VOLUME_JSON", "[]"))
sizes = json.loads(os.environ.get("DOCKER_SYSTEM_DF_JSON", "{}"))
out = {}
for r in rows:
    vn = r.get("Name","")
    lbls = r.get("Labels","") or ""
    match = False
    if f"com.docker.compose.project={proj}" in lbls: match = True
    elif vn.startswith(f"{proj}_") or vn.startswith(f"{proj}-"): match = True
    if match:
        out[vn] = sizes.get(vn, 0)
print(json.dumps(out, sort_keys=True))
PY
)
    fi

    docker_json=$(jq -n \
      --argjson parsed "$compose_parsed" \
      --argjson running "$running_containers" \
      --argjson volsize "$volume_sizes" \
      '$parsed + {running_containers: $running, volume_sizes: $volsize}')
  fi

  # --- env files ---
  local env_files_json='[]'
  local env_list
  env_list=$(find "$path" -maxdepth 1 -type f -name '.env*' 2>/dev/null | sort)
  if [ -n "$env_list" ]; then
    local parts='[]'
    while IFS= read -r ef; do
      [ -z "$ef" ] && continue
      local base sz mode keys_json lh_keys_json lh_count has_lh
      base=$(basename "$ef")
      sz=$(stat -c%s "$ef" 2>/dev/null || echo 0)
      mode=$(stat -c%A "$ef" 2>/dev/null || echo "?")

      local keys_raw lh_keys_raw
      keys_raw=$(env_key_names "$ef")
      lh_keys_raw=$(env_localhost_keys "$ef")

      if [ -n "$keys_raw" ]; then
        keys_json=$(printf '%s\n' "$keys_raw" | jq -R . | jq -s '.')
      else
        keys_json='[]'
      fi
      if [ -n "$lh_keys_raw" ]; then
        lh_keys_json=$(printf '%s\n' "$lh_keys_raw" | jq -R . | jq -s '.')
      else
        lh_keys_json='[]'
      fi
      lh_count=$(printf '%s\n' "$lh_keys_raw" | awk 'NF' | wc -l | awk '{print $1+0}')
      has_lh=$([ "$lh_count" -gt 0 ] && echo true || echo false)

      local entry
      entry=$(jq -n \
        --arg path "$base" \
        --argjson size "$sz" \
        --arg modes "$mode" \
        --argjson has_localhost_refs "$has_lh" \
        --argjson localhost_ref_count "$lh_count" \
        --argjson keys_with_localhost_refs "$lh_keys_json" \
        --argjson env_keys "$keys_json" \
        '{
          path: $path,
          size: $size,
          modes: $modes,
          has_localhost_refs: $has_localhost_refs,
          localhost_ref_count: $localhost_ref_count,
          keys_with_localhost_refs: $keys_with_localhost_refs,
          env_keys: $env_keys
        }')
      parts=$(jq --argjson e "$entry" '. + [$e]' <<<"$parts")
    done <<< "$env_list"
    env_files_json=$(jq 'sort_by(.path)' <<<"$parts")
  fi

  # --- package managers / languages ---
  local pm_arr=()
  local lang_arr=()

  [ -f "$path/pnpm-lock.yaml" ]       && pm_arr+=("pnpm")
  [ -f "$path/yarn.lock" ]            && pm_arr+=("yarn")
  [ -f "$path/package-lock.json" ]    && pm_arr+=("npm")
  [ -f "$path/bun.lockb" ]            && pm_arr+=("bun")
  [ -f "$path/Cargo.toml" ]           && pm_arr+=("cargo")
  [ -f "$path/pyproject.toml" ]       && pm_arr+=("python")
  [ -f "$path/uv.lock" ]              && pm_arr+=("uv")
  [ -f "$path/poetry.lock" ]          && pm_arr+=("poetry")
  [ -f "$path/requirements.txt" ]     && pm_arr+=("pip")
  [ -f "$path/go.mod" ]               && pm_arr+=("go")

  [ -f "$path/package.json" ]   && lang_arr+=("javascript")
  [ -f "$path/Cargo.toml" ]     && lang_arr+=("rust")
  [ -f "$path/pyproject.toml" ] && lang_arr+=("python")
  [ -f "$path/requirements.txt" ] && lang_arr+=("python")
  [ -f "$path/go.mod" ]         && lang_arr+=("go")

  if [ -f "$path/package.json" ]; then
    grep -q '"typescript"' "$path/package.json" 2>/dev/null && lang_arr+=("typescript")
    grep -q '"vue"'        "$path/package.json" 2>/dev/null && lang_arr+=("vue")
    grep -q '"next"'       "$path/package.json" 2>/dev/null && lang_arr+=("nextjs")
    grep -q '"nuxt"'       "$path/package.json" 2>/dev/null && lang_arr+=("nuxt")
    grep -q '"svelte"'     "$path/package.json" 2>/dev/null && lang_arr+=("svelte")
    grep -q '"@quasar/app' "$path/package.json" 2>/dev/null && lang_arr+=("quasar")
  fi

  local pkg_mgrs_json languages_json
  if [ "${#pm_arr[@]}" -gt 0 ]; then
    pkg_mgrs_json=$(printf '%s\n' "${pm_arr[@]}" | jq -R . | jq -s 'unique')
  else
    pkg_mgrs_json='[]'
  fi
  if [ "${#lang_arr[@]}" -gt 0 ]; then
    languages_json=$(printf '%s\n' "${lang_arr[@]}" | jq -R . | jq -s 'unique')
  else
    languages_json='[]'
  fi

  # --- assemble project object ---
  jq -n \
    --arg name "$name" \
    --arg path "$path" \
    --argjson size_bytes "$total_bytes" \
    --argjson size_bytes_excluding_regenerable "$size_excl" \
    --argjson regenerable_dirs_standalone_bytes "$regenerable_json" \
    --argjson regenerable_bytes_total "$regenerable_total" \
    --argjson git "$git_json" \
    --argjson cas_db "$cas_db_json" \
    --argjson docker "$docker_json" \
    --argjson env_files "$env_files_json" \
    --argjson package_managers "$pkg_mgrs_json" \
    --argjson languages "$languages_json" \
    '{
      name: $name,
      path: $path,
      size_bytes: $size_bytes,
      size_bytes_excluding_regenerable: $size_bytes_excluding_regenerable,
      regenerable_bytes_total: $regenerable_bytes_total,
      regenerable_dirs_standalone_bytes: $regenerable_dirs_standalone_bytes,
      git: $git,
      cas_db: $cas_db,
      docker: $docker,
      env_files: $env_files,
      package_managers: $package_managers,
      languages: $languages
    }'
}

###############################################################################
# Global state
###############################################################################

collect_global_state() {
  local cas_db="$HOME/.cas/cas.db"
  local cas_db_present=false
  local cas_db_size=0
  if [ -f "$cas_db" ]; then
    cas_db_present=true
    cas_db_size=$(stat -c%s "$cas_db" 2>/dev/null || echo 0)
  fi

  local claude_projects="$HOME/.claude/projects"
  local cp_count=0
  local cp_present=false
  if [ -d "$claude_projects" ]; then
    cp_present=true
    cp_count=$(find "$claude_projects" -maxdepth 1 -mindepth 1 -type d 2>/dev/null | wc -l | awk '{print $1+0}')
  fi
  # NOTE: deliberately NOT reporting size_bytes for ~/.claude/projects/ — it is
  # continuously written during factory sessions (per-agent JSONL transcripts)
  # so any byte count would break idempotency checks without meaningful value.
  # If you need a snapshot, take it while no factory session is running.

  local settings="$HOME/.claude/settings.json"
  local settings_present=false
  [ -f "$settings" ] && settings_present=true

  local env_file="$HOME/.config/cas/env"
  local env_present=false
  [ -f "$env_file" ] && env_present=true

  jq -n \
    --argjson cas_db_present "$cas_db_present" \
    --argjson cas_db_size "$cas_db_size" \
    --argjson cp_present "$cp_present" \
    --argjson cp_count "$cp_count" \
    --argjson settings_present "$settings_present" \
    --argjson env_present "$env_present" \
    '{
      cas_global_db: {
        path: "~/.cas/cas.db",
        present: $cas_db_present,
        size_bytes: $cas_db_size
      },
      claude_projects_memory: {
        path: "~/.claude/projects/",
        present: $cp_present,
        project_count: $cp_count,
        size_bytes_note: "omitted — directory is continuously written by factory sessions; take a snapshot with factory stopped if a byte count is required"
      },
      claude_settings: {
        path: "~/.claude/settings.json",
        present: $settings_present
      },
      config_cas_env: {
        path: "~/.config/cas/env",
        present: $env_present
      }
    }'
}

###############################################################################
# Aggregate pass — totals, risk flags, docker/secret summaries
###############################################################################

aggregate() {
  # Stdin: the projects array (JSON). Stdout: aggregate object.
  jq '
    . as $projects
    | {
        totals: {
          project_count: ($projects | length),
          total_bytes: ($projects | map(.size_bytes) | add // 0),
          total_bytes_excluding_regenerable: ($projects | map(.size_bytes_excluding_regenerable) | add // 0),
          regenerable_bytes: ($projects | map(.regenerable_bytes_total) | add // 0)
        },
        secrets_summary: {
          env_file_count: ($projects | map(.env_files | length) | add // 0),
          projects_with_env: ($projects | map(select((.env_files | length) > 0) | .name)),
          projects_with_localhost_refs: (
            $projects
            | map(select(any(.env_files[]?; .has_localhost_refs == true)) | .name)
            | sort
          )
        },
        docker_summary: {
          projects_with_compose: (
            $projects | map(select(.docker.compose_file != null) | .name) | sort
          ),
          total_named_volumes: (
            $projects | map(.docker.named_volumes | length) | add // 0
          ),
          total_volume_bytes: (
            $projects
            | map(.docker.volume_sizes | to_entries | map(.value) | add // 0)
            | add // 0
          ),
          currently_running_containers: (
            $projects
            | map(.docker.running_containers[]?)
            | sort_by(.name)
          )
        },
        risk_flags: (
          (
            $projects
            | map(
                select((.git.stash_count // 0) > 0)
                | {
                    severity: "high",
                    project: .name,
                    kind: "git_stash_present",
                    issue: ("\(.git.stash_count) stash entries — will be lost if not handled explicitly before cutover")
                  }
              )
          )
          + (
            $projects
            | map(
                select((.git.unpushed_commits // 0) > 0)
                | {
                    severity: "high",
                    project: .name,
                    kind: "unpushed_commits",
                    issue: ("\(.git.unpushed_commits) local commits not pushed to upstream — push or cherry-pick before decommissioning source")
                  }
              )
          )
          + (
            $projects
            | map(
                select(((.git.dirty_files // 0) + (.git.untracked_files // 0)) > 0)
                | {
                    severity: "high",
                    project: .name,
                    kind: "uncommitted_wip",
                    issue: ("dirty=\(.git.dirty_files // 0), untracked=\(.git.untracked_files // 0) — uncommitted WIP at risk during migration")
                  }
              )
          )
          + (
            $projects
            | map(
                select(any(.env_files[]?; .has_localhost_refs == true))
                | {
                    severity: "high",
                    project: .name,
                    kind: "localhost_env_refs",
                    issue: "env files contain localhost/127.0.0.1/0.0.0.0/pippenz references — will need rewriting for remote host"
                  }
              )
          )
          + (
            $projects
            | map(
                select(.git.is_repo == true and (.git.remote_urls | length) == 0)
                | {
                    severity: "medium",
                    project: .name,
                    kind: "no_git_remote",
                    issue: "git repo has no remotes — nothing to clone from; must rsync or push a new origin before migration"
                  }
              )
          )
          + (
            $projects
            | map(
                select((.cas_db.integrity_check // "") != "" and (.cas_db.integrity_check // "ok") != "ok")
                | {
                    severity: "high",
                    project: .name,
                    kind: "cas_db_integrity",
                    issue: ("cas.db integrity_check returned: \(.cas_db.integrity_check)")
                  }
              )
          )
          + (
            $projects
            | map(
                select(.cas_db.wal_present == true)
                | {
                    severity: "medium",
                    project: .name,
                    kind: "cas_db_wal",
                    issue: "cas.db WAL file present — checkpoint before transfer to avoid inconsistent copy"
                  }
              )
          )
          + (
            $projects
            | map(
                select((.cas_db.probe_error // "") != "" and .cas_db.wal_present == true)
                | {
                    severity: "high",
                    project: .name,
                    kind: "cas_db_locked_during_inventory",
                    issue: ("cas.db probe failed (\(.cas_db.probe_error // "unknown")) AND WAL file is present — a factory session is likely holding the write lock; task_counts and tables are UNRELIABLE for this project; re-run the inventory with the factory stopped")
                  }
              )
          )
          + (
            $projects
            | map(
                select(.docker.named_volumes != null and (.docker.named_volumes | length) > 0)
                | {
                    severity: "high",
                    project: .name,
                    kind: "docker_named_volumes",
                    issue: ("docker named volumes \(.docker.named_volumes) — require explicit volume dump/restore, not just rsync")
                  }
              )
          )
          + (
            $projects
            | map(
                select(.docker.bind_mounts != null and (.docker.bind_mounts | length) > 0)
                | {
                    severity: "medium",
                    project: .name,
                    kind: "docker_bind_mounts",
                    issue: ("docker bind mounts \(.docker.bind_mounts) — verify host paths on target match")
                  }
              )
          )
          + (
            $projects
            | map(
                select(
                  (.package_managers | index("pnpm")) or
                  (.package_managers | index("yarn")) or
                  (.package_managers | index("npm"))
                )
                | {
                    severity: "medium",
                    project: .name,
                    kind: "native_module_rebuild",
                    issue: "node project — native modules (better-sqlite3, bcrypt, etc.) may need rebuild on target architecture"
                  }
              )
          )
          | sort_by(
              (if .severity == "high" then 0 elif .severity == "medium" then 1 else 2 end),
              .project,
              .kind
            )
        )
      }
  '
}

###############################################################################
# Main
###############################################################################

main() {
  # Enumerate project dirs — sorted for determinism, excludes non-directories.
  local project_dirs=()
  while IFS= read -r -d '' dir; do
    project_dirs+=("$dir")
  done < <(find "$SOURCE_ROOT" -mindepth 1 -maxdepth 1 -type d -print0 | sort -z)

  [ "${#project_dirs[@]}" -gt 0 ] || die "No project directories under $SOURCE_ROOT"

  echo "inventory.sh: scanning ${#project_dirs[@]} projects under $SOURCE_ROOT" >&2

  # Build projects array
  local projects_json='[]'
  local p
  for p in "${project_dirs[@]}"; do
    local name
    name=$(basename "$p")
    printf '  - %s\n' "$name" >&2
    local proj
    proj=$(collect_project_json "$p")
    projects_json=$(jq --argjson item "$proj" '. + [$item]' <<<"$projects_json")
  done
  projects_json=$(jq 'sort_by(.name)' <<<"$projects_json")

  local aggregate_json
  aggregate_json=$(printf '%s' "$projects_json" | aggregate)

  local global_json
  global_json=$(collect_global_state)

  # Absent daemon is a stand-alone risk flag
  local docker_meta_flags='[]'
  if [ "$DOCKER_AVAILABLE" != "true" ]; then
    docker_meta_flags=$(jq -n '[{
      severity: "medium",
      project: "(global)",
      kind: "docker_daemon_unavailable",
      issue: "docker daemon not running or not reachable from inventory script — runtime state (containers, volume sizes) not captured. Re-run with daemon up before cutover decisions."
    }]')
  fi

  # Build final manifest. generated_at is set LAST so its position is stable.
  local generated_at
  generated_at=$(date -u '+%Y-%m-%dT%H:%M:%SZ')

  local inventory_script_version="1.0.0"

  jq -n \
    --arg generated_at "$generated_at" \
    --arg source_root "$SOURCE_ROOT" \
    --arg target_root "$TARGET_ROOT" \
    --arg version "$inventory_script_version" \
    --argjson docker_available "$DOCKER_AVAILABLE" \
    --argjson projects "$projects_json" \
    --argjson aggregate "$aggregate_json" \
    --argjson global_state "$global_json" \
    --argjson docker_meta_flags "$docker_meta_flags" \
    '{
      generated_at: $generated_at,
      inventory_script_version: $version,
      source_root: $source_root,
      target_root: $target_root,
      docker_daemon_available: $docker_available,
      totals: $aggregate.totals,
      projects: $projects,
      global_state: $global_state,
      secrets_summary: $aggregate.secrets_summary,
      docker_summary: $aggregate.docker_summary,
      risk_flags: ($docker_meta_flags + $aggregate.risk_flags)
        | sort_by(
            (if .severity == "high" then 0 elif .severity == "medium" then 1 else 2 end),
            .project,
            .kind
          )
    }' > "$OUTPUT.tmp"

  mv "$OUTPUT.tmp" "$OUTPUT"
  echo "inventory.sh: wrote $OUTPUT" >&2
}

main "$@"
