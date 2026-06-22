//! PTY management using portable-pty
//!
//! Provides a wrapper around portable-pty with:
//! - Async read/write operations
//! - Raw byte output (terminal parsing done by ghostty_vt)
//! - Resize support

use crate::error::{Error, Result};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;

/// Instructions injected into Codex supervisor agents via `--config developer_instructions`.
const CODEX_SUPERVISOR_INSTRUCTIONS: &str = "You are the CAS Factory Supervisor. Coordinate only: plan epics, assign tasks, monitor progress, review/merge. Never implement tasks. Use skills cas-supervisor and cas-codex-supervisor-checklist. Use MCP tools explicitly; no /cas-start, /cas-context, or /cas-end. Worker messages (status/blocker/ready) arrive asynchronously as new injected turns framed 'Message from <sender>: …'. Each is a triage trigger, not a fresh startup: read it, then assign/answer/redirect/merge as appropriate and reply via `mcp__cs__coordination action=message target=<worker>`. Finishing one round does not mean you are done — remain available to coordinate the next message.";

/// Instructions injected into Codex worker agents via `--config developer_instructions`.
const CODEX_WORKER_INSTRUCTIONS: &str = "You are a CAS Factory Worker. Always use CAS MCP tools for task lifecycle and coordination. On startup your CAS session is already registered automatically — do NOT call session_start. Just run `mcp__cs__coordination action=whoami` then `mcp__cs__task action=mine`. Work exactly ONE task at a time: choose a single assigned task, run `mcp__cs__task action=show id=<task-id>` then `mcp__cs__task action=start id=<task-id>` before coding, implement it, commit and push your changes, then close it with `mcp__cs__task action=close id=<task-id> reason=\"...\"` (or hand it to the supervisor if close returns verification-required guidance) BEFORE starting any other task. Never start a second task while one is still in progress — the verification jail allows only one unverified in-progress task at a time, so batch-starting tasks will block you. Add progress notes frequently using `mcp__cs__task action=notes id=<task-id> note_type=progress notes=\"...\"`. For blockers, add a blocker note, set `status=blocked`, and message supervisor via `mcp__cs__coordination action=message target=supervisor message=\"...\"`. If close returns verification-required guidance, immediately ask the supervisor to verify/close on your behalf. After closing or handing off a task, stay available — you are not permanently done. The supervisor will send you more work or redirection as new messages. Treat any injected turn framed 'Message from <sender>: …' as an instruction to act on, not noise: read it and follow it. Keep honoring one-task-at-a-time — finish or hand off your current task before starting the next one a message assigns. Do not use /cas-start, /cas-context, or /cas-end. Stay within assigned task scope.";

/// Prefix for the Codex worker startup prompt. The worker name is appended at runtime.
const CODEX_WORKER_STARTUP_PREFIX: &str = "I'm initiating CAS worker startup now: confirm identity, check assigned tasks, then start any assigned task with a progress note. My CAS session is already registered automatically (do NOT call session_start).\n1) Run mcp__cs__coordination action=whoami";

/// Configuration for spawning a PTY
#[derive(Debug, Clone)]
pub struct PtyConfig {
    /// Command to run (e.g., "claude")
    pub command: String,
    /// Arguments for the command
    pub args: Vec<String>,
    /// Working directory
    pub cwd: Option<PathBuf>,
    /// Environment variables to set
    pub env: Vec<(String, String)>,
    /// Initial terminal size
    pub rows: u16,
    pub cols: u16,
}

/// Configuration for spawning an agent with native Claude Code Agent Teams flags.
#[derive(Debug, Clone)]
pub struct TeamsSpawnConfig {
    /// Team name (factory session name)
    pub team_name: String,
    /// Agent ID (e.g., "worker-1@session-name")
    pub agent_id: String,
    /// Agent display name
    pub agent_name: String,
    /// Agent color for UI
    pub agent_color: String,
    /// Agent type (e.g., "team-lead", "general-purpose")
    pub agent_type: String,
    /// Parent session ID for analytics correlation (workers only)
    pub parent_session_id: Option<String>,
    /// Lead session ID — set for the team lead so --session-id matches leadSessionId
    pub lead_session_id: Option<String>,
    /// Optional path to a settings JSON file passed via `--settings <path>`.
    ///
    /// Populated for both the supervisor (`supervisor-settings.json`) and for
    /// every worker (`{worker-name}-settings.json`) so filesystem tool calls
    /// auto-approve from the per-role allowlist instead of escalating through
    /// the team-approval channel. Workers without this file hang on the
    /// phantom `team-lead` mailbox because Claude Code's harness misreads
    /// `agentType="team-lead"` as the lead's display name (upstream bug);
    /// shipping the allowlist eliminates the trigger even while that misread
    /// remains unfixed. See `cas-cli/src/ui/factory/daemon/runtime/teams.rs`
    /// (`supervisor_settings_contents` / `worker_settings_contents`) for the
    /// shape of each file.
    pub settings_path: Option<String>,
}

impl Default for PtyConfig {
    fn default() -> Self {
        Self {
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            rows: 24,
            cols: 80,
        }
    }
}

impl PtyConfig {
    /// Create config for a Claude CLI instance
    ///
    /// # Arguments
    /// * `name` - Agent name
    /// * `role` - Agent role (e.g., "worker", "supervisor")
    /// * `cwd` - Working directory for the agent
    /// * `cas_root` - Optional path to the .cas directory. If provided, sets CAS_ROOT env var
    ///   so workers in clones can access the main repo's CAS state.
    /// * `supervisor_name` - For workers, the name of their supervisor (enables `target: supervisor`)
    #[allow(clippy::too_many_arguments)]
    pub fn claude(
        name: &str,
        role: &str,
        cwd: PathBuf,
        cas_root: Option<&PathBuf>,
        supervisor_name: Option<&str>,
        factory_worker_cli: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        teams: Option<&TeamsSpawnConfig>,
    ) -> Self {
        // Use the lead_session_id for the team lead so leadSessionId in the
        // team config matches the supervisor's --session-id. Without this,
        // Claude Code thinks it's not the leader and won't process inbox.
        let session_id = teams
            .and_then(|t| t.lead_session_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let mut env = vec![
            ("CAS_AGENT_NAME".to_string(), name.to_string()),
            ("CAS_AGENT_ROLE".to_string(), role.to_string()),
            // Mark this process as running inside a factory session.
            // Read by pre_tool jail, close_ops, mcp server, and task update
            // to branch factory-vs-standalone behavior. Without this, the
            // is_factory_worker check in pre_tool.rs fails (it requires both
            // CAS_AGENT_ROLE=worker AND CAS_FACTORY_MODE), so workers get
            // jailed on every verification-pending task.
            ("CAS_FACTORY_MODE".to_string(), "1".to_string()),
            // Provide session ID so CAS MCP server can self-register without hooks
            ("CAS_SESSION_ID".to_string(), session_id.clone()),
            // Set clone path so subagents know the worktree directory
            (
                "CAS_CLONE_PATH".to_string(),
                cwd.to_string_lossy().to_string(),
            ),
            // Suppress interactive prompts, telemetry, and updates for factory agents
            (
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_string(),
                "1".to_string(),
            ),
            ("DISABLE_AUTOUPDATER".to_string(), "1".to_string()),
            ("DISABLE_COST_WARNINGS".to_string(), "1".to_string()),
            (
                "CLAUDE_CODE_DISABLE_TERMINAL_TITLE".to_string(),
                "1".to_string(),
            ),
            ("IS_DEMO".to_string(), "true".to_string()),
        ];

        // Set CAS_ROOT env var if provided (enables workers in clones to use main's .cas)
        if let Some(root) = cas_root {
            env.push(("CAS_ROOT".to_string(), root.to_string_lossy().to_string()));
        }

        // Set supervisor name for workers (enables `target: supervisor` in message action)
        if let Some(sup) = supervisor_name {
            env.push(("CAS_SUPERVISOR_NAME".to_string(), sup.to_string()));
        }
        if let Some(worker_cli) = factory_worker_cli {
            env.push(("CAS_FACTORY_WORKER_CLI".to_string(), worker_cli.to_string()));
        }

        // cas-0bf4: cap cargo parallelism inside factory worker processes
        // so a 4-worker factory doesn't stack `num_cpus`-way rustc jobs
        // per worker and wedge the host via scheduler starvation
        // (cas-4513 Claude Code JS crash-screen symptom). Emitted only
        // for role="worker"; supervisor stays uncapped.
        push_worker_cargo_env(&mut env, role);
        // Point factory workers at the repo's bootstrapped Zig toolchain so the
        // ghostty_vt_sys build script finds Zig on the first `cargo build` in a
        // fresh worker worktree instead of failing and forcing a manual
        // bootstrap-zig.sh + ZIG export dance (observed in the cas-3522 shakedown).
        push_worker_zig_env(&mut env, role, cas_root);

        // Enable native Agent Teams for inter-agent messaging
        if teams.is_some() {
            env.push((
                "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string(),
                "1".to_string(),
            ));
        }

        let mut args = vec![
            "--dangerously-skip-permissions".to_string(),
            "--session-id".to_string(),
            session_id,
        ];
        if let Some(m) = model {
            args.push("--model".to_string());
            args.push(m.to_string());
        }
        // Supervisors need deeper reasoning for planning/coordination;
        // workers execute well-defined tasks where high effort suffices.
        // Config-provided effort takes precedence; role-based defaults preserve
        // backward compatibility when no config value is set.
        let resolved_effort =
            effort.unwrap_or(if role == "supervisor" { "xhigh" } else { "high" });
        args.push("--effort".to_string());
        args.push(resolved_effort.to_string());

        // Add native Agent Teams CLI flags.
        // All agents (including the supervisor) get --teammate-mode tmux
        // so Claude Code activates inbox polling for everyone.
        if let Some(t) = teams {
            args.push("--team-name".to_string());
            args.push(t.team_name.clone());
            args.push("--agent-id".to_string());
            args.push(t.agent_id.clone());
            args.push("--agent-name".to_string());
            args.push(t.agent_name.clone());
            args.push("--agent-color".to_string());
            args.push(t.agent_color.clone());
            args.push("--agent-type".to_string());
            args.push(t.agent_type.clone());
            args.push("--teammate-mode".to_string());
            args.push("tmux".to_string());
            if let Some(ref parent_id) = t.parent_session_id {
                args.push("--parent-session-id".to_string());
                args.push(parent_id.clone());
            }
            // Per-role settings file — both supervisor and workers ship a
            // `permissions.allow` list via `--settings` so Read/Write/Edit/
            // Glob/Grep/Bash/NotebookEdit auto-approve instead of escalating
            // through team-approval routing (the phantom `team-lead` hang).
            // If the caller leaves `settings_path` as None (CLI usage,
            // standalone claude invocations, or tests that deliberately
            // opt out), no flag is emitted — that's a valid fallback.
            if let Some(ref settings_path) = t.settings_path {
                args.push("--settings".to_string());
                args.push(settings_path.clone());
            }
        }

        // cas-0bf4: optionally lower the worker's scheduling priority so
        // the supervisor's Claude Code event loop wins scheduler fights.
        // Only fires for role="worker" when `CAS_FACTORY_NICE_WORKER=1`
        // is set by the supervisor-side factory config bridge.
        let (command, args) = maybe_wrap_with_nice("claude", args, role);

        Self {
            command,
            args,
            cwd: Some(cwd),
            env,
            rows: 24,
            cols: 80,
        }
    }

    /// Create config for a Codex CLI instance
    ///
    /// # Arguments
    /// * `name` - Agent name
    /// * `role` - Agent role (e.g., "worker", "supervisor")
    /// * `cwd` - Working directory for the agent
    /// * `cas_root` - Optional path to the .cas directory. If provided, sets CAS_ROOT env var
    /// * `supervisor_name` - For workers, the name of their supervisor (enables `target: supervisor`)
    #[allow(clippy::too_many_arguments)]
    pub fn codex(
        name: &str,
        role: &str,
        cwd: PathBuf,
        cas_root: Option<&PathBuf>,
        supervisor_name: Option<&str>,
        factory_worker_cli: Option<&str>,
        model: Option<&str>,
        effort: Option<&str>,
        _teams: Option<&TeamsSpawnConfig>,
    ) -> Self {
        // Native Agent Teams is Claude Code-only; Codex CLI does not support it.
        // Keep the human-readable `codex-<name>-` prefix (operator clarity in
        // worker_status/agent_list); nothing parses it, so the suffix is a uuid.
        let session_id = format!("codex-{name}-{}", uuid::Uuid::new_v4());

        let mut env = vec![
            ("CAS_AGENT_NAME".to_string(), name.to_string()),
            ("CAS_AGENT_ROLE".to_string(), role.to_string()),
            // Mark this process as running inside a factory session.
            // See equivalent comment in `claude()` above — without this the
            // pre_tool verification-jail exemption for factory workers does
            // not fire and workers get jailed on every pending task.
            ("CAS_FACTORY_MODE".to_string(), "1".to_string()),
            // Provide session ID so CAS MCP server can self-register without hooks
            ("CAS_SESSION_ID".to_string(), session_id.clone()),
            (
                "CAS_CLONE_PATH".to_string(),
                cwd.to_string_lossy().to_string(),
            ),
            // Suppress interactive prompts, telemetry, and updates for factory agents
            (
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".to_string(),
                "1".to_string(),
            ),
            ("DISABLE_AUTOUPDATER".to_string(), "1".to_string()),
            ("DISABLE_COST_WARNINGS".to_string(), "1".to_string()),
            (
                "CLAUDE_CODE_DISABLE_TERMINAL_TITLE".to_string(),
                "1".to_string(),
            ),
            ("IS_DEMO".to_string(), "true".to_string()),
        ];

        if let Ok(term) = std::env::var("TERM")
            && term.contains("ghostty")
        {
            env.push(("TERM".to_string(), "xterm-256color".to_string()));
        }

        if let Some(root) = cas_root {
            env.push(("CAS_ROOT".to_string(), root.to_string_lossy().to_string()));
        }

        if let Some(sup) = supervisor_name {
            env.push(("CAS_SUPERVISOR_NAME".to_string(), sup.to_string()));
        }
        if let Some(worker_cli) = factory_worker_cli {
            env.push(("CAS_FACTORY_WORKER_CLI".to_string(), worker_cli.to_string()));
        }

        // cas-0bf4: see equivalent comment in `claude()`.
        push_worker_cargo_env(&mut env, role);
        // cas-3522 follow-on: see equivalent comment in `claude()`.
        push_worker_zig_env(&mut env, role, cas_root);

        let mut args = vec!["--yolo".to_string(), "--no-alt-screen".to_string()];

        // cas-bbc2: spawn-inject the CAS MCP server so every Codex agent (worker
        // and supervisor) has mcp__cs__* tools even when the project was never
        // integrated for the Codex harness (no .codex/config.toml). Must precede
        // the developer_instructions block but order among `-c` flags is
        // irrelevant to Codex.
        push_codex_mcp_server_args(&mut args, &session_id);

        if let Some(m) = model {
            args.push("--model".to_string());
            args.push(m.to_string());
        }
        // Codex CLI 0.128.0 has no --effort flag; effort is set via -c TOML override.
        // Valid values: none, minimal, low, medium, high, xhigh (same vocabulary as Claude).
        // Unlike claude(), we do NOT apply a role-based default when effort is None — Codex
        // CLI's built-in server-side default is acceptable and avoids hard-coding a TOML
        // override that would need revisiting each Codex release.
        if let Some(e) = effort {
            args.push("-c".to_string());
            args.push(format!("model_reasoning_effort={e}"));
        }

        if role == "supervisor" {
            let escaped = CODEX_SUPERVISOR_INSTRUCTIONS.replace('"', "\\\"");
            args.push("--config".to_string());
            args.push(format!("developer_instructions=\"{escaped}\""));
        } else if role == "worker" {
            let escaped = CODEX_WORKER_INSTRUCTIONS.replace('"', "\\\"");
            args.push("--config".to_string());
            args.push(format!("developer_instructions=\"{escaped}\""));

            // Pass startup workflow as initial prompt arg so Codex executes it immediately.
            // This is more reliable than post-spawn typed injection, which can leave text
            // in the composer without submitting in some startup timing windows.
            let startup_prompt = format!(
                "{CODEX_WORKER_STARTUP_PREFIX}\n\
                 2) Run mcp__cs__task action=mine\n\
                 3) If a task is assigned: choose exactly ONE task, run mcp__cs__task action=show then action=start, add a progress note, implement it, commit and push, then close it (or hand to supervisor) BEFORE starting any other task. Never start more than one task at a time — batch-starting collides with the one-unverified-in-progress verification jail.\n\
                 4) If no tasks are assigned: send mcp__cs__coordination action=message target=supervisor confirming ready state\n\
                 5) Do NOT message target=cas. Use target=supervisor."
            );
            args.push(startup_prompt);
        }

        // cas-0bf4: see equivalent comment in `claude()`.
        let (command, args) = maybe_wrap_with_nice("codex", args, role);

        Self {
            command,
            args,
            cwd: Some(cwd),
            env,
            rows: 24,
            cols: 80,
        }
    }
}

/// Expected number of concurrent factory workers the CPU is being
/// shared among when auto-computing `CARGO_BUILD_JOBS`. On a 16-thread
/// dev box (soundwave, reference host for cas-4513 + cas-0bf4 evidence)
/// this divides the CPU budget into 4 × 4-thread slices, which kept the
/// host below scheduler saturation in the sessions where we observed
/// the Claude Code JS crash-screen wedges.
///
/// Override the assumption by setting `CAS_FACTORY_CARGO_BUILD_JOBS`
/// explicitly — e.g., a supervisor running 8 workers on a 16-thread
/// host should export `CAS_FACTORY_CARGO_BUILD_JOBS=2`.
const DEFAULT_WORKER_CONCURRENCY_ASSUMPTION: usize = 4;

/// Compute the `CARGO_BUILD_JOBS` value to export into a worker's env.
///
/// Precedence (first match wins):
///   1. `CAS_FACTORY_CARGO_BUILD_JOBS` env — set by the supervisor-side
///      factory config bridge from `factory.cargo_build_jobs` config.
///      Empty value or literal `"auto"` means "fall through to 2–4".
///   2. Auto-compute: `max(2, available_parallelism() / DEFAULT_WORKER_CONCURRENCY_ASSUMPTION)`.
///
/// Returns `None` only when auto-compute fails to read CPU topology,
/// which should be vanishingly rare. In that case we do NOT set
/// `CARGO_BUILD_JOBS` — cargo's own default (= num_cpus) then applies
/// and the cap is a no-op rather than misleading.
fn cargo_build_jobs_for_worker() -> Option<String> {
    if let Ok(explicit) = std::env::var("CAS_FACTORY_CARGO_BUILD_JOBS") {
        let trimmed = explicit.trim();
        // Case-insensitive `"auto"` falls through to the computed cap so
        // users who write `Auto`/`AUTO` in config don't silently defeat
        // the mitigation by shipping a literal non-integer value into
        // `CARGO_BUILD_JOBS`.
        if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("auto") {
            return Some(trimmed.to_string());
        }
    }
    let cores = std::thread::available_parallelism().ok()?.get();
    let capped = std::cmp::max(2, cores / DEFAULT_WORKER_CONCURRENCY_ASSUMPTION);
    Some(capped.to_string())
}

/// Spawn-inject the CAS MCP server into a Codex command via `-c` overrides
/// (cas-bbc2). Codex does NOT read Claude's `.mcp.json`; it only discovers the
/// CAS server from a project `.codex/config.toml` written by `cas init`/`cas
/// update` (`configure_codex_mcp_server`). Projects integrated for Claude but
/// never for Codex (e.g. gabber-studio: has `.mcp.json`, no `.codex/`) therefore
/// spawned Codex agents with **zero** `mcp__cs__*` tools, which burned the whole
/// session reverse-engineering the wire protocol and produced no code.
///
/// Injecting the server at spawn time makes every Codex agent (worker and
/// supervisor) self-contained regardless of downstream integration. We mirror
/// `configure_codex_mcp_server` exactly — `command="cas"`, `args=["serve"]`,
/// `env.CAS_CODEX_FALLBACK_SESSION="1"` — and register under the `cs` key so the
/// resulting tool prefix is `mcp__cs__` (the Codex alias used throughout the
/// factory prompts and skills; intentionally distinct from Claude's `mcp__cas__`).
///
/// Each value is valid TOML so Codex's `-c key=value` parser (value parsed as
/// TOML, raw-string fallback) yields the intended types: quoted strings stay
/// strings, `["serve"]` becomes a string array. If a project DOES ship a
/// `.codex/config.toml`, these `-c` overrides simply add the `cs` server on top
/// — they never remove the project's own entries.
fn push_codex_mcp_server_args(args: &mut Vec<String>, session_id: &str) {
    args.push("-c".to_string());
    args.push("mcp_servers.cs.command=\"cas\"".to_string());
    args.push("-c".to_string());
    args.push("mcp_servers.cs.args=[\"serve\"]".to_string());
    args.push("-c".to_string());
    args.push("mcp_servers.cs.env.CAS_CODEX_FALLBACK_SESSION=\"1\"".to_string());
    // cas-3522: inject the canonical session id into the `cs` MCP server env so
    // `get_agent_id()` auto-registers the agent on its FIRST tool call — the same
    // env fast-path Claude workers rely on. Codex starts MCP servers with a
    // restricted env (it does not pass the codex process env through), so without
    // this the `cs` server comes up identity-less and whoami/task fail with
    // "Agent not registered" until the worker brute-forces a manual `register`.
    args.push("-c".to_string());
    args.push(format!("mcp_servers.cs.env.CAS_SESSION_ID=\"{session_id}\""));
}

/// Returns `true` when an executable named `cas` is resolvable on `PATH`.
///
/// Used by the Codex spawn preflight (cas-bbc2). The CAS MCP server is now
/// spawn-injected as `mcp_servers.cs.command=cas`, but Codex still needs the
/// `cas` binary on `PATH` to actually launch it. If `PATH` is unset we return
/// `true` (skip the check) rather than risk a false refusal — the spawn will
/// surface its own error in that pathological case.
fn cas_binary_on_path() -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return true;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join("cas");
        candidate.is_file()
    })
}

/// Push the `CARGO_BUILD_JOBS` env entry into `env` when `role == "worker"`.
/// Extracted from `PtyConfig::{claude,codex}` to remove the duplicated
/// block those two call sites used to carry verbatim (cas-0bf4).
fn push_worker_cargo_env(env: &mut Vec<(String, String)>, role: &str) {
    if role != "worker" {
        return;
    }
    if let Some(cargo_jobs) = cargo_build_jobs_for_worker() {
        env.push(("CARGO_BUILD_JOBS".to_string(), cargo_jobs));
    }
}

/// Export `ZIG` into a worker's env pointing at the repo's bootstrapped Zig
/// binary (`<repo>/.context/zig/zig`), so the `ghostty_vt_sys` build script can
/// find Zig on the first `cargo build` inside a fresh worker worktree.
///
/// Without this, a worker's first build fails in `ghostty_vt_sys` ("could not
/// find Zig") and the worker has to discover + run `scripts/bootstrap-zig.sh`
/// and export `ZIG` by hand before it can compile anything — wasted turns
/// observed in the cas-3522 Codex shakedown.
///
/// Worker-only and best-effort, mirroring `push_worker_cargo_env`:
/// - `cas_root` is `<repo>/.cas`, so the repo root is its parent.
/// - We only set `ZIG` when the binary actually exists at the expected path;
///   pointing `ZIG` at a missing file would break builds worse than leaving it
///   unset (the build script would still try to bootstrap). The path is
///   absolute, so it resolves correctly from any worktree cwd.
fn push_worker_zig_env(env: &mut Vec<(String, String)>, role: &str, cas_root: Option<&PathBuf>) {
    if role != "worker" {
        return;
    }
    let Some(repo) = cas_root.and_then(|r| r.parent()) else {
        return;
    };
    let zig = repo.join(".context").join("zig").join("zig");
    if zig.is_file() {
        env.push(("ZIG".to_string(), zig.to_string_lossy().to_string()));
    }
}

/// If `CAS_FACTORY_NICE_WORKER=1` is set in the supervisor's env and
/// `role == "worker"`, wrap the spawn command in `nice -n 10` so the
/// worker's process tree (including cargo-driven rustc jobs) runs at
/// a lower scheduling priority than the supervisor. Supervisor panes
/// stay at nice 0 and therefore win CPU-contention fights, which keeps
/// the factory steerable when workers start cargo-storming (cas-0bf4).
///
/// Non-worker roles and sessions without the sentinel env are passed
/// through unchanged. `nice` must be on PATH (standard on every Linux
/// and macOS host CAS supports); if it isn't, the worker will fail to
/// spawn with a clear "nice not found" error from the PTY layer rather
/// than silently running unwrapped — that's the safer fallback.
fn maybe_wrap_with_nice(command: &str, args: Vec<String>, role: &str) -> (String, Vec<String>) {
    if role != "worker" {
        return (command.to_string(), args);
    }
    if std::env::var("CAS_FACTORY_NICE_WORKER").as_deref() != Ok("1") {
        return (command.to_string(), args);
    }
    // Default niceness increment is 10; honour CAS_FACTORY_NICE_LEVEL
    // for power users who want a harder or softer cap. Parse as i32 so
    // a typo like `CAS_FACTORY_NICE_LEVEL=high` cannot propagate to
    // `nice -n high claude ...` and kill every worker spawn with an
    // opaque PTY error — we quietly fall back to the default 10.
    let level = std::env::var("CAS_FACTORY_NICE_LEVEL")
        .ok()
        .and_then(|s| s.trim().parse::<i32>().ok())
        .map(|n| n.to_string())
        .unwrap_or_else(|| "10".to_string());
    let mut new_args = Vec::with_capacity(args.len() + 3);
    new_args.push("-n".to_string());
    new_args.push(level);
    new_args.push(command.to_string());
    new_args.extend(args);
    ("nice".to_string(), new_args)
}

/// Events emitted by a PTY
#[derive(Debug, Clone)]
pub enum PtyEvent {
    /// Terminal output (raw bytes - parsing done by ghostty_vt)
    Output(Vec<u8>),
    /// Process exited
    Exited(Option<i32>),
    /// Error occurred
    Error(String),
}

/// A running PTY process
pub struct Pty {
    /// Unique identifier
    id: String,
    /// Writer handle for sending input
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Channel for receiving raw output
    event_rx: mpsc::Receiver<PtyEvent>,
    /// Handle to the reader task
    _reader_handle: std::thread::JoinHandle<()>,
    /// Child process handle
    child: Box<dyn portable_pty::Child + Send + Sync>,
    /// Master PTY (keep alive)
    master: Box<dyn portable_pty::MasterPty + Send>,
    /// Whether this PTY is running Codex CLI
    is_codex: bool,
}

impl Pty {
    /// Spawn a new PTY with the given configuration
    pub fn spawn(id: impl Into<String>, config: PtyConfig) -> Result<Self> {
        let id = id.into();
        let is_codex = config.command == "codex";

        // cas-bbc2 preflight: a Codex agent's CAS MCP server is spawn-injected as
        // `mcp_servers.cs.command=cas`, but Codex can only launch it if the `cas`
        // binary is resolvable on PATH. Detect Codex by the direct command or the
        // niced wrapper form (cas-0bf4) so the preflight covers both. Refuse loudly
        // with remediation rather than spawning a worker that comes up with zero
        // CAS tools and flails.
        let codex_spawn = is_codex
            || (config.command == "nice" && config.args.iter().any(|a| a == "codex"));
        if codex_spawn && !cas_binary_on_path() {
            return Err(Error::pty(
                "Codex agent cannot start: the `cas` MCP server binary is not on PATH. \
                 CAS is spawn-injected as mcp_servers.cs (command=cas), but Codex needs \
                 `cas` resolvable to launch it. Install CAS / add it to PATH, or run \
                 `cas init` / `cas update` in this project to enable the Codex harness."
                    .to_string(),
            ));
        }

        // Create PTY system and open a PTY pair
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::pty(format!("Failed to open PTY: {e}")))?;

        // Build command
        let mut cmd = CommandBuilder::new(&config.command);
        cmd.args(&config.args);

        if let Some(cwd) = &config.cwd {
            cmd.cwd(cwd);
            // STEP 1 (cas-5232): Log the actual cwd being set on the PTY command so
            // the daemon trace carries an auditable record of where each worker process
            // will land.  This runs on the main thread immediately before spawn, so the
            // log timestamp is tightly coupled to the PTY launch.
            tracing::info!(
                command = %config.command,
                cwd = %cwd.display(),
                "pty: spawning process with explicit cwd"
            );
        }

        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        // Strip CLAUDECODE to prevent nested-session detection in spawned Claude CLI
        cmd.env_remove("CLAUDECODE");

        // Spawn the child process
        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| Error::pty(format!("Failed to spawn command: {e}")))?;

        // Drop slave - the child process owns it now
        drop(pair.slave);

        // Get reader and writer
        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| Error::pty(format!("Failed to clone reader: {e}")))?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|e| Error::pty(format!("Failed to get writer: {e}")))?;

        let writer = Arc::new(Mutex::new(writer));

        if is_codex {
            let writer = Arc::clone(&writer);
            tokio::spawn(async move {
                for _ in 0..10 {
                    let mut locked = writer.lock().await;
                    let _ = locked.write_all(b"\x1b[1;1R");
                    let _ = locked.flush();
                    drop(locked);
                    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                }
            });
        }

        // Create channel for events - larger buffer for multi-agent scenarios
        let (event_tx, event_rx) = mpsc::channel::<PtyEvent>(1024);

        // Spawn reader thread - sends raw bytes, no parsing
        let reader_handle = std::thread::spawn({
            let writer = Arc::clone(&writer);
            move || {
                Self::reader_loop(reader, writer, event_tx);
            }
        });

        Ok(Self {
            id,
            writer,
            event_rx,
            _reader_handle: reader_handle,
            child,
            master: pair.master,
            is_codex,
        })
    }

    /// Reader loop that forwards raw PTY output
    fn reader_loop(
        mut reader: Box<dyn Read + Send>,
        writer: Arc<Mutex<Box<dyn Write + Send>>>,
        event_tx: mpsc::Sender<PtyEvent>,
    ) {
        // Larger buffer for high-throughput scenarios (6 Claudes generating long responses)
        let mut buf = [0u8; 16384];
        let mut carry: Vec<u8> = Vec::new();

        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // EOF - process exited
                    if !carry.is_empty() {
                        let _ =
                            event_tx.blocking_send(PtyEvent::Output(std::mem::take(&mut carry)));
                    }
                    let _ = event_tx.blocking_send(PtyEvent::Exited(None));
                    break;
                }
                Ok(n) => {
                    let (data, new_carry, saw_cpr) =
                        filter_cursor_position_requests(&carry, &buf[..n]);
                    carry = new_carry;

                    if saw_cpr {
                        let mut locked = writer.blocking_lock();
                        let _ = locked.write_all(b"\x1b[1;1R");
                        let _ = locked.flush();
                    }

                    if !data.is_empty() && event_tx.blocking_send(PtyEvent::Output(data)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    let _ = event_tx.blocking_send(PtyEvent::Error(e.to_string()));
                    break;
                }
            }
        }
    }

    /// Get the PTY's identifier
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Returns true when this PTY is running Codex CLI.
    pub fn is_codex(&self) -> bool {
        self.is_codex
    }

    /// Get a clone of the writer handle (for concurrent writing)
    pub fn writer_handle(&self) -> Arc<Mutex<Box<dyn Write + Send>>> {
        self.writer.clone()
    }

    /// Write input to the PTY (for prompt injection)
    pub async fn write(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().await;
        writer
            .write_all(data)
            .map_err(|e| Error::pty(format!("Write failed: {e}")))?;
        writer
            .flush()
            .map_err(|e| Error::pty(format!("Flush failed: {e}")))?;
        Ok(())
    }

    /// Write a string to the PTY
    pub async fn write_str(&self, s: &str) -> Result<()> {
        self.write(s.as_bytes()).await
    }

    /// Send a line of input (appends carriage return to submit, same as Enter key)
    pub async fn send_line(&self, line: &str) -> Result<()> {
        self.write_str(&format!("{line}\r")).await
    }

    /// Receive the next event from the PTY (blocking)
    pub async fn recv(&mut self) -> Option<PtyEvent> {
        self.event_rx.recv().await
    }

    /// Try to receive an event from the PTY (non-blocking)
    pub fn try_recv(&mut self) -> Option<PtyEvent> {
        self.event_rx.try_recv().ok()
    }

    /// Resize the PTY
    pub fn resize(&self, rows: u16, cols: u16) -> Result<()> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| Error::pty(format!("Resize failed: {e}")))
    }

    /// Send Ctrl+C to the process
    pub async fn interrupt(&self) -> Result<()> {
        self.write(&[0x03]).await
    }

    /// Send Ctrl+D (EOF) to the process
    pub async fn send_eof(&self) -> Result<()> {
        self.write(&[0x04]).await
    }

    /// Kill the child process
    pub fn kill(&mut self) {
        let _ = self.child.kill();
    }
}

fn filter_cursor_position_requests(carry: &[u8], chunk: &[u8]) -> (Vec<u8>, Vec<u8>, bool) {
    const CPR: [u8; 4] = [0x1b, 0x5b, 0x36, 0x6e]; // ESC [ 6 n
    const CPR_ALT: [u8; 5] = [0x1b, 0x5b, 0x3f, 0x36, 0x6e]; // ESC [ ? 6 n
    let max_seq = CPR_ALT.len();

    let total_len = carry.len() + chunk.len();
    if total_len == 0 {
        return (Vec::new(), Vec::new(), false);
    }

    let process_len = total_len.saturating_sub(max_seq - 1);
    let mut out = Vec::with_capacity(process_len);
    let mut i = 0usize;
    let mut saw_cpr = false;

    let byte_at = |idx: usize| -> u8 {
        if idx < carry.len() {
            carry[idx]
        } else {
            chunk[idx - carry.len()]
        }
    };

    while i < process_len {
        if i + CPR_ALT.len() <= total_len {
            let mut matches = true;
            for (j, byte) in CPR_ALT.iter().enumerate() {
                if byte_at(i + j) != *byte {
                    matches = false;
                    break;
                }
            }
            if matches {
                saw_cpr = true;
                i += CPR_ALT.len();
                continue;
            }
        }
        if i + CPR.len() <= total_len {
            let mut matches = true;
            for (j, byte) in CPR.iter().enumerate() {
                if byte_at(i + j) != *byte {
                    matches = false;
                    break;
                }
            }
            if matches {
                saw_cpr = true;
                i += CPR.len();
                continue;
            }
        }
        out.push(byte_at(i));
        i += 1;
    }

    let mut new_carry = Vec::with_capacity(total_len - process_len);
    for idx in process_len..total_len {
        new_carry.push(byte_at(idx));
    }

    (out, new_carry, saw_cpr)
}

#[cfg(test)]
mod tests {
    use crate::pty::*;
    use std::sync::{Mutex, MutexGuard};

    // cas-0bf4: module-wide serialization for any test that constructs a
    // `PtyConfig::{claude,codex}` with role="worker". Those constructors
    // now read process-wide env vars (CAS_FACTORY_CARGO_BUILD_JOBS and
    // CAS_FACTORY_NICE_WORKER) at call time; parallel tests can race if
    // one sets the sentinel while another asserts on the non-wrapped
    // command name. All worker-role PtyConfig tests must hold this
    // mutex for the duration of their body.
    pub(crate) static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Lock the env mutex, clear the cas-0bf4 sentinels on entry, and
    /// clear them again on drop. Safe to use from any test that may
    /// observe or mutate those vars.
    pub(crate) struct ScopedEnv {
        _guard: MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        pub(crate) fn new() -> Self {
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: mutex serializes env mutation across tests.
            unsafe {
                std::env::remove_var("CAS_FACTORY_CARGO_BUILD_JOBS");
                std::env::remove_var("CAS_FACTORY_NICE_WORKER");
                std::env::remove_var("CAS_FACTORY_NICE_LEVEL");
            }
            Self { _guard: guard }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            // SAFETY: mutex held for duration of this scope.
            unsafe {
                std::env::remove_var("CAS_FACTORY_CARGO_BUILD_JOBS");
                std::env::remove_var("CAS_FACTORY_NICE_WORKER");
                std::env::remove_var("CAS_FACTORY_NICE_LEVEL");
            }
        }
    }

    #[tokio::test]
    async fn test_pty_config_default() {
        let config = PtyConfig::default();
        assert_eq!(config.command, "bash");
        assert_eq!(config.rows, 24);
        assert_eq!(config.cols, 80);
    }

    #[tokio::test]
    async fn test_pty_config_claude() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::claude(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        assert_eq!(config.command, "claude");
        assert!(
            config
                .args
                .contains(&"--dangerously-skip-permissions".to_string())
        );
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_AGENT_NAME" && v == "test-agent")
        );
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_AGENT_ROLE" && v == "worker")
        );
        // No CAS_ROOT when not provided
        assert!(!config.env.iter().any(|(k, _)| k == "CAS_ROOT"));
    }

    #[tokio::test]
    async fn test_pty_config_claude_with_cas_root() {
        let cas_root = PathBuf::from("/home/user/project/.cas");
        let config = PtyConfig::claude(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            Some(&cas_root),
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_ROOT" && v == "/home/user/project/.cas")
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_with_supervisor() {
        let config = PtyConfig::claude(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            Some("test-supervisor"),
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_SUPERVISOR_NAME" && v == "test-supervisor")
        );
    }

    #[tokio::test]
    async fn test_pty_config_sets_clone_path() {
        let config = PtyConfig::claude(
            "test-worker",
            "worker",
            PathBuf::from("/tmp/worktree"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_CLONE_PATH" && v == "/tmp/worktree")
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_with_model() {
        let config = PtyConfig::claude(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            Some("claude-opus-4-6"),
            None,  // effort
            None,  // teams
        );
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"claude-opus-4-6".to_string()));
    }

    #[tokio::test]
    async fn test_pty_config_claude_without_model() {
        let config = PtyConfig::claude(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        assert!(!config.args.contains(&"--model".to_string()));
    }

    #[tokio::test]
    async fn test_pty_config_codex_with_model() {
        let config = PtyConfig::codex(
            "test-agent",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            Some("gpt-5.3-codex"),
            None,  // effort
            None,  // teams
        );
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"gpt-5.3-codex".to_string()));
    }

    #[tokio::test]
    async fn test_pty_config_codex_worker_uses_cs_prefix() {
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("mcp__cs__"),
            "Codex worker instructions should use mcp__cs__ prefix"
        );
    }

    /// cas-bbc2 AC#2: a Codex worker spawn must inject the CAS MCP server via
    /// `-c` overrides so it has `mcp__cs__*` tools without a project
    /// `.codex/config.toml`. Mirrors `configure_codex_mcp_server`.
    #[tokio::test]
    async fn test_pty_config_codex_worker_injects_cas_mcp_server() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("mcp_servers.cs.command=\"cas\""),
            "codex worker must inject mcp_servers.cs.command=cas; got: {all_args}"
        );
        assert!(
            all_args.contains("mcp_servers.cs.args=[\"serve\"]"),
            "codex worker must inject mcp_servers.cs.args=[\"serve\"]; got: {all_args}"
        );
        assert!(
            all_args.contains("mcp_servers.cs.env.CAS_CODEX_FALLBACK_SESSION=\"1\""),
            "codex worker must inject CAS_CODEX_FALLBACK_SESSION env; got: {all_args}"
        );
    }

    /// cas-bbc2: the supervisor is equally self-contained — a Codex supervisor
    /// must also get the spawn-injected CAS MCP server.
    #[tokio::test]
    async fn test_pty_config_codex_supervisor_injects_cas_mcp_server() {
        let config = PtyConfig::codex(
            "test-supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("mcp_servers.cs.command=\"cas\"")
                && all_args.contains("mcp_servers.cs.args=[\"serve\"]")
                && all_args.contains("mcp_servers.cs.env.CAS_CODEX_FALLBACK_SESSION=\"1\""),
            "codex supervisor must inject the cas MCP server; got: {all_args}"
        );
    }

    /// cas-bbc2 AC#3: the Codex worker startup prompt must drive a single-task
    /// loop (start exactly one task at a time), not the old batch-start wording
    /// that collides with the one-unverified-in-progress verification jail.
    #[tokio::test]
    async fn test_pty_config_codex_worker_single_task_loop() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("exactly ONE task"),
            "startup prompt must instruct starting exactly one task at a time; got: {all_args}"
        );
        assert!(
            !all_args.contains("show/start each task"),
            "old batch-start wording must be gone; got: {all_args}"
        );
        // The developer_instructions must carry the same discipline.
        assert!(
            all_args.contains("Work exactly ONE task at a time"),
            "worker developer_instructions must enforce one-task-at-a-time; got: {all_args}"
        );
    }

    /// cas-3522: the Codex `cs` MCP server must receive the canonical session id
    /// so `get_agent_id()` auto-registers the agent on the first tool call.
    /// Without it the worker burns ~6 failed calls ("Agent not registered")
    /// before brute-forcing a manual register.
    #[tokio::test]
    async fn test_pty_config_codex_worker_injects_session_id() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("mcp_servers.cs.env.CAS_SESSION_ID=\"codex-test-worker-"),
            "codex worker must inject CAS_SESSION_ID into the cs MCP env; got: {all_args}"
        );
        // The same id must be exported into the process env (they must match).
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CAS_SESSION_ID" && v.starts_with("codex-test-worker-")),
            "codex worker process env must carry the matching CAS_SESSION_ID; got: {:?}",
            config.env
        );
    }

    /// cas-3522: the supervisor's cs MCP server also needs CAS_SESSION_ID.
    #[tokio::test]
    async fn test_pty_config_codex_supervisor_injects_session_id() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("mcp_servers.cs.env.CAS_SESSION_ID=\"codex-test-supervisor-"),
            "codex supervisor must inject CAS_SESSION_ID into the cs MCP env; got: {all_args}"
        );
    }

    /// cas-3522: the startup prompt and worker instructions must NOT drive a
    /// `session_start` invocation anymore — auto-registration replaces it.
    #[tokio::test]
    async fn test_pty_config_codex_worker_no_session_start_invocation() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        let all_args = config.args.join(" ");
        assert!(
            !all_args.contains("action=session_start"),
            "codex worker must no longer invoke session_start; got: {all_args}"
        );
        // whoami remains the first explicit identity check.
        assert!(
            all_args.contains("action=whoami"),
            "codex worker startup should still confirm identity via whoami; got: {all_args}"
        );
    }

    /// cas-3522 follow-on: a worker gets `ZIG` pointed at the repo's
    /// bootstrapped binary when it exists; non-workers and missing-binary cases
    /// leave `ZIG` unset.
    #[tokio::test]
    async fn test_push_worker_zig_env_sets_zig_for_worker_when_present() {
        let dir = std::env::temp_dir().join("cas-3522-zig-env-test");
        let _ = std::fs::remove_dir_all(&dir);
        let zig = dir.join(".context").join("zig").join("zig");
        std::fs::create_dir_all(zig.parent().unwrap()).unwrap();
        std::fs::write(&zig, b"#!/bin/sh\n").unwrap();
        let cas_root = dir.join(".cas");
        std::fs::create_dir_all(&cas_root).unwrap();

        let zig_str = zig.to_string_lossy().to_string();

        let mut worker_env: Vec<(String, String)> = Vec::new();
        push_worker_zig_env(&mut worker_env, "worker", Some(&cas_root));
        assert!(
            worker_env.iter().any(|(k, v)| k == "ZIG" && v == &zig_str),
            "worker must get ZIG pointing at the bootstrapped binary; got: {worker_env:?}"
        );

        let mut sup_env: Vec<(String, String)> = Vec::new();
        push_worker_zig_env(&mut sup_env, "supervisor", Some(&cas_root));
        assert!(
            !sup_env.iter().any(|(k, _)| k == "ZIG"),
            "supervisor must NOT get ZIG; got: {sup_env:?}"
        );

        // Missing binary -> no ZIG even for a worker.
        let empty_root = dir.join("empty").join(".cas");
        std::fs::create_dir_all(&empty_root).unwrap();
        let mut missing_env: Vec<(String, String)> = Vec::new();
        push_worker_zig_env(&mut missing_env, "worker", Some(&empty_root));
        assert!(
            !missing_env.iter().any(|(k, _)| k == "ZIG"),
            "worker must not get ZIG when the binary is absent; got: {missing_env:?}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// cas-bbc2: `cas_binary_on_path()` returns true when an executable named
    /// `cas` lives in a PATH entry. Builds a temp dir with a fake `cas` file and
    /// points PATH at it, under ENV_LOCK to avoid racing other env-mutating tests.
    #[tokio::test]
    async fn test_cas_binary_on_path_detects_binary() {
        let _e = ScopedEnv::new();
        let dir = std::env::temp_dir().join("cas-bbc2-preflight-present");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cas"), b"#!/bin/sh\n").unwrap();
        let saved = std::env::var_os("PATH");
        // SAFETY: ENV_LOCK held via ScopedEnv serializes PATH mutation.
        unsafe {
            std::env::set_var("PATH", &dir);
        }
        let found = cas_binary_on_path();
        unsafe {
            match &saved {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(found, "cas_binary_on_path must find a `cas` file on PATH");
    }

    /// cas-bbc2: when no `cas` exists on PATH, the preflight helper reports false
    /// so `Pty::spawn` can refuse a Codex spawn loudly.
    #[tokio::test]
    async fn test_cas_binary_on_path_absent_when_missing() {
        let _e = ScopedEnv::new();
        let dir = std::env::temp_dir().join("cas-bbc2-preflight-absent");
        std::fs::create_dir_all(&dir).unwrap();
        let saved = std::env::var_os("PATH");
        // SAFETY: ENV_LOCK held via ScopedEnv serializes PATH mutation.
        unsafe {
            std::env::set_var("PATH", &dir);
        }
        let found = cas_binary_on_path();
        unsafe {
            match &saved {
                Some(p) => std::env::set_var("PATH", p),
                None => std::env::remove_var("PATH"),
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!found, "cas_binary_on_path must be false when no `cas` is on PATH");
    }

    #[tokio::test]
    async fn test_pty_config_codex_supervisor_instructions() {
        let config = PtyConfig::codex(
            "test-supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            None,  // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("CAS Factory Supervisor"),
            "Codex supervisor should have supervisor instructions"
        );
    }

    /// cas-83c8: the Codex supervisor prompt must explain that worker messages
    /// arrive asynchronously as injected turns and must be triaged + replied to
    /// via mcp__cs__coordination, not treated as a fresh startup.
    #[tokio::test]
    async fn test_pty_config_codex_supervisor_handles_injected_worker_messages() {
        let config = PtyConfig::codex(
            "test-supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("Message from <sender>"),
            "supervisor prompt must reference the injected message framing; got: {all_args}"
        );
        assert!(
            all_args.contains("triage trigger"),
            "supervisor prompt must frame incoming worker messages as a triage trigger; got: {all_args}"
        );
        assert!(
            all_args.contains("mcp__cs__coordination action=message target=<worker>"),
            "supervisor prompt must tell it to reply/redirect via mcp__cs__coordination; got: {all_args}"
        );
        // Must keep the mcp__cs__ alias (not the claude mcp__cas__ alias).
        assert!(
            !all_args.contains("mcp__cas__"),
            "Codex supervisor prompt must use the mcp__cs__ alias, never mcp__cas__; got: {all_args}"
        );
    }

    /// cas-83c8: the Codex worker prompt must instruct continued availability
    /// after a task closes (you are not permanently done) and acting on injected
    /// supervisor messages, without breaking the one-task-at-a-time rule.
    #[tokio::test]
    async fn test_pty_config_codex_worker_stays_available_for_injected_messages() {
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-worker",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None, // model
            None, // effort
            None, // teams
        );
        let all_args = config.args.join(" ");
        assert!(
            all_args.contains("not permanently done"),
            "worker prompt must say it is not permanently done after closing a task; got: {all_args}"
        );
        assert!(
            all_args.contains("Message from <sender>"),
            "worker prompt must instruct acting on injected 'Message from <sender>' turns; got: {all_args}"
        );
        // The one-task-at-a-time discipline must still be present alongside the
        // new continued-availability clause.
        assert!(
            all_args.contains("Work exactly ONE task at a time"),
            "worker prompt must retain one-task-at-a-time discipline; got: {all_args}"
        );
        assert!(
            all_args.contains("finish or hand off your current task before starting the next"),
            "worker prompt must reconcile continued availability with the one-task rule; got: {all_args}"
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_with_teams() {
        let teams = TeamsSpawnConfig {
            team_name: "test-team".to_string(),
            agent_id: "worker-1@test-team".to_string(),
            agent_name: "worker-1".to_string(),
            agent_color: "blue".to_string(),
            agent_type: "general-purpose".to_string(),
            parent_session_id: Some("lead-session-123".to_string()),
            lead_session_id: None,
            settings_path: None,
        };
        let config = PtyConfig::claude(
            "worker-1",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            Some(&teams),
        );
        assert!(config.args.contains(&"--team-name".to_string()));
        assert!(config.args.contains(&"test-team".to_string()));
        assert!(config.args.contains(&"--agent-id".to_string()));
        assert!(config.args.contains(&"worker-1@test-team".to_string()));
        assert!(config.args.contains(&"--agent-name".to_string()));
        assert!(config.args.contains(&"--agent-color".to_string()));
        assert!(config.args.contains(&"--teammate-mode".to_string()));
        assert!(config.args.contains(&"tmux".to_string()));
        assert!(config.args.contains(&"--parent-session-id".to_string()));
        assert!(config.args.contains(&"lead-session-123".to_string()));
        // Workers get --session-id for CAS agent auto-registration
        assert!(config.args.contains(&"--session-id".to_string()));
        assert!(
            config
                .env
                .iter()
                .any(|(k, v)| k == "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS" && v == "1")
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_custom_effort() {
        let config = PtyConfig::claude(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            Some("low"),  // effort override
            None,  // teams
        );
        let effort_idx = config
            .args
            .iter()
            .position(|a| a == "--effort")
            .expect("--effort must be present");
        assert_eq!(
            config.args[effort_idx + 1], "low",
            "custom effort should override hardcoded default"
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_supervisor_default_effort() {
        // When effort is None and role is supervisor, must default to "xhigh"
        let config = PtyConfig::claude(
            "sup",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // no effort — should fall back to "xhigh"
            None,  // teams
        );
        let effort_idx = config
            .args
            .iter()
            .position(|a| a == "--effort")
            .expect("--effort must be present");
        assert_eq!(config.args[effort_idx + 1], "xhigh");
    }

    #[tokio::test]
    async fn test_pty_config_claude_worker_default_effort() {
        // When effort is None and role is worker, must default to "high"
        let config = PtyConfig::claude(
            "wrk",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // no effort — should fall back to "high"
            None,  // teams
        );
        let effort_idx = config
            .args
            .iter()
            .position(|a| a == "--effort")
            .expect("--effort must be present");
        assert_eq!(config.args[effort_idx + 1], "high");
    }

    #[tokio::test]
    async fn test_pty_config_codex_with_effort() {
        // Worker-role tests must hold ENV_LOCK (via ScopedEnv) so CAS_FACTORY_NICE_WORKER
        // cannot be set concurrently, which would shift arg indices via maybe_wrap_with_nice.
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            Some("medium"),  // effort
            None,  // teams
        );
        // Multiple `-c` flags exist now that the CAS MCP server is spawn-injected
        // (cas-bbc2), so locate the effort override by its value rather than
        // assuming it is the first `-c`. It must be emitted as a `-c` pair.
        let effort_idx = config
            .args
            .iter()
            .position(|a| a == "model_reasoning_effort=medium")
            .expect("effort override must emit model_reasoning_effort TOML key");
        assert_eq!(
            config.args[effort_idx - 1],
            "-c",
            "model_reasoning_effort override must be preceded by a -c flag"
        );
    }

    #[tokio::test]
    async fn test_pty_config_codex_no_effort_when_none() {
        // Worker-role tests must hold ENV_LOCK (via ScopedEnv).
        let _e = ScopedEnv::new();
        let config = PtyConfig::codex(
            "test-agent",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort — None means no override; Codex CLI server-side default applies
            None,  // teams
        );
        assert!(
            !config.args.iter().any(|a| a.starts_with("model_reasoning_effort")),
            "no model_reasoning_effort arg should be emitted when effort is None"
        );
        // The CAS MCP server injection (cas-bbc2) always emits `-c` flags, so we
        // can no longer assert the total absence of `-c`. Instead assert that the
        // only `-c` overrides present are the MCP server ones — none configure
        // reasoning effort.
        let c_values: Vec<&String> = config
            .args
            .windows(2)
            .filter(|w| w[0] == "-c")
            .map(|w| &w[1])
            .collect();
        assert!(
            c_values.iter().all(|v| v.starts_with("mcp_servers.cs.")),
            "with effort=None the only -c overrides should be the cas MCP server injection; got: {c_values:?}"
        );
    }

    #[tokio::test]
    async fn test_pty_config_claude_with_teams_lead() {
        let teams = TeamsSpawnConfig {
            team_name: "test-team".to_string(),
            agent_id: "supervisor@test-team".to_string(),
            agent_name: "supervisor".to_string(),
            agent_color: "green".to_string(),
            agent_type: "team-lead".to_string(),
            parent_session_id: None,
            lead_session_id: None,
            settings_path: None,
        };
        let config = PtyConfig::claude(
            "supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,  // model
            None,  // effort
            Some(&teams),
        );
        // Lead also gets --teammate-mode so it polls its inbox
        assert!(config.args.contains(&"--teammate-mode".to_string()));
        assert!(config.args.contains(&"tmux".to_string()));
        // No --parent-session-id for the lead
        assert!(!config.args.contains(&"--parent-session-id".to_string()));
    }

    /// When `TeamsSpawnConfig::settings_path` is set (as it is for the
    /// supervisor in factory mode), the spawned `claude` invocation must
    /// include `--settings <path>` so Claude Code loads the allowlist that
    /// sidesteps the self-leadership routing deadlock. Workers without a
    /// `settings_path` must not get the flag.
    #[tokio::test]
    async fn test_pty_config_claude_teams_supervisor_gets_settings_flag() {
        let settings_path = "/home/pippenz/.claude/teams/deadlock-team/supervisor-settings.json";
        let teams = TeamsSpawnConfig {
            team_name: "deadlock-team".to_string(),
            agent_id: "supervisor@deadlock-team".to_string(),
            agent_name: "supervisor".to_string(),
            agent_color: "green".to_string(),
            agent_type: "team-lead".to_string(),
            parent_session_id: None,
            lead_session_id: None,
            settings_path: Some(settings_path.to_string()),
        };
        let config = PtyConfig::claude(
            "supervisor",
            "supervisor",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,
            None,  // effort
            Some(&teams),
        );
        assert!(
            config.args.contains(&"--settings".to_string()),
            "supervisor spawn must include --settings flag"
        );
        assert!(
            config.args.contains(&settings_path.to_string()),
            "supervisor spawn must pass the settings file path"
        );
    }

    /// Workers now ship their own settings file (cas-e15d). When
    /// `settings_path` is populated, the `--settings <path>` flag must
    /// appear in argv so `claude` loads the per-worker allowlist and the
    /// phantom `team-lead` escalation cannot fire.
    #[tokio::test]
    async fn test_pty_config_claude_teams_worker_gets_settings_flag() {
        let settings_path = "/home/pippenz/.claude/teams/deadlock-team/worker-1-settings.json";
        let teams = TeamsSpawnConfig {
            team_name: "deadlock-team".to_string(),
            agent_id: "worker-1@deadlock-team".to_string(),
            agent_name: "worker-1".to_string(),
            agent_color: "blue".to_string(),
            agent_type: "general-purpose".to_string(),
            parent_session_id: Some("lead-session-xyz".to_string()),
            lead_session_id: None,
            settings_path: Some(settings_path.to_string()),
        };
        let config = PtyConfig::claude(
            "worker-1",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,
            None,  // effort
            Some(&teams),
        );
        assert!(
            config.args.contains(&"--settings".to_string()),
            "worker spawn must include --settings flag"
        );
        assert!(
            config.args.contains(&settings_path.to_string()),
            "worker spawn must pass the worker settings file path"
        );
    }

    /// Argv builder contract: when `settings_path` is deliberately left as
    /// `None` (CLI usage, tests that opt out), the flag must be absent. This
    /// is the correctness gate for the `if let Some(..)` branch — not a
    /// statement about worker doctrine (workers get a path in production).
    #[tokio::test]
    async fn test_pty_config_claude_teams_no_settings_path_omits_flag() {
        let teams = TeamsSpawnConfig {
            team_name: "no-settings-team".to_string(),
            agent_id: "worker-bare@no-settings-team".to_string(),
            agent_name: "worker-bare".to_string(),
            agent_color: "blue".to_string(),
            agent_type: "general-purpose".to_string(),
            parent_session_id: Some("lead-session-xyz".to_string()),
            lead_session_id: None,
            settings_path: None,
        };
        let config = PtyConfig::claude(
            "worker-bare",
            "worker",
            PathBuf::from("/tmp"),
            None,
            None,
            None,
            None,
            None,  // effort
            Some(&teams),
        );
        assert!(
            !config.args.contains(&"--settings".to_string()),
            "no settings_path → argv must omit --settings"
        );
    }

    // cas-0bf4: resource-contention mitigation tests.
    //
    // These exercise `cargo_build_jobs_for_worker` and
    // `maybe_wrap_with_nice` plus their integration with
    // `PtyConfig::claude`. They poke process-wide env vars, so they
    // share a serializing mutex to avoid cross-test flakes when the
    // suite runs with multiple threads. Scope is per-test: each test
    // clears its own env on entry and on the exit via the guard.
    mod cas_0bf4_resource_contention {
        use crate::pty::*;
        use crate::pty::tests::ScopedEnv;

        #[test]
        fn cargo_build_jobs_honours_explicit_env_override() {
            let _e = ScopedEnv::new();
            // SAFETY: _e holds ENV_LOCK.
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "3");
            }
            assert_eq!(cargo_build_jobs_for_worker().as_deref(), Some("3"));
        }

        #[test]
        fn cargo_build_jobs_trims_whitespace_override() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "  6  ");
            }
            assert_eq!(cargo_build_jobs_for_worker().as_deref(), Some("6"));
        }

        #[test]
        fn cargo_build_jobs_auto_falls_through_to_computed() {
            let _e = ScopedEnv::new();
            // Explicit "auto" reads as fallthrough, computed value comes back.
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "auto");
            }
            let got = cargo_build_jobs_for_worker()
                .expect("available_parallelism should succeed on test host");
            let n: usize = got.parse().expect("computed CARGO_BUILD_JOBS must parse");
            assert!(n >= 2, "floor of 2 must hold even on 1–4 core hosts: got {n}");
        }

        #[test]
        fn cargo_build_jobs_empty_env_falls_through_to_computed() {
            let _e = ScopedEnv::new();
            // No env set at all → compute. Same assertion as "auto".
            let got = cargo_build_jobs_for_worker()
                .expect("available_parallelism should succeed on test host");
            let n: usize = got.parse().expect("computed CARGO_BUILD_JOBS must parse");
            assert!(n >= 2);
        }

        #[test]
        fn maybe_wrap_with_nice_is_noop_for_supervisor_role() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
            let (cmd, args) = maybe_wrap_with_nice(
                "claude",
                vec!["--session-id".to_string(), "abc".to_string()],
                "supervisor",
            );
            assert_eq!(cmd, "claude");
            assert_eq!(args, vec!["--session-id".to_string(), "abc".to_string()]);
        }

        #[test]
        fn maybe_wrap_with_nice_is_noop_without_env_sentinel() {
            let _e = ScopedEnv::new();
            // No CAS_FACTORY_NICE_WORKER set — passthrough for workers too.
            let (cmd, args) = maybe_wrap_with_nice(
                "claude",
                vec!["--foo".to_string()],
                "worker",
            );
            assert_eq!(cmd, "claude");
            assert_eq!(args, vec!["--foo".to_string()]);
        }

        #[test]
        fn maybe_wrap_with_nice_wraps_worker_when_sentinel_set() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
            let (cmd, args) = maybe_wrap_with_nice(
                "claude",
                vec!["--session-id".to_string(), "xyz".to_string()],
                "worker",
            );
            assert_eq!(cmd, "nice");
            // Default level 10, original argv preserved after the wrapped command.
            assert_eq!(
                args,
                vec![
                    "-n".to_string(),
                    "10".to_string(),
                    "claude".to_string(),
                    "--session-id".to_string(),
                    "xyz".to_string(),
                ]
            );
        }

        #[test]
        fn maybe_wrap_with_nice_honours_level_override() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
                std::env::set_var("CAS_FACTORY_NICE_LEVEL", "15");
            }
            let (cmd, args) = maybe_wrap_with_nice("claude", vec![], "worker");
            assert_eq!(cmd, "nice");
            assert_eq!(args[..2], ["-n".to_string(), "15".to_string()]);
            assert_eq!(args[2], "claude");
        }

        #[test]
        fn maybe_wrap_with_nice_rejects_non_1_sentinel_value() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "true"); // not "1"
            }
            let (cmd, _args) = maybe_wrap_with_nice("claude", vec![], "worker");
            assert_eq!(cmd, "claude", "only the exact value '1' activates nice-wrap");
        }

        #[test]
        fn claude_worker_gets_cargo_build_jobs_env() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "4");
            }
            let config = PtyConfig::claude(
                "w1",
                "worker",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert!(
                config
                    .env
                    .iter()
                    .any(|(k, v)| k == "CARGO_BUILD_JOBS" && v == "4"),
                "worker PtyConfig must export CARGO_BUILD_JOBS when override is set"
            );
        }

        #[test]
        fn claude_supervisor_does_not_get_cargo_build_jobs_env() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "4");
            }
            let config = PtyConfig::claude(
                "s1",
                "supervisor",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert!(
                !config.env.iter().any(|(k, _)| k == "CARGO_BUILD_JOBS"),
                "supervisor must NOT get CARGO_BUILD_JOBS cap — only workers do"
            );
        }

        #[test]
        fn claude_worker_command_wraps_in_nice_when_sentinel_set() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
            let config = PtyConfig::claude(
                "w1",
                "worker",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert_eq!(config.command, "nice");
            assert_eq!(config.args[0], "-n");
            assert_eq!(config.args[2], "claude");
        }

        #[test]
        fn cargo_build_jobs_case_insensitive_auto_falls_through() {
            // cas-0bf4 adversarial P2: a user who writes "Auto" or "AUTO"
            // in config must not leak the literal string into
            // CARGO_BUILD_JOBS (cargo would reject it as a non-integer
            // and silently defeat the cap).
            let _e = ScopedEnv::new();
            for variant in ["Auto", "AUTO", "auto", "  Auto  "] {
                unsafe {
                    std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", variant);
                }
                let got = cargo_build_jobs_for_worker()
                    .expect("available_parallelism should succeed on test host");
                let n: usize = got.parse().expect("computed value must parse as integer");
                assert!(n >= 2, "variant {variant:?} should fall through to auto-compute, got {got}");
            }
        }

        #[test]
        fn maybe_wrap_with_nice_rejects_non_numeric_level() {
            // cas-0bf4 correctness P2: a non-numeric NICE_LEVEL must not
            // reach `nice -n <garbage>` — would fail every worker spawn.
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
                std::env::set_var("CAS_FACTORY_NICE_LEVEL", "high");
            }
            let (cmd, args) = maybe_wrap_with_nice("claude", vec![], "worker");
            assert_eq!(cmd, "nice");
            assert_eq!(args[..2], ["-n".to_string(), "10".to_string()],
                "non-numeric NICE_LEVEL must fall back to default 10");
        }

        #[test]
        fn maybe_wrap_with_nice_accepts_negative_numeric_level() {
            // Negative values parse as valid i32 and pass through; `nice`
            // itself rejects them for non-root, which is a separate OS
            // concern outside this helper. Documents the contract so a
            // future clamp-to-positive refactor is an explicit decision.
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
                std::env::set_var("CAS_FACTORY_NICE_LEVEL", "-5");
            }
            let (_cmd, args) = maybe_wrap_with_nice("claude", vec![], "worker");
            assert_eq!(args[1], "-5");
        }

        #[test]
        fn codex_worker_gets_cargo_build_jobs_env() {
            // cas-0bf4 testing P1: codex spawn path must mirror claude.
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "4");
            }
            let config = PtyConfig::codex(
                "w1",
                "worker",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert!(
                config
                    .env
                    .iter()
                    .any(|(k, v)| k == "CARGO_BUILD_JOBS" && v == "4"),
                "codex worker PtyConfig must export CARGO_BUILD_JOBS when override is set"
            );
        }

        #[test]
        fn codex_supervisor_does_not_get_cargo_build_jobs_env() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", "4");
            }
            let config = PtyConfig::codex(
                "s1",
                "supervisor",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert!(
                !config.env.iter().any(|(k, _)| k == "CARGO_BUILD_JOBS"),
                "codex supervisor must NOT get CARGO_BUILD_JOBS cap"
            );
        }

        #[test]
        fn codex_worker_command_wraps_in_nice_when_sentinel_set() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
            let config = PtyConfig::codex(
                "w1",
                "worker",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert_eq!(config.command, "nice");
            assert_eq!(config.args[0], "-n");
            assert_eq!(config.args[2], "codex");
        }

        #[test]
        fn claude_supervisor_command_unwrapped_even_when_sentinel_set() {
            let _e = ScopedEnv::new();
            unsafe {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
            let config = PtyConfig::claude(
                "s1",
                "supervisor",
                PathBuf::from("/tmp"),
                None,
                None,
                None,
                None,
                None,  // effort
                None,
            );
            assert_eq!(
                config.command, "claude",
                "supervisor must not be niced — the whole point is it stays at nice 0"
            );
        }
    }

    // ---- cas-c931: turn-break keystroke characterization ----
    //
    // The urgent interrupt-and-redirect path breaks a worker's turn with Esc
    // (0x1b), NOT Ctrl+C (0x03). `Pty::interrupt` sends 0x03; the Esc payload
    // is sent at the Pane/Mux layer (`Pane::break_turn`). These tests lock the
    // byte values against a real PTY so we never regress the payload.

    /// Esc (0x1b) is NOT a signal-generating control char (unlike 0x03 = INTR),
    /// so it traverses the PTY rather than killing the child. We send Esc then a
    /// newline through `cat`: canonical-mode `cat` flushes the line and the
    /// content echoes back. The Esc surfaces either verbatim (0x1b) or as the
    /// line-discipline control rendering `^[` (0x5e 0x5b) depending on ECHOCTL.
    /// Either proves the exact `Pane::break_turn` payload reaches the child
    /// intact — and, crucially, that it does NOT terminate the process the way
    /// Ctrl+C does (the contrast locked by `interrupt_sends_ctrl_c...`).
    #[tokio::test]
    async fn esc_byte_reaches_pty_child_verbatim() {
        let config = PtyConfig {
            command: "cat".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            rows: 24,
            cols: 80,
        };
        let mut pty = match Pty::spawn("esc-probe", config) {
            Ok(p) => p,
            Err(_) => return, // `cat` unavailable in this environment — skip.
        };

        // Esc (the exact payload of Pane::break_turn) followed by newline so
        // canonical-mode cat flushes the line back to us.
        pty.write(&[0x1b]).await.expect("write esc");
        pty.write(b"\r").await.expect("write newline");

        // Drain echoed output for up to ~2s. Accept raw 0x1b OR the ECHOCTL
        // rendering "^[". Also assert the child stays ALIVE (no Exited event) —
        // Esc must not behave like Ctrl+C.
        let mut saw_esc = false;
        let mut exited = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while std::time::Instant::now() < deadline && !saw_esc {
            match tokio::time::timeout(std::time::Duration::from_millis(250), pty.recv()).await {
                Ok(Some(PtyEvent::Output(data))) => {
                    let rendered_caret = data
                        .windows(2)
                        .any(|w| w == [0x5e, 0x5b]); // "^["
                    if data.contains(&0x1b) || rendered_caret {
                        saw_esc = true;
                    }
                }
                Ok(Some(PtyEvent::Exited(_))) | Ok(Some(PtyEvent::Error(_))) => {
                    exited = true;
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(_) => {} // timeout tick — keep waiting until deadline
            }
        }
        pty.kill();
        assert!(
            !exited,
            "Esc (0x1b) must NOT terminate the child the way Ctrl+C does"
        );
        assert!(
            saw_esc,
            "Esc (0x1b) must reach the PTY child and echo back (verbatim or as ^[)"
        );
    }

    /// Lock the `Pty::interrupt` payload: it sends Ctrl+C (0x03), the quit
    /// signal — distinct from the Esc turn-break. We assert by behavior: 0x03
    /// is INTR in the default line discipline, so it terminates `cat`.
    #[tokio::test]
    async fn interrupt_sends_ctrl_c_and_terminates_cat() {
        let config = PtyConfig {
            command: "cat".to_string(),
            args: vec![],
            cwd: None,
            env: vec![],
            rows: 24,
            cols: 80,
        };
        let mut pty = match Pty::spawn("intr-probe", config) {
            Ok(p) => p,
            Err(_) => return, // `cat` unavailable — skip.
        };

        pty.interrupt().await.expect("interrupt"); // writes 0x03

        // 0x03 = INTR → SIGINT → cat exits. Expect an Exited/Error event.
        let mut exited = false;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match tokio::time::timeout(std::time::Duration::from_millis(250), pty.recv()).await {
                Ok(Some(PtyEvent::Exited(_))) | Ok(Some(PtyEvent::Error(_))) => {
                    exited = true;
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    exited = true; // channel closed = process gone
                    break;
                }
                Err(_) => {}
            }
        }
        pty.kill();
        assert!(
            exited,
            "Ctrl+C (0x03) from interrupt() must terminate cat (INTR signal)"
        );
    }
}
