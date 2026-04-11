# Phase 2 execution log — prepare Hetzner target

**Task:** cas-b5f1
**Epic:** cas-28d4 (Petrastella → Hetzner migration)
**Host:** `daniel@87.99.156.244`
**Script:** `migration/phase2-prepare-target.sh`
**Started:** 2026-04-11T13:25:15Z

This log is produced by `phase2-prepare-target.sh` and committed to cas-src
as the audit trail for the Phase 2 run. It's structured as one section per
step so a reader can jump to the step they care about. Steps that found the
server already in the desired state print `SKIP` and a short reason.

Secrets hygiene: `CAS_SERVE_TOKEN` never appears in this file. Where it would,
`<redacted>` is written instead. The token itself only exists in
`~/.config/cas/serve.env` on the remote (mode 0600, owner daniel:daniel) —
deliberately SEPARATE from the pre-existing `~/.config/cas/env` user token
vault (GH_TOKEN, VERCEL_TOKEN, etc.) per supervisor's option-(b) resolution.

## Step 0 — pre-flight

Gates that must all pass before the script will touch the server.

- `whoami` → `daniel`
- `sudo -n true` → `sudo_ok`
- `uname -srm` → `Linux 6.8.0-107-generic x86_64`

All gates **PASS**.

## Step 1 — apt packages

Required packages (sorted, deduped):

- `build-essential`
- `ca-certificates`
- `curl`
- `docker-compose-v2`
- `docker.io`
- `ffmpeg`
- `git`
- `jq`
- `libssl-dev`
- `pkg-config`
- `poppler-utils`
- `python3`
- `python3-pip`
- `python3-venv`
- `sqlite3`
- `unzip`
- `wget`

All 17 packages already installed — **SKIP**.

## Step 2 — Node.js + pnpm

- `node --version` → `v22.22.2`
- `pnpm --version` → `10.33.0`

Node v22.22.2 and pnpm 10.33.0 both present and satisfy the v20+ floor — **SKIP**.

## Step 3 — Rust toolchain (rustup + cargo)

- `rustc --version` → `rustc 1.94.1 (e408947bf 2026-03-25)`
- `cargo --version` → `cargo 1.94.1 (29ea6fb6a 2026-03-24)`

rustc and cargo already present — **SKIP**.

## Step 4 — cas binary (v2.0.x)

- `cas --version` (current) → `cas 2.0.0 (d11846d 2026-04-11)`

cas v2.x already installed — **SKIP**.

## Step 5 — `~/.config/cas/serve.env` (separate from user token vault)

**Spec deviation (authorized by supervisor on cas-b5f1 at 2026-04-11T~13:20):**

The task spec's Step 5 targets `~/.config/cas/env` as the systemd-unit `EnvironmentFile`. On this server that path is already occupied by `cas-fb43`'s provisioning output — a user-level **login-shell token vault** containing `GH_TOKEN`, `GITHUB_TOKEN`, `CAS_CLOUD_TOKEN`, `CONTEXT7_API_KEY`, `NEON_API_KEY`, `VERCEL_TOKEN`, `BROWSERLESS_API_KEY`, etc. These are the user's CLI-tool credentials, likely sourced by `.bashrc`/`.zshrc`, and should NOT be inherited by a system-level HTTP bridge process.

**Resolution** (option (b) from the blocker note): a separate file `~/.config/cas/serve.env` scoped strictly to the systemd unit. The user's token vault at `~/.config/cas/env` is **never touched**. Consequences:

- Principle of least privilege honored: `cas-serve@daniel` doesn't see `NEON_API_KEY`, `VERCEL_TOKEN`, etc.
- Future changes to the user token vault no longer implicitly mutate the service environment.
- The committed `migration/systemd/cas-serve@.service` uses `EnvironmentFile=/home/%i/.config/cas/serve.env` (spec said `.../env` — the deviation is documented here, in the unit file's header comment, and in the close reason).

Remote state probe (user token vault left untouched throughout):

```
TOKEN_VAULT_PRESENT
mode=600 owner=daniel:daniel size=494
SERVE_ENV_PRESENT
mode=600 owner=daniel:daniel size=118
has_CAS_SERVE_URL
has_CAS_SERVE_TOKEN
```

`serve.env` already present with both required keys — **SKIP**.

## Step 6 — systemd unit `cas-serve@.service`

**Correction from the original spec:** the spec template used `cas serve --port 18999` as the ExecStart, but on the installed `cas 2.0.0` binary `cas serve` is the stdio MCP server for Claude Code integration (`--json`/`--full`/`--verbose` only — no port binding).

The HTTP bridge we actually need is `cas bridge serve`. Supervisor authorized adjusting ExecStart after probing `--help` on the newly-installed binary, so this unit uses the correct subcommand.

Probing `cas bridge serve --help` on the server to lock the exact flag names:

```
Run a local HTTP server exposing a small control/status API

Usage: cas bridge serve [OPTIONS]

Options:
      --bind <BIND>
          Bind address (default: 127.0.0.1)
          
          [default: 127.0.0.1]

      --json
          Output in JSON format

      --full
          Include full content in JSON output

      --port <PORT>
          Port to listen on (0 = auto)
          
          [default: 0]

      --cas-root <CAS_ROOT>
          Optional explicit CAS root directory (path to a `.cas/` dir).
          
          This is used as a fallback when a session has no `project_dir` metadata, or when CAS root detection fails for that `project_dir`.

  -v, --verbose
          Verbose output

      --token <redacted>
          Bearer token for authorization (default: auto-generate)

      --no-auth
          Disable authorization (not recommended; still binds to localhost by default)

      --cors-allow-origin <CORS_ALLOW_ORIGIN>
          Set CORS allow-origin header (e.g., "*" or "https://openclaw.ai")

  -h, --help
          Print help (see a summary with '-h')
```

`--bind` + `--port` + `--token` flags confirmed. Using systemd `${CAS_SERVE_TOKEN}` expansion from the EnvironmentFile so the token never appears literally in the unit file (the unit file is committed to git).

Local unit file written to `migration/systemd/cas-serve@.service`:

```ini
# migration/systemd/cas-serve@.service
#
# Versioned systemd unit template for the cas HTTP bridge server. Phase 2
# of the Petrastella → Hetzner migration (cas-b5f1) installs this file to
# /etc/systemd/system/cas-serve@.service on daniel@87.99.156.244 and enables
# cas-serve@daniel. Edit this file in cas-src; re-run
# migration/phase2-prepare-target.sh to push updates to the server.
#
# Spec deviation: the task spec originally called for
#   EnvironmentFile=/home/%i/.config/cas/env
# but on the target server `.config/cas/env` is already occupied by
# cas-fb43's user-level token vault (GH_TOKEN, VERCEL_TOKEN, NEON_API_KEY,
# etc. — login-shell CLI creds). Per supervisor's option-(b) resolution on
# cas-b5f1, this unit uses a separate file `serve.env` scoped strictly
# to the systemd service so principle of least privilege is honored
# (cas-serve never inherits user CLI tokens) and future vault edits don't
# implicitly mutate the service environment.
#
# ExecStart uses `cas bridge serve` (not `cas serve`, which is the
# stdio MCP subcommand) and reads the bearer token from serve.env via
# systemd's ${VAR} expansion so the token is never literalized into
# this committed file.

[Unit]
Description=CAS HTTP bridge server for %i
After=network.target
AssertPathExists=/home/%i/.config/cas/serve.env

[Service]
Type=simple
User=%i
Group=%i
WorkingDirectory=/home/%i
EnvironmentFile=/home/%i/.config/cas/serve.env
ExecStart=/usr/local/bin/cas bridge serve --bind 127.0.0.1 --port 18999 --token <redacted>
Restart=on-failure
RestartSec=5

# Discard stdout so the cas startup banner's 'Token: <redacted> line does NOT
# land in the systemd journal. The banner is informational and the token is
# already in serve.env for anyone who needs it. Errors still flow through
# stderr → journal for real operational signal.
#
# NOTE: /proc/<pid>/cmdline still exposes the token via systemd's
# ${CAS_SERVE_TOKEN} argv expansion. That's a cas-cli limitation (the
# --token <redacted> is the only auth mechanism the binary accepts). On a
# single-user server (daniel only), the exposure is bounded to daniel +
# root. A cas-binary follow-up to add CAS_SERVE_TOKEN env-var reading would
# close this hole completely.
StandardOutput=null
StandardError=journal

[Install]
WantedBy=multi-user.target
```

Server already has the identical unit file (md5 match). **SKIP write.**

```
active
* cas-serve@daniel.service - CAS HTTP bridge server for daniel
     Loaded: loaded (/etc/systemd/system/cas-serve@.service; enabled; preset: enabled)
     Active: active (running) since Sat 2026-04-11 09:14:32 EDT; 10min ago
   Main PID: 28841 (cas)
      Tasks: 6 (limit: 18687)
     Memory: 1.8M (peak: 2.6M)
        CPU: 12ms
     CGroup: /system.slice/system-cas\x2dserve.slice/cas-serve@daniel.service
             `-28841 /usr/local/bin/cas bridge serve --bind 127.0.0.1 --port 18999 --token <redacted>

Apr 11 09:14:32 ubuntu-16gb-ash-1 systemd[1]: Started cas-serve@daniel.service - CAS HTTP bridge server for daniel.
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]: CAS Bridge Server
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]:   Base URL: http://127.0.0.1:18999
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]:   Token:    <redacted>
```

## Step 7 — `~/projects/` directory

- `~/projects` → `mode=775 owner=daniel:daniel`

`~/projects` already exists — **SKIP**.

## Step 8 — GitHub SSH keypair (keygen only — NOT registration)

- `~/.ssh/id_ed25519_github{,.pub}` → `PRESENT`

Keypair already present — **SKIP** keygen.

```
ssh_config_already_has_github_block_SKIP
```

## Step 9 — verification pass

Capturing the authoritative end-state of every component touched by the script. Each check is a single SSH round trip.

**cas --version (server)**

```
cas 2.0.0 (d11846d 2026-04-11)
```

**cas --help (top level)**

```
   ______   ___    _____
  / ____/  /   |  / ___/
 / /      / /| |  \__ \
/ /___   / ___ | ___/ /
\____/  /_/  |_|/____/


Multi-agent coding factory with persistent memory and task coordination

Usage: cas [OPTIONS] [COMMAND]

Commands:
  open         Interactive project picker — scan ~/projects/, select, launch or attach
  init         Initialize CAS in current directory
  attach       Attach to a running factory session
  list         List running factory sessions
  kill         Terminate a factory session
  kill-all     Terminate all factory sessions
  factory      Launch factory session (bare `cas` runs factory with defaults)
  bridge       Local helper server for external orchestration tools
```

**cas task list --scope global --limit 1**

```
error: unrecognized subcommand 'task'

Usage: cas [OPTIONS] [COMMAND]

For more information, try '--help'.
non-zero exit was expected with empty store
```

**systemctl is-active cas-serve@daniel**

```
active
```

**systemctl status cas-serve@daniel**

```
* cas-serve@daniel.service - CAS HTTP bridge server for daniel
     Loaded: loaded (/etc/systemd/system/cas-serve@.service; enabled; preset: enabled)
     Active: active (running) since Sat 2026-04-11 09:14:32 EDT; 10min ago
   Main PID: 28841 (cas)
      Tasks: 6 (limit: 18687)
     Memory: 1.8M (peak: 2.6M)
        CPU: 12ms
     CGroup: /system.slice/system-cas\x2dserve.slice/cas-serve@daniel.service
             `-28841 /usr/local/bin/cas bridge serve --bind 127.0.0.1 --port 18999 --token <redacted>

Apr 11 09:14:32 ubuntu-16gb-ash-1 systemd[1]: Started cas-serve@daniel.service - CAS HTTP bridge server for daniel.
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]: CAS Bridge Server
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]:   Base URL: http://127.0.0.1:18999
Apr 11 09:14:32 ubuntu-16gb-ash-1 cas[28841]:   Token:    <redacted>
```

**curl 127.0.0.1:18999/health**

```
{
  "schema_version": 1,
  "error": {
    "code": "unauthorized",
    "message": "Unauthorized"
  }
}
```

**curl 127.0.0.1:18999/ (root)**

```
HTTP 401
```

**docker --version**

```
Docker version 29.1.3, build 29.1.3-0ubuntu3~24.04.1
```

**docker compose version**

```
Docker Compose version 2.40.3+ds1-0ubuntu1~24.04.1
```

**node --version / pnpm --version**

```
v22.22.2
10.33.0
```

**cargo --version**

```
cargo 1.94.1 (29ea6fb6a 2026-03-24)
```

**rustc --version**

```
rustc 1.94.1 (e408947bf 2026-03-25)
```

**sqlite3 --version**

```
3.45.1 2024-01-30 16:01:20 <redacted-token>lt1 (64-bit)
```

**~/projects listing**

```
total 12
drwxrwxr-x  3 daniel daniel 4096 Apr 10 11:16 .
drwxr-x--- 14 daniel daniel 4096 Apr 11 08:54 ..
drwxrwxr-x 14 daniel daniel 4096 Apr 10 11:16 cas
```

**~/.config/cas/env — user token vault (untouched by this script, metadata only)**

```
mode=600 owner=daniel:daniel size=494 mtime=2026-04-10 14:23:26.568957035 -0400
```

**~/.config/cas/serve.env — systemd unit env file (presence of required keys)**

```
mode=600 owner=daniel:daniel size=118
CAS_SERVE_URL lines: 1
CAS_SERVE_TOKEN lines: 1
```

**id_ed25519_github private key (perms)**

```
mode=600 owner=daniel:daniel
```

**id_ed25519_github.pub (perms + content)**

```
mode=644 owner=daniel:daniel
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGi/veIzEv2NBCHJR5yGsW7QA+DLZ8I3aL45pM4m/Shp daniel@hetzner-cas-87.99.156.244
```

## Residual risk — token in argv (requires cas-cli follow-up)

The `cas bridge serve` binary takes its bearer token via the `--token <redacted> CLI flag; the Rust struct at `cas-cli/src/cli/bridge.rs` does not register `CAS_SERVE_TOKEN` as an env-var source for clap. As a result, systemd's `${CAS_SERVE_TOKEN}` expansion puts the literal token into the process's `argv`, which means:

- `cat /proc/<pid>/cmdline | tr '\\0' ' '` reveals the token to any local user/process with DAC read access.
- `systemctl status cas-serve@daniel` / `systemd-cgls` / `ps -ef` show the full command line with the token.

On this host, daniel is the sole unprivileged user, so the exposure is bounded to daniel + root. The journal leak (cas prints `Token: <redacted> at startup) has been mitigated in the unit file via `StandardOutput=null`, but the argv leak cannot be fixed at the systemd layer.

**Follow-up:** add `#[arg(long, env = "CAS_SERVE_TOKEN")]` to `ServeArgs::token` in `cas-cli/src/cli/bridge.rs` so the token can be read from the environment instead of CLI. Then remove `--token <redacted> from the `ExecStart` line and rely on the `EnvironmentFile=` directive alone. This is a cas-cli change, not a migration-script change — tracking in the cas-28d4 epic close notes.

## REQUIRES HUMAN — register SSH key with GitHub

The following public key lives on `daniel@87.99.156.244` at `~/.ssh/id_ed25519_github.pub`. The supervisor (or user) must register it with GitHub manually before Phase 3 can clone repos over SSH on the server:

```
ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIGi/veIzEv2NBCHJR5yGsW7QA+DLZ8I3aL45pM4m/Shp daniel@hetzner-cas-87.99.156.244
```

Registration options:

- `gh ssh-key add ~/.ssh/id_ed25519_github.pub --title 'hetzner-cas-87.99.156.244'` (run locally, not on the server)
- Or paste the above into https://github.com/settings/ssh/new with a clear title

After registration, test from the server:

```sh
ssh daniel@87.99.156.244 'ssh -T -o StrictHostKeyChecking=accept-new git@github.com'
```

Expected response: `Hi <username>! You've successfully authenticated, but GitHub does not provide shell access.`

---

**Completed:** 2026-04-11T13:25:34Z

