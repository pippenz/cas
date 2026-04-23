use crate::mcp::daemon::*;
use std::path::PathBuf;
use tempfile::TempDir;

use crate::cloud::SyncQueue;
use crate::store::SqliteStore;
use crate::store::init_cas_dir;
use cas_types::Session;

#[test]
fn test_activity_tracker() {
    let tracker = ActivityTracker::new(5);
    assert!(tracker.idle_seconds() < 1);
    assert!(!tracker.is_idle());

    tracker.touch();
    assert!(tracker.idle_seconds() < 1);
}

#[test]
fn test_daemon_config_conversion() {
    let config = EmbeddedDaemonConfig {
        cas_root: PathBuf::from("/tmp/cas"),
        maintenance_interval_secs: 1800,
        ..Default::default()
    };

    let daemon_config = config.to_daemon_config();
    assert_eq!(daemon_config.interval_minutes, 30);
    assert_eq!(daemon_config.cas_root, PathBuf::from("/tmp/cas"));
}

// =========================================================================
// EmbeddedDaemonStatus tests
// =========================================================================

#[test]
fn test_embedded_daemon_status_default() {
    let status = EmbeddedDaemonStatus::default();
    assert!(!status.running);
    assert!(status.last_maintenance.is_none());
    assert!(status.last_cloud_sync.is_none());
    assert!(status.next_maintenance.is_none());
    assert_eq!(status.observations_processed, 0);
    assert_eq!(status.decay_applied, 0);
    assert_eq!(status.cloud_sync_pending, 0);
    assert!(!status.cloud_sync_available);
    assert_eq!(status.cloud_items_pushed, 0);
    assert_eq!(status.cloud_items_pulled, 0);
    assert_eq!(status.idle_seconds, 0);
    assert!(!status.is_idle);
    assert!(status.last_error.is_none());
}

// =========================================================================
// EmbeddedDaemonConfig tests
// =========================================================================

#[test]
fn test_embedded_daemon_config_default() {
    let config = EmbeddedDaemonConfig::default();
    assert_eq!(config.maintenance_interval_secs, 30 * 60);
    assert_eq!(config.cloud_sync_interval_secs, 60);
    assert_eq!(config.min_idle_secs, 60);
    assert!(config.apply_decay);
    assert!(config.process_observations);
    assert!(config.cloud_sync_enabled);
    assert_eq!(config.batch_size, 20);
}

#[test]
fn test_get_sessions_for_sync_uses_cas_root_directory_path() {
    let temp = TempDir::new().expect("temp dir");
    let cas_root = init_cas_dir(temp.path()).expect("init cas dir");

    let sqlite_store = SqliteStore::open(&cas_root).expect("open sqlite store");

    let mut session = Session::new(
        "session-for-sync".to_string(),
        temp.path().display().to_string(),
        Some("default".to_string()),
    );
    session.started_at = chrono::Utc::now() - chrono::Duration::hours(1);
    sqlite_store
        .start_session(&session)
        .expect("insert session");

    let queue = SyncQueue::open(&cas_root).expect("open sync queue");
    queue.init().expect("init sync queue");

    let sessions = super::get_sessions_for_sync(&cas_root, &queue);
    assert_eq!(sessions.len(), 1, "expected one session from sqlite");
    assert_eq!(sessions[0].session_id, "session-for-sync");
}

// =========================================================================
// Agent heartbeat liveness gate (EPIC cas-9508 / cas-2749)
// =========================================================================
//
// Regression coverage: the shared `cas serve` daemon must not keep pinging
// `store.heartbeat()` for an agent whose Claude Code client has died. Without
// this gate, a crashed CC client (e.g. Bun/React-Ink unhandled-rejection
// zombie) keeps the agent's `last_heartbeat` fresh forever and supervisors see
// dead workers as "active" in `worker_status`.

#[cfg(unix)]
#[test]
fn pid_alive_self_is_live() {
    let our_pid = std::process::id();
    assert!(
        crate::mcp::daemon::pid_alive(our_pid),
        "our own PID must report alive"
    );
}

#[cfg(unix)]
#[test]
fn pid_alive_dead_child_is_dead() {
    // Spawn a short-lived child, wait for it to exit, then confirm its PID
    // is reported dead. We poll briefly because `wait()` reaps but the exact
    // timing of ESRCH visibility can lag on some kernels under load — using
    // `kill -0` via pid_alive gives us the same signal CAS uses in prod.
    let mut child = std::process::Command::new("/bin/true")
        .spawn()
        .expect("spawn /bin/true");
    let pid = child.id();
    let _ = child.wait().expect("wait for child");

    // After reap, the PID should report dead. Allow a couple polls for the
    // kernel to flip ESRCH on heavily loaded CI.
    let mut dead = false;
    for _ in 0..20 {
        if !crate::mcp::daemon::pid_alive(pid) {
            dead = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        dead,
        "reaped child PID {pid} must report dead (ESRCH) after wait"
    );
}

#[cfg(unix)]
#[test]
fn pid_alive_obviously_invalid_pid_is_dead() {
    // PID space on Linux tops out at /proc/sys/kernel/pid_max (typically
    // 4_194_304). A PID near u32::MAX cannot be a live process. This guards
    // the liveness gate from silently treating out-of-range PIDs as live,
    // which would defeat the whole cas-2749 fix.
    assert!(
        !crate::mcp::daemon::pid_alive(u32::MAX - 1),
        "out-of-range PID must report dead"
    );
}

// =========================================================================
// PID-reuse-resistant fingerprint via /proc/<pid>/stat starttime (cas-ea46)
// =========================================================================
//
// Regression coverage: `pid_alive(pid)` alone cannot distinguish the original
// Claude Code client from a kernel-recycled occupant of the same PID slot.
// The liveness gate now pairs `pid_alive` with `read_pid_starttime` (field
// 22 of /proc/<pid>/stat) via `pid_matches_fingerprint` so PID reuse no
// longer bypasses the gate.

#[cfg(target_os = "linux")]
#[test]
fn read_pid_starttime_self_is_stable() {
    // Our own process starttime must parse and return a positive u64.
    // Repeated reads within a single test must yield the same value —
    // starttime is set at exec and does not drift.
    let our_pid = std::process::id();
    let first = crate::mcp::daemon::read_pid_starttime(our_pid)
        .expect("read_pid_starttime must succeed on self");
    assert!(first > 0, "starttime must be positive clock ticks since boot");
    let second = crate::mcp::daemon::read_pid_starttime(our_pid)
        .expect("second read must also succeed");
    assert_eq!(
        first, second,
        "starttime must be invariant for the lifetime of a process"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn read_pid_starttime_out_of_range_is_none() {
    // /proc/<u32::MAX-1>/stat does not exist; the helper must return None
    // rather than panic, so callers can fall back to pid-only liveness.
    assert_eq!(
        crate::mcp::daemon::read_pid_starttime(u32::MAX - 1),
        None,
        "out-of-range PID must yield None (no panic, no false positive)"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn read_pid_starttime_reaped_child_is_none() {
    // After a child has been reaped, /proc/<pid>/stat disappears. This
    // path is the workhorse for detecting a dead CC client: the agent
    // record's stashed starttime will never again match a live process
    // under the same PID, because the stat file itself is gone.
    let mut child = std::process::Command::new("/bin/true")
        .spawn()
        .expect("spawn /bin/true");
    let pid = child.id();
    let _ = child.wait().expect("wait for child");
    // /proc cleanup is synchronous after reap on Linux — no poll needed.
    assert_eq!(
        crate::mcp::daemon::read_pid_starttime(pid),
        None,
        "reaped child's /proc/<pid>/stat must be gone; starttime read returns None"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn pid_matches_fingerprint_true_on_self_with_correct_starttime() {
    // Positive control: the fingerprint helper must agree with itself
    // when fed the live PID and its just-read starttime.
    let our_pid = std::process::id();
    let st = crate::mcp::daemon::read_pid_starttime(our_pid).expect("starttime on self");
    assert!(
        crate::mcp::daemon::pid_matches_fingerprint(our_pid, st),
        "self + current starttime must match"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn pid_matches_fingerprint_false_on_self_with_wrong_starttime() {
    // This is the cas-ea46 core assertion: a live PID with a *different*
    // starttime than what was stashed at registration must be rejected as
    // "someone else got this PID". We fake the stashed value by flipping
    // a bit in the real starttime; the helper must return false.
    let our_pid = std::process::id();
    let real_st = crate::mcp::daemon::read_pid_starttime(our_pid).expect("starttime on self");
    let wrong_st = real_st.wrapping_add(1);
    assert_ne!(real_st, wrong_st, "bit-flip must change the value");
    assert!(
        !crate::mcp::daemon::pid_matches_fingerprint(our_pid, wrong_st),
        "live PID + mismatched starttime must report as dead (PID recycled)"
    );
}

#[cfg(unix)]
#[test]
fn pid_matches_fingerprint_false_on_dead_pid() {
    // An out-of-range PID is dead regardless of claimed starttime; the
    // pid_alive() short-circuit must return false before any /proc read.
    assert!(
        !crate::mcp::daemon::pid_matches_fingerprint(u32::MAX - 1, 12345),
        "dead PID must report as non-matching regardless of starttime"
    );
}

#[test]
fn parse_starttime_from_stat_handles_comm_with_parens_and_spaces() {
    // Adversarial/testing review: /proc/<pid>/stat field 2 (`comm`) is wrapped
    // in parens and may itself contain spaces and parens. The parser splits
    // on the *last* `)` to preserve correct field indexing; flipping to the
    // *first* `)` would silently shift every subsequent field index by the
    // number of stray `)` inside comm. Pin the invariant with a synthetic
    // line where comm contains both a space and a `)`.
    //
    // Field layout (after comm): state=R, ppid=1, pgrp=1, session=1, tty_nr=0,
    // tpgid=-1, flags=0, minflt=0, cminflt=0, majflt=0, cmajflt=0, utime=0,
    // stime=0, cutime=0, cstime=0, priority=20, nice=0, num_threads=1,
    // itrealvalue=0, starttime=9876543210. That's 19 fields after state,
    // matching field 22 = index 19 in the post-comm tail.
    let synthetic = "1234 (weird )name with spaces) R 1 1 1 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 9876543210 1 2 3";
    assert_eq!(
        crate::mcp::daemon::parse_starttime_from_stat(synthetic),
        Some(9876543210),
        "parser must split on the last `)` and land on field 22"
    );
}

#[test]
fn parse_starttime_from_stat_returns_none_on_malformed_input() {
    // Garbage input must not panic and must not fabricate a starttime.
    assert_eq!(
        crate::mcp::daemon::parse_starttime_from_stat(""),
        None,
        "empty input → None"
    );
    assert_eq!(
        crate::mcp::daemon::parse_starttime_from_stat("no paren here"),
        None,
        "no `)` → None"
    );
    // Too few fields after comm.
    assert_eq!(
        crate::mcp::daemon::parse_starttime_from_stat("1 (short) R 1 2"),
        None,
        "truncated stat → None"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn stamp_pid_fingerprint_writes_metadata_for_self() {
    // cas-ea46 / maintainability review: a single helper writes the
    // pid_starttime metadata key, so every registration site gets the
    // fingerprint without drift. Assert the helper actually populates the
    // key with a parseable u64 matching read_pid_starttime.
    let mut agent = crate::types::Agent::new("test-stamp".to_string(), "unit".to_string());
    let pid = std::process::id();
    let expected = crate::mcp::daemon::read_pid_starttime(pid).expect("starttime on self");
    crate::mcp::daemon::stamp_pid_fingerprint(&mut agent, pid);
    let stamped = agent
        .metadata
        .get(crate::mcp::daemon::PID_STARTTIME_KEY)
        .expect("stamp_pid_fingerprint must populate PID_STARTTIME_KEY");
    assert_eq!(
        stamped.parse::<u64>().ok(),
        Some(expected),
        "stamped value must round-trip as a u64 equal to read_pid_starttime(self)"
    );
}
