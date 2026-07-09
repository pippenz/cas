//! Factory worker liveness triage and recovery verbs (cas-4513).
//!
//! When a Claude Code worker's React-Ink UI throws an unhandled rejection it
//! dumps the Bun-bundled minified source + JS stack into stdout; the Bun event
//! loop does NOT exit on unhandled rejection, so the process stays alive with
//! its PID visible, but tool calls never complete. The supervisor sees a
//! "crashed-looking" pane, a fresh heartbeat (daemon-faked), and no way to
//! distinguish "alive but starved", "wedged in JS crash screen", or "actually
//! dead" without manual triage.
//!
//! This module adds three operator verbs to `cas factory`:
//!
//! * `is-wedged <worker>` — classify the worker as Alive / Wedged / Starved /
//!   Dead / Unverified by combining PID liveness, transcript mtime, worktree
//!   edit recency, and a content grep for the Bun/React-Ink crash-screen
//!   signature. Exits with a differentiated code so supervisor skills can
//!   script. `Dead` requires at least two independent signals to agree
//!   (cas-f781) — a pid-only "gone" reading that's contradicted by a fresh
//!   transcript or worktree edit reports `Unverified` instead, since that
//!   combination is what a stale/wrong tracked pid looks like while the
//!   real worker is still alive.
//! * `debug <worker>` — print the tail of the worker's transcript JSONL so a
//!   supervisor can see the last in-flight tool call without attaching the
//!   TUI. Essential triage input before deciding to kill.
//! * `kill <worker>` — SIGKILL the worker (SIGTERM doesn't exit cleanly on
//!   the Bun wedge) and best-effort release the CAS lease.
//!
//! See `cas-cli/src/mcp/tools/service/factory_ops.rs::resolve_transcript`
//! (cas-900b) for the transcript path resolver used by `is-wedged` / `debug`.

use anyhow::{Context, Result, anyhow, bail};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::mcp::tools::service::factory_ops::{
    TranscriptResolution, default_claude_projects_dir, resolve_transcript,
};

/// Window in which a transcript mtime counts as "recent" — used to distinguish
/// a worker that is still writing tool results (alive or wedged) from one
/// whose transcript has gone cold (starved or dead).
///
/// 60 seconds chosen to comfortably cover:
///   - the 30 s `WORKER_STALE_SECS` supervisor-heartbeat threshold
///     (factory_ops.rs cas-8240), and
///   - the ~45 s upper end of a single `cargo test` run on the saturated-host
///     case documented in cas-0bf4.
///
/// A transcript that hasn't been touched in a minute is almost certainly not
/// actively executing; that's the signal the supervisor needs.
pub(crate) const TRANSCRIPT_FRESH_WINDOW: Duration = Duration::from_secs(60);

/// Number of trailing JSONL lines inspected for the crash-screen signature.
///
/// Bumped to 200 from the original 20 after adversarial review (cas-4513)
/// flagged the tail-window gap: the Bun event loop can continue writing
/// transcript entries after an Ink crash renders on the PTY, and a single
/// long assistant reply can evict a 20-line crash block out of the
/// detection window. 200 lines comfortably covers roughly the last
/// half-dozen tool-call cycles on a typical transcript while still
/// bounding memory.
pub(crate) const CRASH_SIGNATURE_TAIL_LINES: usize = 200;

/// Evidence collected by [`classify_worker`], surfaced verbatim in
/// `cas factory is-wedged` output so a supervisor can audit the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerEvidence {
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub transcript_path: Option<PathBuf>,
    pub transcript_mtime_age_secs: Option<u64>,
    pub crash_signature_match: bool,
    /// Age since the most recently modified dirty file under the worker's
    /// worktree (per `git status --porcelain`), if resolvable. Second
    /// corroborating signal for the Dead/Unverified split (cas-f781 AC c).
    pub worktree_edit_age_secs: Option<u64>,
    /// Raw session_id the classification resolved against (reported so the
    /// supervisor can grep the projects tree manually if they distrust the
    /// resolution, per the cas-900b always-surface-session-id contract).
    pub session_id: String,
}

/// Liveness classification produced by [`classify_worker`]. The variants are
/// intentionally operator-facing — they match the names a supervisor would
/// use in a runbook, not any internal state model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum WorkerLivenessState {
    /// PID alive, transcript fresh, no crash signature. Worker is running.
    Alive,
    /// PID alive, transcript fresh, crash signature matched. Worker is in
    /// the Bun/React-Ink wedge — SIGKILL + respawn is the recovery.
    Wedged,
    /// PID alive, transcript stale (no writes in
    /// [`TRANSCRIPT_FRESH_WINDOW`]). Likely scheduler-starved or hung on a
    /// tool call. Often resolves with patience; not automatically fatal.
    Starved,
    /// PID gone AND a second signal corroborates it (transcript stale AND
    /// worktree not recently edited). The cleanup path is the same as
    /// SIGKILL-after-wedge (release lease, prune worktree). Not an error —
    /// just means the worker already exited.
    Dead,
    /// PID probe says gone, but the transcript is still fresh or the
    /// worktree was recently edited — a contradiction. cas-f781: this is
    /// exactly what a stale/wrong tracked pid looks like while the real
    /// worker is still alive and working. Never auto-reset a lease off this
    /// state alone; investigate with `debug` first.
    Unverified,
}

impl WorkerLivenessState {
    /// Process exit code for the `is-wedged` subcommand. Different values so
    /// supervisor bash scripts can branch without parsing stdout.
    ///
    /// Keep in sync with the `STATE_EXIT_CODES` constant asserted in
    /// `classify_worker_state_exit_codes_are_pinned`.
    pub(crate) fn exit_code(&self) -> i32 {
        match self {
            WorkerLivenessState::Alive => 0,
            WorkerLivenessState::Wedged => 1,
            WorkerLivenessState::Starved => 2,
            WorkerLivenessState::Dead => 3,
            WorkerLivenessState::Unverified => 4,
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            WorkerLivenessState::Alive => "alive",
            WorkerLivenessState::Wedged => "wedged",
            WorkerLivenessState::Starved => "starved",
            WorkerLivenessState::Dead => "dead",
            WorkerLivenessState::Unverified => "unverified",
        }
    }
}

/// React-Ink + Bun bundle signature bytes that leak into stdout when the CLI
/// throws an unhandled rejection. A match on any of these inside the
/// transcript's last [`CRASH_SIGNATURE_TAIL_LINES`] lines is sufficient —
/// each one independently identifies the crash-screen (cas-4513 discovery
/// note 2026-04-23 15:11 UTC captured all three in one pane).
///
/// Ordered most-specific-first: the literal Ink guard-text is a near-zero
/// false-positive signal and lives at the front. The bundler-path signals
/// (`/$bunfs/root`, `createInstance (/`) could, hypothetically, appear in
/// legitimate diagnostic output — hitting them alone is still a strong
/// enough signal to classify Wedged, but if the cheaper string match up
/// front catches the common case that's a clear win.
const CRASH_SIGNATURE_NEEDLES: &[&str] = &[
    // Literal React-Ink runtime invariant — when this renders, the UI is
    // guaranteed wedged (upstream: anthropics/claude-code#52337).
    "<Box> can't be nested inside <Text>",
    // React-Ink element construction leaking through the error handler.
    "createElement(\"ink-",
    "ink-box",
    // Bun single-file-bundle prefix — only appears in stack frames dumped
    // by the error handler, never in normal transcripts.
    "/$bunfs/root",
    "createInstance (/",
];

/// Pure classifier — takes pre-measured inputs so tests drive it without
/// touching the real PID table or filesystem. The orchestrating
/// [`classify_worker`] wrapper does the measurement; keeping the decision
/// logic separate means the 4-way branch is exhaustively unit-testable
/// without ptrace or tempdir dependencies.
pub(crate) fn classify_from_evidence(
    pid_alive: bool,
    transcript_mtime_age: Option<Duration>,
    crash_signature: bool,
    worktree_recent_edit_age: Option<Duration>,
) -> WorkerLivenessState {
    let fresh = transcript_mtime_age
        .map(|age| age < TRANSCRIPT_FRESH_WINDOW)
        .unwrap_or(false);
    if !pid_alive {
        // cas-f781 AC c: a pid-only "not alive" reading must never emit
        // Dead by itself — require a second independent signal to
        // corroborate. If the transcript is still fresh OR the worktree
        // was recently edited while the pid probe says gone, that's a
        // contradiction: the concrete cas-f781 repro is a stale/wrong
        // tracked pid reading dead while the real worker process keeps
        // writing to its transcript and worktree. Report Unverified so a
        // caller (e.g. a supervisor auto-reset) doesn't treat one
        // contradicted signal as ground truth for a destructive action.
        let worktree_recent = worktree_recent_edit_age
            .map(|age| age < TRANSCRIPT_FRESH_WINDOW)
            .unwrap_or(false);
        return if fresh || worktree_recent {
            WorkerLivenessState::Unverified
        } else {
            WorkerLivenessState::Dead
        };
    }
    match (fresh, crash_signature) {
        (true, true) => WorkerLivenessState::Wedged,
        (true, false) => WorkerLivenessState::Alive,
        (false, _) => WorkerLivenessState::Starved,
    }
}

/// Measure transcript mtime-age. `None` means the file doesn't exist or the
/// mtime could not be read — treated as "not fresh" by the classifier.
pub(crate) fn transcript_mtime_age(path: &Path) -> Option<Duration> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    SystemTime::now().duration_since(mtime).ok()
}

/// Age since the most recently modified file that `git status --porcelain`
/// reports as changed under `clone_path` — the "is this worktree actively
/// being edited" signal (cas-f781 AC c, third corroborating signal
/// alongside pid liveness and transcript mtime). Only files git considers
/// dirty are checked, not the whole tree — `.git/objects` and `target/`
/// churn constantly regardless of real edits and would swamp the signal.
/// `None` when `clone_path` isn't a git worktree, git isn't on `PATH`, or
/// nothing is dirty — callers must treat `None` as "no signal", never as
/// "confirmed clean" (a worker between edits with a clean tree still
/// exists and may be alive).
pub(crate) fn worktree_recent_edit_age(clone_path: &Path) -> Option<Duration> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(clone_path)
        .arg("status")
        .arg("--porcelain")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut newest: Option<Duration> = None;
    for line in stdout.lines() {
        // `git status --porcelain` format: two status columns + a space,
        // then the path (renames use "old -> new"; take the new side).
        let Some(rel) = line.get(3..) else { continue };
        let rel = rel.rsplit(" -> ").next().unwrap_or(rel);
        if let Some(age) = transcript_mtime_age(&clone_path.join(rel)) {
            newest = Some(newest.map_or(age, |cur: Duration| cur.min(age)));
        }
    }
    newest
}

/// Collect the last `n` lines from `reader` via a bounded ring buffer.
/// Takes `Read` so tests drive it with `Cursor<Vec<u8>>`. A 0-line request
/// is a hard short-circuit — otherwise `VecDeque::with_capacity(0)` would
/// grow unboundedly as every iteration hits `ring.len() == 0` (a no-op
/// `pop_front` on empty, then `push_back`). cas-4513 P2 correctness catch.
pub(crate) fn collect_tail_lines<R: Read>(reader: R, n: usize) -> Vec<String> {
    if n == 0 {
        return Vec::new();
    }
    let bufread = BufReader::new(reader);
    let mut ring: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(n);
    for line in bufread.lines().map_while(Result::ok) {
        if ring.len() == n {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    ring.into_iter().collect()
}

/// Grep the last [`CRASH_SIGNATURE_TAIL_LINES`] lines of `reader` for any of
/// [`CRASH_SIGNATURE_NEEDLES`]. Takes `Read` so tests can point at a
/// `std::io::Cursor<Vec<u8>>` without touching the filesystem. Large
/// transcripts (thousands of lines) are fine — we only retain a bounded tail
/// window in memory.
pub(crate) fn has_crash_signature<R: Read>(reader: R, tail_lines: usize) -> bool {
    let tail = collect_tail_lines(reader, tail_lines);
    tail.iter()
        .any(|l| CRASH_SIGNATURE_NEEDLES.iter().any(|needle| l.contains(*needle)))
}

/// Convenience wrapper that opens `path` and runs [`has_crash_signature`].
/// Missing or unreadable files read as "no signature" — the classifier then
/// treats the absence as Alive/Starved based on pid + mtime alone.
pub(crate) fn transcript_has_crash_signature(path: &Path, tail_lines: usize) -> bool {
    match std::fs::File::open(path) {
        Ok(f) => has_crash_signature(f, tail_lines),
        Err(_) => false,
    }
}

/// Orchestrator that combines PID liveness, transcript mtime, and signature
/// grep. Called by all three verbs with the same inputs so a Wedged decision
/// in one surfaces consistently in the others.
///
/// `pid_alive_probe` is injectable so tests don't need to exercise the real
/// `kill(pid, 0)` path (cas-2749's `pid_alive` helper covers production).
pub(crate) fn classify_worker<F, G>(
    pid: Option<u32>,
    transcript_path: Option<&Path>,
    clone_path: Option<&Path>,
    session_id: &str,
    pid_alive_probe: F,
    worktree_age_probe: G,
) -> (WorkerLivenessState, WorkerEvidence)
where
    F: FnOnce(u32) -> bool,
    G: FnOnce(&Path) -> Option<Duration>,
{
    let pid_alive = pid.map(pid_alive_probe).unwrap_or(false);
    let (age_opt, sig) = match transcript_path {
        Some(p) => (
            transcript_mtime_age(p),
            transcript_has_crash_signature(p, CRASH_SIGNATURE_TAIL_LINES),
        ),
        None => (None, false),
    };
    let worktree_age = clone_path.and_then(worktree_age_probe);
    let state = classify_from_evidence(pid_alive, age_opt, sig, worktree_age);
    let evidence = WorkerEvidence {
        pid,
        pid_alive,
        transcript_path: transcript_path.map(PathBuf::from),
        transcript_mtime_age_secs: age_opt.map(|d| d.as_secs()),
        crash_signature_match: sig,
        worktree_edit_age_secs: worktree_age.map(|d| d.as_secs()),
        session_id: session_id.to_string(),
    };
    (state, evidence)
}

/// Resolve `(pid, clone_path, session_id, transcript_path)` for a worker by
/// name. Reads the active agent row from AgentStore. Returns an error if the
/// worker is unknown or has no registered PID — the verbs treat that as a
/// hard stop rather than making up evidence.
pub(crate) fn resolve_worker(
    cas_root: &Path,
    worker_name: &str,
) -> Result<ResolvedWorker> {
    use cas_store::{AgentStore, SqliteAgentStore};
    use cas_types::AgentStatus;
    let store = SqliteAgentStore::open(cas_root)
        .with_context(|| "open agent store")?;
    let mut matches: Vec<_> = [AgentStatus::Active, AgentStatus::Stale]
        .iter()
        .flat_map(|s| store.list(Some(*s)).unwrap_or_default())
        .filter(|a| a.name == worker_name)
        .collect();
    // Same name could be registered Stale + Active — prefer Active.
    matches.sort_by_key(|a| match a.status {
        AgentStatus::Active => 0,
        AgentStatus::Stale => 1,
        _ => 2,
    });
    let agent = matches
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("no worker named `{worker_name}` in agent store"))?;
    let pid = agent.pid;
    let clone_path = agent.metadata.get("clone_path").cloned();
    // factory-mode agents: id IS the CC session UUID (see cas-900b caller
    // comment). cc_session_id is populated in some non-factory registration
    // flows; prefer it when available.
    let session_id = agent
        .cc_session_id
        .clone()
        .unwrap_or_else(|| agent.id.clone());
    let transcript_path = match resolve_transcript(
        default_claude_projects_dir().as_deref(),
        clone_path.as_deref(),
        &session_id,
    ) {
        TranscriptResolution::Resolved(p) => Some(p),
        TranscriptResolution::Ambiguous { mut matches, .. } => {
            // Deterministic: most-recently-modified first. Ambiguity is rare
            // and always logged in the evidence; picking the freshest minimizes
            // the surprise when the supervisor runs `debug` on the chosen path.
            matches.sort_by_key(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .ok()
            });
            matches.pop()
        }
        TranscriptResolution::Synthesized(_) => {
            // Synthesized paths are a best-guess reconstruction of the
            // `<escaped-cwd>` — probably wrong on unicode / space paths
            // (which is the whole reason cas-900b exists). Treat as
            // unresolved here so the mtime and signature checks do not
            // fire against a potentially bogus path; the classifier
            // will fall through to Starved-or-Dead and the operator
            // runs `debug` with an explicit path if they need more.
            None
        }
    };
    Ok(ResolvedWorker {
        name: worker_name.to_string(),
        pid,
        // cas-4513 adversarial P0: thread the pid_starttime fingerprint
        // from the agent row so `execute_kill` can guard against a PID
        // that was recycled after the agent record was written. Falls
        // back to the stringly-typed metadata key for legacy rows
        // predating cas-b157's typed promotion.
        pid_starttime: agent.pid_starttime.or_else(|| {
            agent
                .metadata
                .get(crate::mcp::daemon::PID_STARTTIME_KEY)
                .and_then(|s| s.parse::<u64>().ok())
        }),
        clone_path,
        session_id,
        transcript_path,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedWorker {
    pub name: String,
    pub pid: Option<u32>,
    /// `/proc/<pid>/stat` starttime fingerprint, when the registration
    /// path captured one. Used by `execute_kill` to refuse SIGKILL on a
    /// PID whose fingerprint no longer matches (= PID was recycled).
    pub pid_starttime: Option<u64>,
    pub clone_path: Option<String>,
    pub session_id: String,
    pub transcript_path: Option<PathBuf>,
}

// ---------------------------------------------------------------------------
// Subcommand execution — thin glue between clap args and the helpers above.
// ---------------------------------------------------------------------------

/// `cas factory is-wedged <worker>`: classify + print evidence + exit.
pub(crate) fn execute_is_wedged(
    cas_root: Option<&Path>,
    worker: &str,
    json: bool,
) -> Result<()> {
    let cas_root =
        cas_root.ok_or_else(|| anyhow!("--cas-root required or run from a CAS project"))?;
    // Scope the store opens so their SqliteConnection drops (running any
    // pending WAL checkpoint) before we call `std::process::exit` — that
    // function skips Rust destructors entirely. cas-4513 adversarial P2.
    let exit_code = {
        let w = resolve_worker(cas_root, worker)?;
        let (state, evidence) = classify_worker(
            w.pid,
            w.transcript_path.as_deref(),
            w.clone_path.as_deref().map(Path::new),
            &w.session_id,
            crate::mcp::daemon::pid_alive,
            worktree_recent_edit_age,
        );
        if json {
            println!("{}", format_state_json(&state, &evidence));
        } else {
            println!("{}", format_state_human(&state, &evidence));
        }
        state.exit_code()
    };
    std::process::exit(exit_code);
}

/// `cas factory debug <worker>`: print tail of worker transcript.
pub(crate) fn execute_debug(
    cas_root: Option<&Path>,
    worker: &str,
    tail: usize,
) -> Result<()> {
    let cas_root =
        cas_root.ok_or_else(|| anyhow!("--cas-root required or run from a CAS project"))?;
    let w = resolve_worker(cas_root, worker)?;
    let Some(path) = w.transcript_path.as_deref() else {
        bail!(
            "no transcript found for worker `{worker}` (session {}). Try `cas factory status` \
             to see what the agent store knows.",
            w.session_id
        );
    };
    let lines = read_last_lines(path, tail)
        .with_context(|| format!("read transcript at {}", path.display()))?;
    println!("# transcript: {}", path.display());
    println!("# session:    {}", w.session_id);
    println!("# tail:       {} lines\n", lines.len());
    for line in lines {
        println!("{line}");
    }
    Ok(())
}

/// Minimal abstraction over the OS process table, injected so tests can
/// simulate `/proc` contents without spawning real processes. Real usage is
/// [`RealProcessTable`]; tests provide an in-memory fake. cas-f781.
pub(crate) trait ProcessTable {
    /// All PIDs currently visible in the table.
    fn pids(&self) -> Vec<u32>;
    /// Raw `/proc/<pid>/cmdline` bytes (NUL-separated argv), if readable.
    fn cmdline(&self, pid: u32) -> Option<Vec<u8>>;
    /// Raw `/proc/<pid>/environ` bytes (NUL-separated `KEY=VALUE`), if
    /// readable. Codex workers carry their identity here rather than in
    /// argv (cas-f781 investigation: the `codex` CLI has no `--agent-name`
    /// equivalent, only the `CAS_AGENT_NAME` env var).
    fn environ(&self, pid: u32) -> Option<Vec<u8>>;
}

/// Live `/proc` implementation. Linux-only, matching the existing
/// `read_pid_starttime` / fingerprint-guard gating in `daemon.rs` — other
/// platforms get an empty table and [`find_worker_pid`] always falls back
/// to the tracked pid.
pub(crate) struct RealProcessTable;

impl ProcessTable for RealProcessTable {
    #[cfg(target_os = "linux")]
    fn pids(&self) -> Vec<u32> {
        let Ok(entries) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|s| s.parse::<u32>().ok()))
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    fn pids(&self) -> Vec<u32> {
        Vec::new()
    }

    #[cfg(target_os = "linux")]
    fn cmdline(&self, pid: u32) -> Option<Vec<u8>> {
        std::fs::read(format!("/proc/{pid}/cmdline")).ok()
    }
    #[cfg(not(target_os = "linux"))]
    fn cmdline(&self, _pid: u32) -> Option<Vec<u8>> {
        None
    }

    #[cfg(target_os = "linux")]
    fn environ(&self, pid: u32) -> Option<Vec<u8>> {
        std::fs::read(format!("/proc/{pid}/environ")).ok()
    }
    #[cfg(not(target_os = "linux"))]
    fn environ(&self, _pid: u32) -> Option<Vec<u8>> {
        None
    }
}

/// Extract the value of a `--agent-name <value>` argument from raw
/// NUL-separated `/proc/<pid>/cmdline` bytes. Scans tokens rather than
/// assuming a fixed position so a `nice -n <N> claude ...` wrapper
/// (`maybe_wrap_with_nice`, cas-pty) doesn't shift the match.
pub(crate) fn agent_name_from_cmdline(cmdline: &[u8]) -> Option<String> {
    let tokens: Vec<&str> = cmdline
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .filter_map(|s| std::str::from_utf8(s).ok())
        .collect();
    tokens
        .iter()
        .position(|t| *t == "--agent-name")
        .and_then(|i| tokens.get(i + 1))
        .map(|s| s.to_string())
}

/// Extract `CAS_AGENT_NAME=<value>` from raw NUL-separated
/// `/proc/<pid>/environ` bytes — the Codex worker identity signal, since
/// Codex's argv carries no `--agent-name` flag (cas-f781 investigation).
pub(crate) fn agent_name_from_environ(environ: &[u8]) -> Option<String> {
    environ.split(|b| *b == 0).filter(|s| !s.is_empty()).find_map(|entry| {
        std::str::from_utf8(entry)
            .ok()
            .and_then(|s| s.strip_prefix("CAS_AGENT_NAME="))
            .map(|v| v.to_string())
    })
}

/// Scan the live process table for the pid whose cmdline or environ
/// identifies it as `worker_name`. This is the authoritative resolution
/// [`execute_kill`] trusts over the agent store's `pid` column — that
/// column can be overwritten by an unrelated process's self-registration
/// (cas-f781 discovery: an MCP-server child process re-registers over the
/// real `claude --agent-name <worker>` pid using its own
/// `std::process::id()`). Matching against a live process's own
/// argv/environ is a direct identity proof, unlike a stored pid that might
/// describe the wrong process entirely.
pub(crate) fn find_worker_pid<T: ProcessTable + ?Sized>(
    table: &T,
    worker_name: &str,
) -> Option<u32> {
    let pids = table.pids();
    // cas-a91b: argv (`--agent-name`) is only ever present on the actual
    // `claude`/leader process's OWN command line — unlike an env var, argv
    // is never copied to child processes. `CAS_AGENT_NAME`, by contrast, is
    // *inherited* by every descendant the worker spawns (its `cas serve`
    // child, git, cargo, ...), so an environ-only match is ambiguous — it
    // could be the leader or any of its children, and `ProcessTable::pids()`
    // order is unspecified. Search cmdline across ALL pids first (a global
    // pass, not interleaved per-pid); only fall back to the environ signal
    // (needed for Codex, whose argv carries no identifying flag at all) when
    // no process's own argv identifies it as this worker.
    if let Some(pid) = pids.iter().copied().find(|&pid| {
        table
            .cmdline(pid)
            .and_then(|c| agent_name_from_cmdline(&c))
            .as_deref()
            == Some(worker_name)
    }) {
        return Some(pid);
    }
    pids.into_iter().find(|&pid| {
        table
            .environ(pid)
            .and_then(|e| agent_name_from_environ(&e))
            .as_deref()
            == Some(worker_name)
    })
}

/// Convert `pid` to its process GROUP LEADER's pid via `getpgid()` (cas-a91b).
/// `find_worker_pid`'s environ-based fallback (Codex workers) can still
/// resolve a descendant rather than the actual session leader, since
/// `CAS_AGENT_NAME` is inherited by every child process. Converting through
/// the kernel's own process-group bookkeeping — rather than assuming the
/// resolved pid IS the pgid — is what makes `killpg` safe to call on it:
/// descendants stay in their parent's process group unless they explicitly
/// detach (`setsid`/`setpgid`), so `getpgid(descendant_pid)` correctly
/// returns the leader's pid. Returns `None` if the process is already gone
/// (`getpgid` fails, e.g. ESRCH) — callers fall back to the original pid,
/// which the subsequent liveness/kill checks handle as "already dead".
fn resolve_group_leader_pid(pid: u32) -> Option<u32> {
    let pgid = unsafe { libc::getpgid(pid as libc::pid_t) };
    if pgid < 0 { None } else { Some(pgid as u32) }
}

/// Pick which pid [`execute_kill`] targets: a live process-table match (by
/// agent-name/environ) always wins over the tracked agent-store pid, since
/// it's a direct identity proof rather than a value that might have been
/// clobbered (cas-f781). Falls back to the tracked pid when no live match
/// is found (offline host, non-Linux, or an unrecognized worker CLI).
pub(crate) fn pick_kill_pid(tracked_pid: Option<u32>, resolved_pid: Option<u32>) -> Option<u32> {
    resolved_pid.or(tracked_pid)
}

/// Decide whether `execute_kill` should proceed to reset the worker's
/// task leases, given the kill verdict and (for the `Go` case) whether
/// death was actually confirmed after the SIGKILL was delivered. cas-f781
/// AC b: a still-alive process — whether because the kill was refused or
/// because it demonstrably survived the signal — must never have its lease
/// reset out from under it.
pub(crate) fn decide_post_kill_action(verdict: &KillVerdict, death_confirmed_after_kill: bool) -> bool {
    match verdict {
        KillVerdict::AlreadyDead => true,
        KillVerdict::Go => death_confirmed_after_kill,
        KillVerdict::RefuseFingerprintMismatch | KillVerdict::RefuseNoFingerprint => false,
    }
}

/// Parse the process `state` (field 3) out of a raw `/proc/<pid>/stat` line,
/// `true` iff it's `Z` (zombie). Same comm-parsing caveat as
/// `daemon::parse_starttime_from_stat` (`comm` is parenthesized and may
/// itself contain spaces/parens) — split on the LAST `)` before reading
/// fields from the tail. Field 3 is the first field after the parens.
fn is_zombie_state(raw: &str) -> bool {
    let Some(last_paren) = raw.rfind(')') else {
        return false;
    };
    let Some(tail) = raw.get(last_paren + 1..) else {
        return false;
    };
    tail.trim_start().split_whitespace().next() == Some("Z")
}

/// Whether `pid` is currently a zombie (exited but not yet reaped by its
/// parent). Linux-only (`/proc`); non-Linux always reports `false` — see
/// `verify_death` doc for why that's the safe default there.
#[cfg(target_os = "linux")]
fn pid_is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_some_and(|raw| is_zombie_state(&raw))
}

#[cfg(not(target_os = "linux"))]
fn pid_is_zombie(_pid: u32) -> bool {
    false
}

/// Poll pid liveness briefly after SIGKILL. The signal-delivery syscall
/// returning success is not proof of death — the kernel needs a scheduling
/// tick to actually reap the process — and cas-f781 AC b requires the lease
/// reset to wait for genuinely-confirmed death rather than assume the kill
/// worked. 10 x 20ms = 200ms ceiling: generous for an already-signalled
/// process while keeping `cas factory kill` responsive.
///
/// cas-a91b: a killed process GROUP LEADER becomes a zombie under its
/// original parent (typically the daemon that spawned it, not this `cas
/// factory kill` invocation) — `pid_alive` (`kill(pid, 0)`) reports a zombie
/// as alive, since its `/proc` entry still exists until reaped. Without the
/// zombie check, `verify_death` would time out and return `false` for a
/// worker that is, for all practical purposes, dead — leaving its task
/// stuck InProgress with no way to reclaim it short of a manual reset.
fn verify_death(pid: u32) -> bool {
    for _ in 0..10 {
        if !crate::mcp::daemon::pid_alive(pid) || pid_is_zombie(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    !crate::mcp::daemon::pid_alive(pid) || pid_is_zombie(pid)
}

/// `cas factory kill <worker>`: SIGKILL the worker process and release any
/// active CAS lease. Idempotent — already-dead worker still runs the cleanup.
///
/// PID-recycling guard (cas-4513 adversarial P0): before delivering SIGKILL,
/// we verify the agent's stored `pid_starttime` fingerprint matches the
/// process currently at that PID. On a busy host the kernel can recycle a
/// PID between the agent row being written and `kill` being called;
/// without the fingerprint guard we could SIGKILL an unrelated process.
/// When the fingerprint check fails, we refuse unless `--force` is set.
/// Legacy agents without a stored fingerprint also require `--force`.
///
/// Process resolution (cas-f781 P0): the agent store's `pid` column is not
/// trusted blindly — it can be overwritten by an unrelated process's
/// self-registration (an MCP-server child stomping the real `claude
/// --agent-name <worker>` pid with its own). Before falling back to that
/// tracked pid, we scan the live process table for a process whose own
/// argv/environ identifies it as `worker` and prefer that instead
/// ([`pick_kill_pid`]). The resolved target is killed via the process
/// GROUP (`killpg`, see `send_sigkill`), not a single pid, since workers are
/// spawned as session leaders and may have forked children of their own.
///
/// Lease reset only fires after death is independently confirmed
/// ([`decide_post_kill_action`] + [`verify_death`]) — a kill that was
/// refused (fingerprint mismatch / no fingerprint) or that demonstrably
/// didn't take never resets the task lease out from under a still-running
/// worker.
pub(crate) fn execute_kill(
    cas_root: Option<&Path>,
    worker: &str,
    force: bool,
) -> Result<()> {
    let cas_root =
        cas_root.ok_or_else(|| anyhow!("--cas-root required or run from a CAS project"))?;
    let w = resolve_worker(cas_root, worker)?;
    let mut summary = Vec::<String>::new();

    let resolved_pid = find_worker_pid(&RealProcessTable, &w.name);
    if let (Some(tracked), Some(resolved)) = (w.pid, resolved_pid) {
        if tracked != resolved {
            summary.push(format!(
                "process-table scan resolved a live process for `{}` at pid {resolved} \
                 (agent-name match) — overriding stale tracked pid {tracked}",
                w.name
            ));
        }
    }
    let kill_pid = pick_kill_pid(w.pid, resolved_pid);
    let scan_confirmed = resolved_pid.is_some() && kill_pid == resolved_pid;

    // Inner scope so the SqliteAgentStore / SqliteTaskStore connections
    // opened by `reset_worker_tasks` drop (and any WAL checkpoints fire)
    // BEFORE we print the summary. cas-4513 adversarial P2.
    let death_confirmed = {
        match kill_pid {
            Some(pid) => {
                // A scan-resolved pid is already authoritatively identified
                // by its own live argv/environ — the starttime fingerprint
                // gate exists to guard a *tracked* pid that might describe
                // the wrong (recycled) process, which doesn't apply here.
                let verdict = if scan_confirmed {
                    if crate::mcp::daemon::pid_alive(pid) {
                        KillVerdict::Go
                    } else {
                        KillVerdict::AlreadyDead
                    }
                } else {
                    kill_verdict(pid, w.pid_starttime, force)
                };
                let death_after_attempt = match &verdict {
                    KillVerdict::Go => {
                        // cas-a91b: convert to the actual process GROUP
                        // LEADER before signaling — `pid` may be a descendant
                        // that inherited CAS_AGENT_NAME in its environ
                        // (find_worker_pid's Codex fallback), not the leader
                        // itself. `killpg`/`verify_death` only make sense
                        // against the real pgid; falling back to the raw
                        // `pid` when the process just vanished mid-resolve is
                        // fine — the ESRCH/pid_alive checks downstream still
                        // handle that safely.
                        let group_pid = resolve_group_leader_pid(pid).unwrap_or(pid);
                        match send_sigkill(group_pid) {
                            Ok(()) => {
                                summary.push(format!(
                                    "SIGKILL delivered to process group {group_pid}"
                                ));
                                verify_death(group_pid)
                            }
                            Err(e) => {
                                summary.push(format!(
                                    "SIGKILL failed for pid {group_pid}: {e}"
                                ));
                                // cas-a91b: do NOT fall through to verify_death
                                // on a failed/refused kill — a failure here
                                // must never be treated as "confirmed dead".
                                false
                            }
                        }
                    }
                    KillVerdict::AlreadyDead => {
                        summary.push(format!("pid {pid} already dead — skipping SIGKILL"));
                        true
                    }
                    KillVerdict::RefuseFingerprintMismatch => {
                        summary.push(format!(
                            "pid {pid} SKIPPED: starttime fingerprint mismatch (PID recycled). Pass --force to override."
                        ));
                        false
                    }
                    KillVerdict::RefuseNoFingerprint => {
                        summary.push(format!(
                            "pid {pid} SKIPPED: no starttime fingerprint recorded (legacy agent). Pass --force to override."
                        ));
                        false
                    }
                };
                let reset_ok = decide_post_kill_action(&verdict, death_after_attempt);
                if !reset_ok {
                    summary.push(format!(
                        "death not verified for pid {pid} — lease NOT reset (worker may still be running)"
                    ));
                }
                reset_ok
            }
            None => {
                summary.push(
                    "worker has no PID recorded and no live process resolved by agent-name — treating as dead"
                        .into(),
                );
                true
            }
        }
    };

    if death_confirmed {
        // Release leases + reset task status to Open. cas-4513 correctness P2
        // flagged that just releasing the lease (like the pre-fix code did)
        // leaves tasks stuck at InProgress with no assignee, so a fresh worker
        // can never claim them. Match the MCP `cas_task_reset` semantics:
        // release lease + status=Open + clear assignee, covers both
        // InProgress and Blocked task states (adversarial P2).
        match reset_worker_tasks(cas_root, &w.name) {
            Ok(n) if n > 0 => summary.push(format!(
                "reset {n} task(s) held by {}: released lease + status→Open + cleared assignee",
                w.name
            )),
            Ok(_) => summary.push("no active leases to release".into()),
            Err(e) => summary.push(format!("task reset failed: {e}")),
        }
    } else {
        summary.push(format!(
            "skipping lease reset for `{}` — worker death not confirmed",
            w.name
        ));
    }

    println!("kill-worker `{}` completed:", w.name);
    for line in summary {
        println!("  - {line}");
    }
    Ok(())
}

/// Decision for the SIGKILL stage of `execute_kill`, separated so the
/// PID-recycling guard logic is testable without real processes.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum KillVerdict {
    /// PID is alive AND fingerprint matches (or force override).
    Go,
    /// PID is already gone — cleanup still runs, kill is a no-op.
    AlreadyDead,
    /// PID is alive but fingerprint mismatch — refuse unless forced.
    RefuseFingerprintMismatch,
    /// PID is alive, no fingerprint stored (legacy agent) — refuse
    /// unless forced. Preserves PID-recycling safety for registrations
    /// predating cas-ea46.
    RefuseNoFingerprint,
}

fn kill_verdict(pid: u32, expected_starttime: Option<u64>, force: bool) -> KillVerdict {
    if !crate::mcp::daemon::pid_alive(pid) {
        return KillVerdict::AlreadyDead;
    }
    if force {
        return KillVerdict::Go;
    }
    match expected_starttime {
        None => KillVerdict::RefuseNoFingerprint,
        Some(expected) => {
            if crate::mcp::daemon::pid_matches_fingerprint(pid, expected) {
                KillVerdict::Go
            } else {
                KillVerdict::RefuseFingerprintMismatch
            }
        }
    }
}

fn send_sigkill(pgid: u32) -> Result<()> {
    // cas-f781: kill the process GROUP, not just the single recorded pid.
    // Workers are spawned as session leaders (portable_pty calls setsid()
    // before exec), so pid == pgid for the actual leader — killpg here also
    // reaps any children the worker forked (e.g. an in-flight tool
    // subprocess), where a bare `kill(pid)` would leave those running.
    // `pgid` must already be a real process-group id by the time this is
    // called — callers convert via `resolve_group_leader_pid` first
    // (cas-a91b), since a raw resolved pid can be a descendant rather than
    // the leader (see `find_worker_pid`'s environ-fallback ambiguity).
    // SAFETY: libc::killpg with SIGKILL has no side effects on this process.
    let rc = unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) };
    if rc == 0 {
        return Ok(());
    }
    let errno = std::io::Error::last_os_error();
    if errno.raw_os_error() == Some(libc::ESRCH) {
        // cas-a91b adversarial P1: ESRCH means "no process group with this
        // id exists" — but that's only trustworthy as "the worker's group is
        // dead" if `pgid` doesn't independently resolve to a still-alive
        // process. If it does, we passed the wrong number (e.g. a
        // descendant's raw pid that was never actually a valid pgid) and
        // silently treating this as success would let a live worker's task
        // lease get reset out from under it — the exact destructive bug this
        // task exists to close. Refuse instead of guessing.
        if crate::mcp::daemon::pid_alive(pgid) {
            bail!(
                "killpg({pgid}) returned ESRCH but pid {pgid} is still alive — refusing to \
                 treat this as a successful kill (the resolved target was not a valid process \
                 group; the worker may still be running)"
            );
        }
        return Ok(());
    }
    Err(errno.into())
}

/// Fully reset every active task held by `worker_name`: release lease,
/// force status to Open, clear assignee. Matches the MCP `cas_task_reset`
/// semantics (see `task_claiming.rs` cas_task_reset) so a supervisor
/// running `cas factory kill` doesn't have to chase up with a second
/// `action=reset` per task to make them claimable again. Covers both
/// `InProgress` and `Blocked` assignment states (cas-4513 adversarial P2).
fn reset_worker_tasks(cas_root: &Path, worker_name: &str) -> Result<usize> {
    use cas_store::{AgentStore, SqliteAgentStore, SqliteTaskStore, TaskStore};
    use cas_types::TaskStatus;
    let task_store = SqliteTaskStore::open(cas_root).with_context(|| "open task store")?;
    let agent_store =
        SqliteAgentStore::open(cas_root).with_context(|| "open agent store")?;
    let assigned: Vec<_> = task_store
        .list(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            matches!(t.status, TaskStatus::InProgress | TaskStatus::Blocked)
                && t.assignee.as_deref() == Some(worker_name)
        })
        .collect();
    let mut reset_count = 0usize;
    for mut t in assigned {
        // Same three steps as `cas_task_reset` (task_claiming.rs):
        //   1. Force-release any active lease (idempotent — Ok(false)
        //      when no lease exists).
        //   2. Set task.status = Open.
        //   3. Clear task.assignee.
        let _ = agent_store.release_lease_for_task(&t.id);
        t.status = TaskStatus::Open;
        t.assignee = None;
        t.updated_at = chrono::Utc::now();
        if task_store.update(&t).is_ok() {
            reset_count += 1;
        }
    }
    Ok(reset_count)
}

fn read_last_lines(path: &Path, tail: usize) -> Result<Vec<String>> {
    let f = std::fs::File::open(path)?;
    // cas-4513 correctness P2: delegates to the shared helper which
    // guards `tail == 0` against the unbounded-growth bug (empty ring
    // buffer + push_back would retain the entire file).
    Ok(collect_tail_lines(f, tail))
}

fn format_state_human(state: &WorkerLivenessState, ev: &WorkerEvidence) -> String {
    let mut s = format!("state: {}\n", state.label());
    // cas-4513 maintainability P3: render pid as a bare integer so
    // `cas factory is-wedged | grep pid | awk '{print $2}' | xargs kill`
    // actually works — Rust's `{:?}` would print `Some(4242)`.
    let pid_str = ev
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "<none>".into());
    s.push_str(&format!("  pid: {pid_str} (alive: {})\n", ev.pid_alive));
    if let Some(ref p) = ev.transcript_path {
        s.push_str(&format!("  transcript: {}\n", p.display()));
    } else {
        s.push_str("  transcript: <unresolved>\n");
    }
    match ev.transcript_mtime_age_secs {
        Some(age) => s.push_str(&format!("  transcript mtime age: {age}s\n")),
        None => s.push_str("  transcript mtime age: <unknown>\n"),
    }
    s.push_str(&format!(
        "  crash signature match: {}\n",
        ev.crash_signature_match
    ));
    match ev.worktree_edit_age_secs {
        Some(age) => s.push_str(&format!("  worktree recent-edit age: {age}s\n")),
        None => s.push_str("  worktree recent-edit age: <unknown>\n"),
    }
    s.push_str(&format!("  session: {}\n", ev.session_id));
    s
}

fn format_state_json(state: &WorkerLivenessState, ev: &WorkerEvidence) -> String {
    // cas-4513 adversarial P2: use serde_json so backslashes, control
    // characters, and any non-ASCII session_id / path bytes are escaped
    // correctly. The prior hand-rolled escape only handled `"` and
    // produced malformed JSON for paths or session ids containing
    // backslashes or control chars.
    let transcript = ev
        .transcript_path
        .as_ref()
        .map(|p| serde_json::Value::String(p.display().to_string()))
        .unwrap_or(serde_json::Value::Null);
    let body = serde_json::json!({
        "state": state.label(),
        "pid": ev.pid,
        "pid_alive": ev.pid_alive,
        "transcript_path": transcript,
        "transcript_mtime_age_secs": ev.transcript_mtime_age_secs,
        "crash_signature_match": ev.crash_signature_match,
        "worktree_edit_age_secs": ev.worktree_edit_age_secs,
        "session_id": ev.session_id,
    });
    body.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn classify_dead_when_pid_gone_and_second_signal_corroborates() {
        // cas-f781 AC c: Dead requires TWO independent signals to agree —
        // pid gone AND (transcript stale AND worktree not recently edited).
        for sig in [true, false] {
            let got = classify_from_evidence(false, Some(Duration::from_secs(5 * 60)), sig, None);
            assert_eq!(got, WorkerLivenessState::Dead, "sig={sig}");
        }
    }

    #[test]
    fn classify_unverified_when_pid_gone_but_transcript_still_fresh() {
        // cas-f781 core fix: a pid-only "gone" reading contradicted by a
        // transcript still being written in the last minute must NOT be
        // reported as Dead — that combination is exactly the stale/wrong
        // tracked-pid bug (the real worker is still alive and writing).
        // Report Unverified so an operator investigates before a caller
        // (e.g. a supervisor auto-reset) treats it as ground truth.
        for sig in [true, false] {
            let got = classify_from_evidence(false, Some(Duration::from_secs(5)), sig, None);
            assert_eq!(got, WorkerLivenessState::Unverified, "sig={sig}");
        }
    }

    #[test]
    fn classify_unverified_when_pid_gone_but_worktree_recently_edited() {
        // Same contradiction, corroborated by worktree activity instead of
        // transcript mtime — matches the bug report's concrete repro
        // ("fresh worktree edits, 20s-old transcript").
        let got = classify_from_evidence(
            false,
            Some(Duration::from_secs(5 * 60)),
            false,
            Some(Duration::from_secs(20)),
        );
        assert_eq!(got, WorkerLivenessState::Unverified);
    }

    #[test]
    fn classify_dead_when_no_corroborating_signals_available_at_all() {
        // No transcript resolved, no worktree resolved, pid gone: nothing
        // contradicts "dead", so Dead still fires — matches the
        // no-pid-registered case (classify_worker_no_pid_short_circuits_to_dead).
        let got = classify_from_evidence(false, None, true, None);
        assert_eq!(got, WorkerLivenessState::Dead);
    }

    #[test]
    fn classify_wedged_when_alive_fresh_and_signature_matches() {
        let got = classify_from_evidence(true, Some(Duration::from_secs(5)), true, None);
        assert_eq!(got, WorkerLivenessState::Wedged);
    }

    #[test]
    fn classify_alive_when_fresh_and_no_signature() {
        let got = classify_from_evidence(true, Some(Duration::from_secs(5)), false, None);
        assert_eq!(got, WorkerLivenessState::Alive);
    }

    #[test]
    fn classify_starved_when_alive_but_stale() {
        // Stale wins over signature: a crashed-but-not-touched-in-5min
        // worker is functionally hung, not wedged — the recovery playbook
        // is the same (SIGKILL + respawn) but the label matters for
        // operator triage.
        for sig in [true, false] {
            let got = classify_from_evidence(true, Some(Duration::from_secs(120)), sig, None);
            assert_eq!(got, WorkerLivenessState::Starved, "sig={sig}");
        }
    }

    #[test]
    fn classify_starved_when_no_mtime_available() {
        // File missing / mtime unreadable → treated as not-fresh.
        let got = classify_from_evidence(true, None, true, None);
        assert_eq!(got, WorkerLivenessState::Starved);
    }

    #[test]
    fn classify_state_exit_codes_are_pinned() {
        // cas-4513 AC: supervisor bash scripts branch on exit code.
        assert_eq!(WorkerLivenessState::Alive.exit_code(), 0);
        assert_eq!(WorkerLivenessState::Wedged.exit_code(), 1);
        assert_eq!(WorkerLivenessState::Starved.exit_code(), 2);
        assert_eq!(WorkerLivenessState::Dead.exit_code(), 3);
        assert_eq!(WorkerLivenessState::Unverified.exit_code(), 4);
    }

    #[test]
    fn worktree_recent_edit_age_detects_dirty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        let status = std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .current_dir(repo)
            .status()
            .expect("git init should run");
        assert!(status.success());
        std::fs::write(repo.join("touched.txt"), "hello").unwrap();
        let age = worktree_recent_edit_age(repo).expect("dirty file should be detected");
        assert!(
            age < Duration::from_secs(5),
            "expected fresh edit age, got {age:?}"
        );
    }

    #[test]
    fn worktree_recent_edit_age_none_when_not_a_git_repo() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(worktree_recent_edit_age(tmp.path()).is_none());
    }

    #[test]
    fn crash_signature_matches_bun_root_path() {
        // Evidence from cas-4513 discovery note: `/$bunfs/root` prefix
        // inside a JS stack frame is the strongest single signal.
        let transcript = r#"{"type":"assistant","text":"..."}
{"type":"tool_use","name":"Bash"}
{"error":"at createInstance (/$bunfs/root/src/entrypoints/cli.js:496:249)"}"#;
        assert!(has_crash_signature(
            Cursor::new(transcript),
            CRASH_SIGNATURE_TAIL_LINES
        ));
    }

    #[test]
    fn crash_signature_matches_literal_ink_guard_text() {
        // Supervisor's cas-4513 nit: the literal Ink invariant text is a
        // stronger signal than bundler paths. If this regresses, the whole
        // crash-screen detection weakens to a path-heuristic only.
        let transcript = "normal\n{\"error\":\"<Box> can't be nested inside <Text>\"}\nmore";
        assert!(has_crash_signature(
            Cursor::new(transcript),
            CRASH_SIGNATURE_TAIL_LINES
        ));
    }

    #[test]
    fn crash_signature_matches_ink_createelement() {
        let transcript = "normal line\nanother line\ncreateElement(\"ink-box\", {ref:V})";
        assert!(has_crash_signature(
            Cursor::new(transcript),
            CRASH_SIGNATURE_TAIL_LINES
        ));
    }

    #[test]
    fn crash_signature_no_match_on_clean_transcript() {
        let transcript = r#"{"type":"user","text":"hi"}
{"type":"assistant","text":"hello"}
{"type":"tool_use","name":"Read"}"#;
        assert!(!has_crash_signature(
            Cursor::new(transcript),
            CRASH_SIGNATURE_TAIL_LINES
        ));
    }

    #[test]
    fn crash_signature_ignores_old_lines_outside_tail_window() {
        // cas-4513 scope note: we only look at the LAST N lines. A crash
        // signature buried earlier in a long transcript should NOT fire
        // — the worker recovered from it.
        let mut lines: Vec<String> =
            vec!["createElement(\"ink-\")".to_string()];
        for i in 0..50 {
            lines.push(format!("{{\"msg\":\"line {i}\"}}"));
        }
        let body = lines.join("\n");
        assert!(!has_crash_signature(Cursor::new(body), 20));
    }

    #[test]
    fn classify_worker_orchestrator_threads_probe_fn() {
        // The orchestrating wrapper must actually call the injectable pid
        // probe (not hardcode a kill(0) call). Use a Cell to observe it.
        let called = std::cell::Cell::new(false);
        let probe = |_: u32| {
            called.set(true);
            true
        };
        let worktree_probe = |_: &Path| None::<Duration>;
        let (state, ev) = classify_worker(Some(1234), None, None, "ses", probe, worktree_probe);
        assert!(called.get(), "probe must be called when pid is Some");
        // no transcript → not fresh, crash=false, alive=true → Starved.
        assert_eq!(state, WorkerLivenessState::Starved);
        assert_eq!(ev.pid, Some(1234));
        assert!(ev.pid_alive);
        assert!(!ev.crash_signature_match);
        assert_eq!(ev.session_id, "ses");
    }

    #[test]
    fn classify_worker_no_pid_short_circuits_to_dead() {
        let probe = |_: u32| panic!("probe must not be called when pid is None");
        let worktree_probe = |_: &Path| None::<Duration>;
        let (state, ev) = classify_worker(None, None, None, "ses", probe, worktree_probe);
        assert_eq!(state, WorkerLivenessState::Dead);
        assert!(!ev.pid_alive);
    }

    #[test]
    fn classify_worker_threads_worktree_probe_only_when_clone_path_present() {
        // clone_path=None must short-circuit without invoking the probe —
        // mirrors the existing no-pid short-circuit contract for the pid
        // probe. cas-f781.
        let pid_probe = |_: u32| false;
        let worktree_probe = |_: &Path| panic!("worktree probe must not run without a clone_path");
        let (state, ev) = classify_worker(
            None,
            None,
            None,
            "ses",
            pid_probe,
            worktree_probe,
        );
        assert_eq!(state, WorkerLivenessState::Dead);
        assert_eq!(ev.worktree_edit_age_secs, None);
    }

    #[test]
    fn classify_worker_surfaces_worktree_evidence_when_clone_path_present() {
        let pid_probe = |_: u32| false;
        let worktree_probe = |_: &Path| Some(Duration::from_secs(20));
        let (state, ev) = classify_worker(
            None,
            None,
            Some(Path::new("/some/clone/path")),
            "ses",
            pid_probe,
            worktree_probe,
        );
        // pid gone, transcript unresolved (not fresh), but worktree edited
        // 20s ago (fresh) — contradiction → Unverified, not Dead.
        assert_eq!(state, WorkerLivenessState::Unverified);
        assert_eq!(ev.worktree_edit_age_secs, Some(20));
    }

    #[test]
    fn transcript_mtime_age_reads_recent_write() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("fresh.jsonl");
        std::fs::write(&path, b"{}").unwrap();
        let age = transcript_mtime_age(&path).expect("fresh file must have mtime");
        assert!(
            age < Duration::from_secs(5),
            "just-written file should be < 5s old, got {age:?}"
        );
    }

    #[test]
    fn transcript_mtime_age_none_for_missing_file() {
        let missing = Path::new("/tmp/does-not-exist-cas-4513");
        assert!(transcript_mtime_age(missing).is_none());
    }

    #[test]
    fn transcript_has_crash_signature_missing_file_is_false() {
        let missing = Path::new("/tmp/does-not-exist-cas-4513");
        assert!(!transcript_has_crash_signature(missing, 20));
    }

    #[test]
    fn format_state_human_surfaces_session_and_state() {
        let ev = WorkerEvidence {
            pid: Some(4242),
            pid_alive: true,
            transcript_path: Some(PathBuf::from("/p/a.jsonl")),
            transcript_mtime_age_secs: Some(7),
            crash_signature_match: true,
            worktree_edit_age_secs: Some(3),
            session_id: "ses-xyz".to_string(),
        };
        let out = format_state_human(&WorkerLivenessState::Wedged, &ev);
        assert!(out.contains("state: wedged"));
        assert!(out.contains("session: ses-xyz"));
        // cas-4513 maintainability P3: bare integer, not Debug `Some(4242)`.
        assert!(out.contains("pid: 4242"), "expected bare integer, got:\n{out}");
        assert!(!out.contains("Some(4242)"));
        assert!(out.contains("transcript: /p/a.jsonl"));
        assert!(out.contains("crash signature match: true"));
        assert!(out.contains("worktree recent-edit age: 3s"));
    }

    #[test]
    fn format_state_human_none_fields_render_placeholders() {
        // cas-4513 testing P3: the None branches for pid, transcript_path,
        // and transcript_mtime_age_secs must produce a legible placeholder
        // rather than nothing / a crash.
        let ev = WorkerEvidence {
            pid: None,
            pid_alive: false,
            transcript_path: None,
            transcript_mtime_age_secs: None,
            crash_signature_match: false,
            worktree_edit_age_secs: None,
            session_id: "ses-abc".to_string(),
        };
        let out = format_state_human(&WorkerLivenessState::Dead, &ev);
        assert!(out.contains("pid: <none>"));
        assert!(out.contains("transcript: <unresolved>"));
        assert!(out.contains("transcript mtime age: <unknown>"));
        assert!(out.contains("worktree recent-edit age: <unknown>"));
        assert!(out.contains("session: ses-abc"));
    }

    #[test]
    fn format_state_json_escapes_quotes_and_is_valid() {
        let ev = WorkerEvidence {
            pid: Some(4242),
            pid_alive: true,
            transcript_path: Some(PathBuf::from("/p/with\"quote.jsonl")),
            transcript_mtime_age_secs: None,
            crash_signature_match: false,
            worktree_edit_age_secs: None,
            session_id: "ses\"id".to_string(),
        };
        let out = format_state_json(&WorkerLivenessState::Alive, &ev);
        // Should be parseable as JSON.
        let parsed: serde_json::Value =
            serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(parsed["state"], "alive");
        assert_eq!(parsed["pid"], 4242);
        assert_eq!(parsed["session_id"], "ses\"id");
        assert_eq!(parsed["transcript_mtime_age_secs"], serde_json::Value::Null);
    }

    #[test]
    fn read_last_lines_returns_at_most_tail_count() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("long.jsonl");
        let body: String = (0..100).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, body).unwrap();
        let got = read_last_lines(&path, 5).unwrap();
        assert_eq!(got.len(), 5);
        assert_eq!(got[0], "line 95");
        assert_eq!(got[4], "line 99");
    }

    #[test]
    fn read_last_lines_short_file_returns_all() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("short.jsonl");
        std::fs::write(&path, "a\nb\nc\n").unwrap();
        let got = read_last_lines(&path, 100).unwrap();
        assert_eq!(got, vec!["a", "b", "c"]);
    }

    #[test]
    fn read_last_lines_tail_zero_returns_empty_not_unbounded() {
        // cas-4513 correctness P2: `tail = 0` used to grow the ring
        // buffer unboundedly (VecDeque::with_capacity(0) + len==0 guard
        // fires on every push). The shared helper now short-circuits.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("long.jsonl");
        let body: String = (0..10_000).map(|i| format!("line {i}\n")).collect();
        std::fs::write(&path, body).unwrap();
        let got = read_last_lines(&path, 0).unwrap();
        assert!(got.is_empty(), "tail=0 must return empty, not retain file");
    }

    #[test]
    fn has_crash_signature_tail_zero_is_false() {
        // cas-4513 testing P3: explicit coverage for the 0-line guard.
        let transcript = "<Box> can't be nested inside <Text>";
        assert!(!has_crash_signature(Cursor::new(transcript), 0));
    }

    #[test]
    fn collect_tail_lines_returns_bounded_window() {
        let body: String = (0..50).map(|i| format!("line {i}\n")).collect();
        let got = collect_tail_lines(Cursor::new(body), 3);
        assert_eq!(got, vec!["line 47", "line 48", "line 49"]);
    }

    #[test]
    fn kill_verdict_refuses_legacy_agent_without_force() {
        // cas-4513 adversarial P0: legacy agent (no pid_starttime) must
        // NOT auto-kill without --force. Use a PID guaranteed alive on
        // every Linux host: PID 1 (init).
        let verdict = kill_verdict(1, None, false);
        assert_eq!(verdict, KillVerdict::RefuseNoFingerprint);
    }

    #[test]
    fn kill_verdict_force_overrides_missing_fingerprint() {
        // Force path documented in the skill: legacy agent with operator-
        // confirmed PID can be killed via --force.
        let verdict = kill_verdict(1, None, true);
        assert_eq!(verdict, KillVerdict::Go);
    }

    #[test]
    fn kill_verdict_dead_pid_is_already_dead_regardless_of_force() {
        // Use u32::MAX-1 which is out-of-range → kill(pid,0) returns ESRCH.
        let verdict = kill_verdict(u32::MAX - 1, None, false);
        assert_eq!(verdict, KillVerdict::AlreadyDead);
        let verdict_force = kill_verdict(u32::MAX - 1, None, true);
        assert_eq!(verdict_force, KillVerdict::AlreadyDead);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_verdict_refuses_fingerprint_mismatch() {
        // cas-4513 adversarial P0: a live PID with the wrong starttime
        // is treated as a recycled PID and refused (the core protection).
        // PID 1 on Linux has some real starttime; passing 0 guarantees mismatch.
        let verdict = kill_verdict(1, Some(0), false);
        assert_eq!(verdict, KillVerdict::RefuseFingerprintMismatch);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn kill_verdict_go_on_fingerprint_match_self() {
        // Use our own pid + our own starttime — must classify Go.
        let my_pid = std::process::id();
        let my_starttime =
            crate::mcp::daemon::read_pid_starttime(my_pid).expect("self should have starttime");
        let verdict = kill_verdict(my_pid, Some(my_starttime), false);
        assert_eq!(verdict, KillVerdict::Go);
    }

    #[test]
    fn format_state_json_handles_backslash_and_control_chars() {
        // cas-4513 adversarial P2: the old hand-rolled escaper only
        // handled `"`; a path with `\` or a session_id with `\n` produced
        // malformed JSON. serde_json handles all of these.
        let ev = WorkerEvidence {
            pid: Some(4242),
            pid_alive: true,
            transcript_path: Some(PathBuf::from("/p/back\\slash\"quote.jsonl")),
            transcript_mtime_age_secs: None,
            crash_signature_match: false,
            worktree_edit_age_secs: None,
            // Newline + backslash inside session_id — worst-case.
            session_id: "ses\nfoo\\bar".to_string(),
        };
        let out = format_state_json(&WorkerLivenessState::Alive, &ev);
        // Parse round-trip. If escaping is wrong, this panics with a clear
        // error — catching any regression back to the hand-rolled path.
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(parsed["session_id"], "ses\nfoo\\bar");
        assert_eq!(parsed["transcript_path"], "/p/back\\slash\"quote.jsonl");
    }

    // -------------------------------------------------------------------
    // cas-f781: process-table resolution (AC a) + post-kill lease gating
    // (AC b).
    // -------------------------------------------------------------------

    struct FakeProcessTable {
        entries: std::collections::HashMap<u32, (Option<Vec<u8>>, Option<Vec<u8>>)>,
    }

    impl ProcessTable for FakeProcessTable {
        fn pids(&self) -> Vec<u32> {
            self.entries.keys().copied().collect()
        }
        fn cmdline(&self, pid: u32) -> Option<Vec<u8>> {
            self.entries.get(&pid).and_then(|e| e.0.clone())
        }
        fn environ(&self, pid: u32) -> Option<Vec<u8>> {
            self.entries.get(&pid).and_then(|e| e.1.clone())
        }
    }

    #[test]
    fn agent_name_from_cmdline_extracts_flag_value() {
        let cmdline = b"claude\0--dangerously-skip-permissions\0--agent-name\0hv-live\0";
        assert_eq!(
            agent_name_from_cmdline(cmdline),
            Some("hv-live".to_string())
        );
    }

    #[test]
    fn agent_name_from_cmdline_tolerates_nice_wrapper_prefix() {
        // maybe_wrap_with_nice (cas-pty) may prepend `nice -n <N>` — the
        // flag search must not assume a fixed argv position.
        let cmdline = b"nice\0-n\010\0claude\0--agent-name\0hv-live\0";
        assert_eq!(
            agent_name_from_cmdline(cmdline),
            Some("hv-live".to_string())
        );
    }

    #[test]
    fn agent_name_from_cmdline_none_without_flag() {
        let cmdline = b"cas\0serve\0--foreground\0";
        assert_eq!(agent_name_from_cmdline(cmdline), None);
    }

    #[test]
    fn agent_name_from_environ_extracts_codex_env_var() {
        // Codex workers carry identity only in env (no --agent-name in
        // argv) — cas-f781 investigation.
        let environ = b"PATH=/usr/bin\0CAS_AGENT_NAME=hv-live\0CAS_AGENT_ROLE=worker\0";
        assert_eq!(
            agent_name_from_environ(environ),
            Some("hv-live".to_string())
        );
    }

    #[test]
    fn agent_name_from_environ_none_without_var() {
        let environ = b"PATH=/usr/bin\0HOME=/root\0";
        assert_eq!(agent_name_from_environ(environ), None);
    }

    #[test]
    fn find_worker_pid_prefers_live_agent_name_match_over_unrelated_process() {
        let mut entries = std::collections::HashMap::new();
        // An unrelated process (e.g. the tracked stale child pid from the
        // agent store) with no agent-name of its own.
        entries.insert(9999, (Some(b"cas\0serve\0".to_vec()), None));
        // The real worker: claude spawned with --agent-name hv-live.
        entries.insert(
            4242,
            (
                Some(
                    b"claude\0--dangerously-skip-permissions\0--agent-name\0hv-live\0".to_vec(),
                ),
                None,
            ),
        );
        let table = FakeProcessTable { entries };
        assert_eq!(find_worker_pid(&table, "hv-live"), Some(4242));
    }

    #[test]
    fn find_worker_pid_matches_codex_via_environ() {
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            5555,
            (
                Some(b"codex\0exec\0".to_vec()),
                Some(b"PATH=/usr/bin\0CAS_AGENT_NAME=hv-live\0".to_vec()),
            ),
        );
        let table = FakeProcessTable { entries };
        assert_eq!(find_worker_pid(&table, "hv-live"), Some(5555));
    }

    #[test]
    fn find_worker_pid_none_when_no_process_matches() {
        let mut entries = std::collections::HashMap::new();
        entries.insert(1, (Some(b"init\0".to_vec()), None));
        let table = FakeProcessTable { entries };
        assert_eq!(find_worker_pid(&table, "hv-live"), None);
    }

    /// A `ProcessTable` whose `pids()` returns entries in an EXPLICIT,
    /// caller-controlled order — `FakeProcessTable`'s `HashMap`-backed
    /// `pids()` has unspecified iteration order, which can't reliably
    /// reproduce "the wrong candidate is scanned first" (cas-a91b).
    struct OrderedFakeProcessTable {
        order: Vec<u32>,
        entries: std::collections::HashMap<u32, (Option<Vec<u8>>, Option<Vec<u8>>)>,
    }

    impl ProcessTable for OrderedFakeProcessTable {
        fn pids(&self) -> Vec<u32> {
            self.order.clone()
        }
        fn cmdline(&self, pid: u32) -> Option<Vec<u8>> {
            self.entries.get(&pid).and_then(|e| e.0.clone())
        }
        fn environ(&self, pid: u32) -> Option<Vec<u8>> {
            self.entries.get(&pid).and_then(|e| e.1.clone())
        }
    }

    #[test]
    fn find_worker_pid_prefers_cmdline_match_over_environ_match_regardless_of_scan_order() {
        // cas-a91b P1: CAS_AGENT_NAME is inherited by EVERY descendant of the
        // worker (its `cas serve` child, git, cargo, ...) — an environ-only
        // match is not proof of being the actual leader, unlike argv, which
        // is never copied to children. Simulate the exact failure mode: a
        // descendant (environ match only) is enumerated BEFORE the real
        // leader (cmdline match) — proving the two-pass priority fix picks
        // the leader regardless of scan order, where the pre-fix single-pass
        // `.find()` would have nondeterministically returned the descendant.
        let mut entries = std::collections::HashMap::new();
        entries.insert(
            9999,
            (
                Some(b"cas\0serve\0".to_vec()),
                Some(b"PATH=/usr/bin\0CAS_AGENT_NAME=hv-live\0".to_vec()),
            ),
        );
        entries.insert(
            4242,
            (
                Some(b"claude\0--dangerously-skip-permissions\0--agent-name\0hv-live\0".to_vec()),
                None,
            ),
        );
        let table = OrderedFakeProcessTable {
            order: vec![9999, 4242], // descendant (environ match) scanned FIRST
            entries,
        };
        assert_eq!(
            find_worker_pid(&table, "hv-live"),
            Some(4242),
            "the cmdline (leader) match must win even though the environ-matching \
             descendant was scanned first"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_group_leader_pid_converts_descendant_to_actual_leader() {
        // cas-a91b: prove getpgid() correctly walks a descendant pid back to
        // its real process-group leader — the fix that makes killpg safe to
        // call on whatever find_worker_pid resolved, even when that's a
        // descendant rather than the leader itself. Spawn a detached leader
        // (own session/group, distinct from the `cargo test` process group)
        // whose script backgrounds a child in the SAME group, then confirm
        // resolve_group_leader_pid(child_pid) == leader_pid.
        use std::os::unix::process::CommandExt;

        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("child.pid");
        let script = format!("sleep 5 & echo $! > {} ; wait", pidfile.display());

        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&script);
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut leader = cmd.spawn().expect("spawn detached leader");
        let leader_pid = leader.id();

        let mut child_pid: Option<u32> = None;
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(p) = s.trim().parse::<u32>() {
                    child_pid = Some(p);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let child_pid = child_pid.expect("background child pid should appear");

        assert_eq!(
            resolve_group_leader_pid(child_pid),
            Some(leader_pid),
            "a descendant's group leader must resolve to the actual session leader"
        );
        assert_eq!(
            resolve_group_leader_pid(leader_pid),
            Some(leader_pid),
            "the leader's own group leader is itself (pid == pgid via setsid())"
        );

        // Clean up: killpg the real group so nothing outlives the test.
        let _ = send_sigkill(leader_pid);
        let _ = leader.wait();
    }

    #[cfg(unix)]
    #[test]
    fn send_sigkill_refuses_esrch_when_target_pid_still_alive() {
        // cas-a91b P1: killpg() on an ordinary (non-leader) pid returns
        // ESRCH because no process group has that id — but if the
        // underlying process is still alive, blindly treating ESRCH as
        // "already dead" silently no-ops the kill while the caller believes
        // it succeeded. This is the exact destructive path: a live worker's
        // task lease would then get reset out from under it. Use a plain
        // child process (NOT a session/group leader — it stays in this
        // test's own process group) as the "wrong pid resolved" stand-in.
        let mut child = std::process::Command::new("sleep")
            .arg("5")
            .spawn()
            .expect("spawn plain child");
        let pid = child.id();
        assert!(crate::mcp::daemon::pid_alive(pid));

        let result = send_sigkill(pid);
        assert!(
            result.is_err(),
            "send_sigkill must refuse (return Err), not silently succeed, when killpg(pid) \
             returns ESRCH but pid is still alive: {result:?}"
        );

        // This test process is the child's real parent — kill + reap directly
        // rather than relying on the (deliberately refused) send_sigkill.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGKILL);
        }
        let _ = child.wait();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn verify_death_treats_zombie_as_confirmed_dead() {
        // cas-a91b P3: a killed process GROUP LEADER becomes a zombie under
        // its original parent (not `cas factory kill`) — pid_alive
        // (kill(pid,0)) reports a zombie as alive, since its /proc entry
        // persists until reaped. Without zombie detection, verify_death
        // would time out (200ms) and return false for an effectively-dead
        // worker, leaving its task stuck InProgress.
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("spawn short-lived child");
        let pid = child.id();
        // Give it a moment to exit — a zombie exists once the process exits
        // but before THIS process (its parent) reaps it via wait().
        std::thread::sleep(Duration::from_millis(100));
        assert!(
            pid_is_zombie(pid),
            "child should be a zombie by now (exited, not yet reaped)"
        );
        assert!(
            verify_death(pid),
            "a zombie must be treated as confirmed-dead, not timed-out-alive"
        );
        let _ = child.wait(); // reap so no zombie leaks past the test
    }

    #[test]
    fn is_zombie_state_parses_z_state_from_stat_line() {
        // Pure-function coverage independent of a real /proc round-trip.
        // Synthetic stat line: "pid (comm) state ppid ...".
        assert!(is_zombie_state("1234 (sh) Z 1 1234 1234 0 -1 ..."));
        assert!(!is_zombie_state("1234 (sh) R 1 1234 1234 0 -1 ..."));
        assert!(!is_zombie_state("1234 (sh) S 1 1234 1234 0 -1 ..."));
    }

    #[test]
    fn is_zombie_state_handles_comm_with_parens_and_spaces() {
        // comm can itself contain spaces/parens — must split on the LAST
        // `)`, same caveat as daemon::parse_starttime_from_stat.
        assert!(is_zombie_state("1234 (my (weird) proc) Z 1 1234 1234 0 -1 ..."));
    }

    #[test]
    fn pick_kill_pid_prefers_resolved_over_stale_tracked_pid() {
        // cas-f781 AC a: a live process-table match must win over a stale
        // tracked child pid, even though the tracked pid also exists.
        assert_eq!(pick_kill_pid(Some(9999), Some(4242)), Some(4242));
    }

    #[test]
    fn pick_kill_pid_falls_back_to_tracked_when_no_scan_match() {
        assert_eq!(pick_kill_pid(Some(9999), None), Some(9999));
    }

    #[test]
    fn pick_kill_pid_none_when_nothing_available() {
        assert_eq!(pick_kill_pid(None, None), None);
    }

    #[test]
    fn decide_post_kill_action_resets_when_already_dead() {
        assert!(decide_post_kill_action(&KillVerdict::AlreadyDead, false));
    }

    #[test]
    fn decide_post_kill_action_resets_only_if_death_confirmed_after_go() {
        assert!(decide_post_kill_action(&KillVerdict::Go, true));
        assert!(!decide_post_kill_action(&KillVerdict::Go, false));
    }

    #[test]
    fn decide_post_kill_action_never_resets_on_refused_kill() {
        // cas-f781 AC b: a still-alive process — kill refused, never
        // attempted — must never have its lease reset out from under it.
        assert!(!decide_post_kill_action(
            &KillVerdict::RefuseFingerprintMismatch,
            false
        ));
        assert!(!decide_post_kill_action(
            &KillVerdict::RefuseNoFingerprint,
            false
        ));
        // Even if the process happened to die of unrelated causes right
        // after the refusal, the gate keys only off the verdict for the
        // refuse cases — a refused kill is never a green light to reset.
        assert!(!decide_post_kill_action(
            &KillVerdict::RefuseFingerprintMismatch,
            true
        ));
    }

    #[cfg(unix)]
    #[test]
    fn send_sigkill_terminates_the_whole_process_group_not_just_the_leader() {
        // cas-f781 AC a: prove `send_sigkill` uses killpg semantics — a
        // bare `kill(leader_pid)` would leave a backgrounded sibling in the
        // same process group alive. Spawn a detached session leader (own
        // pgid, distinct from the `cargo test` process group) whose script
        // backgrounds a second long-lived process in the same group, then
        // confirm BOTH die from a single send_sigkill(leader_pid) call.
        use std::os::unix::process::CommandExt;

        let tmp = tempfile::tempdir().unwrap();
        let pidfile = tmp.path().join("child.pid");
        let script = format!("sleep 30 & echo $! > {} ; wait", pidfile.display());

        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c").arg(&script);
        // SAFETY: setsid() is async-signal-safe; standard pattern for
        // detaching a test child into its own session/process group so
        // this test can't touch the surrounding `cargo test` process group.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut leader = cmd.spawn().expect("spawn detached leader");
        let leader_pid = leader.id();

        let mut child_pid: Option<u32> = None;
        for _ in 0..50 {
            if let Ok(s) = std::fs::read_to_string(&pidfile) {
                if let Ok(p) = s.trim().parse::<u32>() {
                    child_pid = Some(p);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let child_pid = child_pid.expect("background child pid should appear");

        assert!(crate::mcp::daemon::pid_alive(leader_pid));
        assert!(crate::mcp::daemon::pid_alive(child_pid));

        send_sigkill(leader_pid).expect("killpg should succeed");

        // `leader` is a direct child of THIS test process, so once killed
        // it's a zombie (kill(pid,0)/pid_alive would report it "alive"
        // forever) until its parent reaps it — use `wait()` rather than
        // `verify_death` to confirm the leader specifically.
        let status = leader.wait().expect("wait on killed leader");
        assert!(!status.success(), "leader should have been SIGKILLed, not exited cleanly");

        // `child_pid` (the backgrounded grandchild) is NOT a child of this
        // test process — it's reparented away once the shell dies, so
        // `pid_alive` polling is the right liveness check here, exactly as
        // `execute_kill` uses it in production.
        assert!(
            verify_death(child_pid),
            "background sibling in the same process group should ALSO die via killpg \
             — a bare kill(leader_pid) would leave it running"
        );
    }
}
