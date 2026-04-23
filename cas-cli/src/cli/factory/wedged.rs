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
//!   Dead by combining PID liveness, transcript mtime, and a content grep for
//!   the Bun/React-Ink crash-screen signature. Exits with a differentiated
//!   code so supervisor skills can script.
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
/// Small because the signature — React-Ink error-handler dumping bundle source
/// — spans at most a dozen lines; scanning more just costs I/O.
pub(crate) const CRASH_SIGNATURE_TAIL_LINES: usize = 20;

/// Evidence collected by [`classify_worker`], surfaced verbatim in
/// `cas factory is-wedged` output so a supervisor can audit the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorkerEvidence {
    pub pid: Option<u32>,
    pub pid_alive: bool,
    pub transcript_path: Option<PathBuf>,
    pub transcript_mtime_age_secs: Option<u64>,
    pub crash_signature_match: bool,
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
    /// PID gone. The cleanup path is the same as SIGKILL-after-wedge
    /// (release lease, prune worktree). Not an error — just means the worker
    /// already exited.
    Dead,
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
        }
    }

    pub(crate) fn label(&self) -> &'static str {
        match self {
            WorkerLivenessState::Alive => "alive",
            WorkerLivenessState::Wedged => "wedged",
            WorkerLivenessState::Starved => "starved",
            WorkerLivenessState::Dead => "dead",
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
) -> WorkerLivenessState {
    if !pid_alive {
        return WorkerLivenessState::Dead;
    }
    let fresh = transcript_mtime_age
        .map(|age| age < TRANSCRIPT_FRESH_WINDOW)
        .unwrap_or(false);
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

/// Grep the last [`CRASH_SIGNATURE_TAIL_LINES`] lines of `reader` for any of
/// [`CRASH_SIGNATURE_NEEDLES`]. Takes `Read` so tests can point at a
/// `std::io::Cursor<Vec<u8>>` without touching the filesystem. Large
/// transcripts (thousands of lines) are fine — we only retain a bounded tail
/// window in memory.
pub(crate) fn has_crash_signature<R: Read>(reader: R, tail_lines: usize) -> bool {
    if tail_lines == 0 {
        return false;
    }
    let bufread = BufReader::new(reader);
    let mut ring: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(tail_lines);
    for line in bufread.lines().map_while(Result::ok) {
        if ring.len() == tail_lines {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    ring.iter()
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
pub(crate) fn classify_worker<F>(
    pid: Option<u32>,
    transcript_path: Option<&Path>,
    session_id: &str,
    pid_alive_probe: F,
) -> (WorkerLivenessState, WorkerEvidence)
where
    F: FnOnce(u32) -> bool,
{
    let pid_alive = pid.map(pid_alive_probe).unwrap_or(false);
    let (age_opt, sig) = match transcript_path {
        Some(p) => (
            transcript_mtime_age(p),
            transcript_has_crash_signature(p, CRASH_SIGNATURE_TAIL_LINES),
        ),
        None => (None, false),
    };
    let state = classify_from_evidence(pid_alive, age_opt, sig);
    let evidence = WorkerEvidence {
        pid,
        pid_alive,
        transcript_path: transcript_path.map(PathBuf::from),
        transcript_mtime_age_secs: age_opt.map(|d| d.as_secs()),
        crash_signature_match: sig,
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
        TranscriptResolution::Synthesized(_) => None,
    };
    Ok(ResolvedWorker {
        name: worker_name.to_string(),
        pid,
        clone_path,
        session_id,
        transcript_path,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedWorker {
    pub name: String,
    pub pid: Option<u32>,
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
    let w = resolve_worker(cas_root, worker)?;
    let (state, evidence) = classify_worker(
        w.pid,
        w.transcript_path.as_deref(),
        &w.session_id,
        crate::mcp::daemon::pid_alive,
    );
    if json {
        println!("{}", format_state_json(&state, &evidence));
    } else {
        println!("{}", format_state_human(&state, &evidence));
    }
    std::process::exit(state.exit_code());
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

/// `cas factory kill <worker>`: SIGKILL the worker process and release any
/// active CAS lease. Idempotent — already-dead worker still runs the cleanup.
pub(crate) fn execute_kill(
    cas_root: Option<&Path>,
    worker: &str,
) -> Result<()> {
    let cas_root =
        cas_root.ok_or_else(|| anyhow!("--cas-root required or run from a CAS project"))?;
    let w = resolve_worker(cas_root, worker)?;
    let mut summary = Vec::<String>::new();

    // 1. Signal. SIGKILL explicitly — observed that SIGTERM doesn't exit
    //    cleanly on the Bun-wedged process (nimble-tiger-98 2026-04-23).
    match w.pid {
        Some(pid) if crate::mcp::daemon::pid_alive(pid) => {
            match send_sigkill(pid) {
                Ok(()) => summary.push(format!("SIGKILL delivered to pid {pid}")),
                Err(e) => summary.push(format!("SIGKILL failed for pid {pid}: {e}")),
            }
        }
        Some(pid) => summary.push(format!("pid {pid} already dead — skipping SIGKILL")),
        None => summary.push("worker has no PID recorded — skipping SIGKILL".into()),
    }

    // 2. Release any leased tasks. Best-effort: a failure here shouldn't
    //    block the kill operation — supervisor can re-run `task action=reset`
    //    manually. Present every action in the summary so an operator can
    //    see what landed and what didn't.
    match release_worker_leases(cas_root, &w.name) {
        Ok(n) if n > 0 => summary.push(format!("reset {n} task lease(s) held by {}", w.name)),
        Ok(_) => summary.push("no active leases to release".into()),
        Err(e) => summary.push(format!("lease release failed: {e}")),
    }

    println!("kill-worker `{}` completed:", w.name);
    for line in summary {
        println!("  - {line}");
    }
    Ok(())
}

fn send_sigkill(pid: u32) -> Result<()> {
    // SAFETY: libc::kill with SIGKILL has no side effects on this process.
    // ESRCH (process already gone) is treated as success by the caller.
    let rc = unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
    if rc == 0 {
        return Ok(());
    }
    let errno = std::io::Error::last_os_error();
    if errno.raw_os_error() == Some(libc::ESRCH) {
        return Ok(());
    }
    Err(errno.into())
}

fn release_worker_leases(cas_root: &Path, worker_name: &str) -> Result<usize> {
    use cas_store::{AgentStore, SqliteAgentStore, SqliteTaskStore, TaskStore};
    use cas_types::TaskStatus;
    let task_store = SqliteTaskStore::open(cas_root).with_context(|| "open task store")?;
    let agent_store = SqliteAgentStore::open(cas_root)
        .with_context(|| "open agent store")?;
    let in_progress: Vec<_> = task_store
        .list(None)
        .unwrap_or_default()
        .into_iter()
        .filter(|t| {
            t.status == TaskStatus::InProgress
                && t.assignee.as_deref() == Some(worker_name)
        })
        .collect();
    let mut released = 0usize;
    for t in in_progress {
        // Force-release the lease (no ownership check — we're recovering a
        // wedged worker). The task stays InProgress; supervisor can re-reset
        // via `mcp__cas__task action=reset` if they want to re-open it.
        // That's the same semantic contract as the MCP `cas_task_reset`
        // helper's first step (see task_claiming.rs:435).
        if agent_store
            .release_lease_for_task(&t.id)
            .unwrap_or(false)
        {
            released += 1;
        }
    }
    Ok(released)
}

fn read_last_lines(path: &Path, tail: usize) -> Result<Vec<String>> {
    let f = std::fs::File::open(path)?;
    let buf = BufReader::new(f);
    let mut ring: std::collections::VecDeque<String> =
        std::collections::VecDeque::with_capacity(tail);
    for line in buf.lines().map_while(Result::ok) {
        if ring.len() == tail {
            ring.pop_front();
        }
        ring.push_back(line);
    }
    Ok(ring.into_iter().collect())
}

fn format_state_human(state: &WorkerLivenessState, ev: &WorkerEvidence) -> String {
    let mut s = format!("state: {}\n", state.label());
    s.push_str(&format!("  pid: {:?} (alive: {})\n", ev.pid, ev.pid_alive));
    if let Some(ref p) = ev.transcript_path {
        s.push_str(&format!("  transcript: {}\n", p.display()));
    } else {
        s.push_str("  transcript: <unresolved>\n");
    }
    if let Some(age) = ev.transcript_mtime_age_secs {
        s.push_str(&format!("  transcript mtime age: {age}s\n"));
    }
    s.push_str(&format!(
        "  crash signature match: {}\n",
        ev.crash_signature_match
    ));
    s.push_str(&format!("  session: {}\n", ev.session_id));
    s
}

fn format_state_json(state: &WorkerLivenessState, ev: &WorkerEvidence) -> String {
    // Keep the JSON shape flat + stable; no external serde derive so the
    // schema doesn't drift from what operator scripts depend on.
    let transcript = ev
        .transcript_path
        .as_ref()
        .map(|p| format!("\"{}\"", p.display().to_string().replace('"', "\\\"")))
        .unwrap_or_else(|| "null".into());
    let age = ev
        .transcript_mtime_age_secs
        .map(|a| a.to_string())
        .unwrap_or_else(|| "null".into());
    let pid = ev
        .pid
        .map(|p| p.to_string())
        .unwrap_or_else(|| "null".into());
    format!(
        "{{\"state\":\"{}\",\"pid\":{},\"pid_alive\":{},\"transcript_path\":{},\"transcript_mtime_age_secs\":{},\"crash_signature_match\":{},\"session_id\":\"{}\"}}",
        state.label(),
        pid,
        ev.pid_alive,
        transcript,
        age,
        ev.crash_signature_match,
        ev.session_id.replace('"', "\\\"")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn classify_dead_when_pid_gone_regardless_of_transcript() {
        // cas-4513 AC: Dead supersedes everything — the transcript content
        // and freshness stop mattering once the process is gone.
        for fresh in [true, false] {
            for sig in [true, false] {
                let age = if fresh {
                    Some(Duration::from_secs(5))
                } else {
                    Some(Duration::from_secs(5 * 60))
                };
                let got = classify_from_evidence(false, age, sig);
                assert_eq!(
                    got,
                    WorkerLivenessState::Dead,
                    "fresh={fresh} sig={sig}"
                );
            }
        }
    }

    #[test]
    fn classify_wedged_when_alive_fresh_and_signature_matches() {
        let got = classify_from_evidence(true, Some(Duration::from_secs(5)), true);
        assert_eq!(got, WorkerLivenessState::Wedged);
    }

    #[test]
    fn classify_alive_when_fresh_and_no_signature() {
        let got = classify_from_evidence(true, Some(Duration::from_secs(5)), false);
        assert_eq!(got, WorkerLivenessState::Alive);
    }

    #[test]
    fn classify_starved_when_alive_but_stale() {
        // Stale wins over signature: a crashed-but-not-touched-in-5min
        // worker is functionally hung, not wedged — the recovery playbook
        // is the same (SIGKILL + respawn) but the label matters for
        // operator triage.
        for sig in [true, false] {
            let got = classify_from_evidence(true, Some(Duration::from_secs(120)), sig);
            assert_eq!(got, WorkerLivenessState::Starved, "sig={sig}");
        }
    }

    #[test]
    fn classify_starved_when_no_mtime_available() {
        // File missing / mtime unreadable → treated as not-fresh.
        let got = classify_from_evidence(true, None, true);
        assert_eq!(got, WorkerLivenessState::Starved);
    }

    #[test]
    fn classify_state_exit_codes_are_pinned() {
        // cas-4513 AC: supervisor bash scripts branch on exit code.
        assert_eq!(WorkerLivenessState::Alive.exit_code(), 0);
        assert_eq!(WorkerLivenessState::Wedged.exit_code(), 1);
        assert_eq!(WorkerLivenessState::Starved.exit_code(), 2);
        assert_eq!(WorkerLivenessState::Dead.exit_code(), 3);
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
        let (state, ev) = classify_worker(Some(1234), None, "ses", probe);
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
        let (state, ev) = classify_worker(None, None, "ses", probe);
        assert_eq!(state, WorkerLivenessState::Dead);
        assert!(!ev.pid_alive);
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
            session_id: "ses-xyz".to_string(),
        };
        let out = format_state_human(&WorkerLivenessState::Wedged, &ev);
        assert!(out.contains("state: wedged"));
        assert!(out.contains("session: ses-xyz"));
        assert!(out.contains("pid: Some(4242)"));
        assert!(out.contains("transcript: /p/a.jsonl"));
        assert!(out.contains("crash signature match: true"));
    }

    #[test]
    fn format_state_json_escapes_quotes_and_is_valid() {
        let ev = WorkerEvidence {
            pid: Some(4242),
            pid_alive: true,
            transcript_path: Some(PathBuf::from("/p/with\"quote.jsonl")),
            transcript_mtime_age_secs: None,
            crash_signature_match: false,
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
}
