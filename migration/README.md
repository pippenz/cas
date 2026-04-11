# migration/

Phase 1 artifacts for the Petrastella → Hetzner migration (epic `cas-28d4`).

## What's here

| File | Purpose |
|---|---|
| `inventory.sh` | Idempotent, read-only script that scans `~/Petrastella/*/` and emits `manifest.json` |
| `manifest.json` | Snapshot of every project: size, git state, .cas DB, docker compose, .env metadata, risk flags |
| `README.md` | This file |

The manifest is the data source for Phase 0 decisions (scope, docker volume strategy, cutover plan). Phase 1 does **not** make those decisions — it only gathers the data.

## Running the inventory

```sh
./migration/inventory.sh                         # scans ~/Petrastella → writes migration/manifest.json
SOURCE_ROOT=/some/path ./migration/inventory.sh  # override source
OUTPUT=/tmp/foo.json   ./migration/inventory.sh  # override output path
```

The script is **read-only** on the source tree — no git commits, no stashes, no docker ops, no `.env` edits. The only writes are to `migration/manifest.json` (and a temp file during generation).

### Requirements

- `bash` 4+, `jq`, `python3` with `pyyaml`, GNU `du`/`stat`/`find`/`grep`
- `git` (optional — absence just skips per-project repo data)
- `docker` (optional — absence skips runtime container/volume data; compose files are still parsed statically)
- `sqlite3` CLI is **not** required; `.cas/cas.db` probing goes through Python's stdlib `sqlite3`

### Idempotency

Running the script twice produces **byte-identical output modulo the `generated_at` field** on a quiescent host. Verify with:

```sh
OUTPUT=/tmp/m1.json ./migration/inventory.sh
OUTPUT=/tmp/m2.json ./migration/inventory.sh
diff <(grep -v '"generated_at"' /tmp/m1.json) <(grep -v '"generated_at"' /tmp/m2.json)
# should print nothing
```

Caveats that can break idempotency if you're unlucky:
- A project with an active cas-src factory session will have a moving `cas.db`/`-wal` file. Stop the factory first.
- `~/.claude/projects/` is deliberately **not** sized (continuously written by factory sessions) — only `project_count` is recorded.

## Schema walk-through

Top-level:

```jsonc
{
  "generated_at": "ISO8601 UTC",
  "inventory_script_version": "1.0.0",
  "source_root": "/home/pippenz/Petrastella",
  "target_root": "daniel@87.99.156.244:~/projects",
  "docker_daemon_available": false,
  "totals": {
    "project_count": 26,
    "total_bytes": ...,
    "total_bytes_excluding_regenerable": ...,  // computed via `du --exclude=...` in ONE pass (hard-link safe)
    "regenerable_bytes": ...                    // derived = total - excluding
  },
  "projects": [ /* one object per ~/Petrastella/*/ */ ],
  "global_state": { /* ~/.cas, ~/.claude, ~/.config/cas */ },
  "secrets_summary": { /* aggregate env-file counts, projects with localhost refs */ },
  "docker_summary": { /* projects with compose, total named volumes */ },
  "risk_flags": [ /* sorted by severity then project then kind */ ]
}
```

### Per-project object

Each project entry includes:

- **`size_bytes`** — `du -sb $path` once
- **`size_bytes_excluding_regenerable`** — `du -sb --exclude=node_modules --exclude=... $path` once
- **`regenerable_bytes_total`** — derived (`size_bytes − size_bytes_excluding_regenerable`), so it matches the total exactly
- **`regenerable_dirs_standalone_bytes`** — standalone `du -sb` of each regenerable child dir. **These are NOT additive** — `.cas/worktrees` and `.git` share hard-linked blobs so Σ(children) often exceeds `size_bytes`. Use them as per-dir hints, not for totals.
- **`git`** — `is_repo`, `current_branch`, `head_sha`, `remote_urls` (token-redacted), `dirty_files`, `untracked_files`, `stash_count`, `unpushed_commits`
- **`cas_db`** — presence, size, `tables`, `task_counts`, `integrity_check`, `wal_present`, `shm_present`
- **`docker`** — parsed `compose_file`, `services`, `images`, `named_volumes`, `bind_mounts` (parsed statically from YAML — never via `docker compose config` which would interpolate `.env` values into the manifest), plus live `running_containers` and `volume_sizes` if the daemon is up
- **`env_files`** — for each `.env*` file: path, size, modes, `env_keys` (**KEY names only, never values**), `has_localhost_refs`, `localhost_ref_count`, `keys_with_localhost_refs`
- **`package_managers`**, **`languages`** — detected by presence of lockfiles + `package.json` hints

### Reading `risk_flags`

Each flag is `{severity, project, kind, issue}`. Severity is `high`, `medium`, or `low`. Flags are sorted high → medium → low, then by project name, then by kind.

The `kind` field is a stable machine-readable key — use it in filters:

| kind | Meaning | Recommended action before cutover |
|---|---|---|
| `git_stash_present` | Project has `git stash list` entries | Pop-and-commit, or `git stash show -p > patch.diff` and carry the diff |
| `unpushed_commits` | Local commits not on upstream | `git push` before decommissioning source — rsync won't move refs |
| `uncommitted_wip` | Dirty or untracked files | Commit, stash, or explicitly discard |
| `localhost_env_refs` | `.env` values reference `localhost` / `127.0.0.1` / `0.0.0.0` / `pippenz` | Rewrite for remote host. KEY names are recorded in `keys_with_localhost_refs` — values are **not** (secrets hygiene) |
| `no_git_remote` | Git repo has no remotes | Must rsync the whole tree, or push a new origin first |
| `cas_db_integrity` | `PRAGMA integrity_check` returned something other than `ok` | Repair or rebuild before transfer |
| `cas_db_wal` | `cas.db-wal` exists at snapshot time | `pragma wal_checkpoint(TRUNCATE);` before copying to avoid inconsistent snapshot |
| `docker_named_volumes` | Compose file declares named volumes | Requires explicit dump/restore (e.g. `pg_dump` → `pg_restore`, or `docker run --rm -v vol:/data -v $PWD:/backup alpine tar`). rsync of the compose file alone does **not** move the data |
| `docker_bind_mounts` | Compose uses host bind mounts | Verify host paths exist on target |
| `native_module_rebuild` | Node project — may use `better-sqlite3`, `bcrypt`, etc. | Rebuild on target architecture (`pnpm rebuild` / `npm rebuild`) |
| `docker_daemon_unavailable` | Inventory ran with docker down | Re-run with daemon up if you need live volume sizes / running container lists |

Typical pre-cutover reads:

```sh
# Everything that will lose data if we don't handle it
jq '[.risk_flags[] | select(.severity=="high")]' migration/manifest.json

# Projects with pending local work
jq '.projects[] | select(.git.stash_count>0 or .git.unpushed_commits>0 or .git.dirty_files>0) | {name, stash_count: .git.stash_count, unpushed: .git.unpushed_commits, dirty: .git.dirty_files, untracked: .git.untracked_files}' migration/manifest.json

# Compose projects and their named volumes (need per-service dump plans)
jq '.projects[] | select(.docker.compose_file != null) | {name, services: .docker.services, named_volumes: .docker.named_volumes}' migration/manifest.json
```

## Secrets hygiene

The manifest is committed to git. `inventory.sh` will never write a secret value into it:

- `.env*` files: only KEY names + reference counts are recorded. Values (including the string after `localhost` URLs) are **not** read into memory past the regex match that counts them.
- `git remote -v` URLs: `https://user:token@host/...` is redacted to `https://***:***@host/...` before landing in the manifest.
- `docker-compose.yml` is parsed as static YAML via PyYAML — we explicitly do **not** call `docker compose config`, because that interpolates `${VAR}` references against the project's `.env` and would leak secrets into the output.
- `git status --porcelain` output is **not** included as a string field. The counts (`dirty_files`, `untracked_files`) convey the same Phase-0 signal without the risk of a filename coincidentally matching a secret-prefix grep.

Before committing, run the secrets check:

```sh
# Prefixes covered:
#   xoxb-          Slack bot tokens
#   sk-            Anthropic (sk-ant-*), OpenAI (sk-proj-*), Stripe (sk_live_*, sk_test_*)
#   pk_            Stripe publishable keys
#   AKIA           AWS long-lived access key IDs
#   ASIA           AWS STS temporary access key IDs
#   GOCSPX         Google OAuth client secrets
#   AIzaSy         Google API keys
#   ghp_           GitHub classic PATs
#   github_pat_    GitHub fine-grained PATs
#   ya29\.         Google OAuth access tokens
#   service_account Google service-account JSON blobs
#   -----BEGIN     PEM-encoded private keys / certificates
for pat in 'xoxb-' 'sk-' 'pk_' 'AKIA' 'ASIA' 'GOCSPX' 'AIzaSy' 'ghp_' 'github_pat_' 'ya29\.' 'service_account' '-----BEGIN'; do
  echo "[$pat] $(grep -cE -- "$pat" migration/manifest.json)"
done
# all counts must be 0
```
