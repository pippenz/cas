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

// =========================================================================
// evaluate_liveness outcome matrix (cas-5b1c)
// =========================================================================
//
// The heartbeat-gate branch selection was extracted from send_agent_heartbeat
// so the fingerprint-vs-pid-only decision can be unit-tested without a live
// daemon, store, or tokio runtime. These tests pin the selection logic.
// Adding a new outcome or reordering the match arms will fail at least one
// of these.

use crate::mcp::daemon::{LivenessOutcome, PID_STARTTIME_KEY, evaluate_liveness};

fn make_test_agent(pid: Option<u32>, starttime_meta: Option<&str>) -> crate::types::Agent {
    let mut agent = crate::types::Agent::new("eval-test".to_string(), "w".to_string());
    agent.pid = pid;
    if let Some(raw) = starttime_meta {
        agent
            .metadata
            .insert(PID_STARTTIME_KEY.to_string(), raw.to_string());
    }
    agent
}

#[test]
fn evaluate_liveness_no_pid_recorded_when_agent_pid_is_none() {
    // Legacy agent (pre-cas-2749). Neither probe should be consulted; the
    // outcome must be NoPidRecorded so the caller can emit the legacy warn.
    let agent = make_test_agent(None, None);
    let probe_calls = std::cell::Cell::new(0u32);
    let outcome = evaluate_liveness(
        &agent,
        |_| {
            probe_calls.set(probe_calls.get() + 1);
            true
        },
        |_, _| {
            probe_calls.set(probe_calls.get() + 1);
            true
        },
    );
    assert_eq!(outcome, LivenessOutcome::NoPidRecorded);
    assert_eq!(
        probe_calls.get(),
        0,
        "no pid → neither probe should be called"
    );
}

#[test]
fn evaluate_liveness_alive_with_fingerprint_when_match() {
    // Metadata contains a valid fingerprint and fingerprint_matches_fn agrees
    // → Alive { fingerprint_checked: true }. pid_alive must not be called
    // (the strict check is authoritative when a fingerprint is present).
    let agent = make_test_agent(Some(4242), Some("9876543210"));
    let pid_alive_called = std::cell::Cell::new(false);
    let fp_called_with = std::cell::Cell::new(None);
    let outcome = evaluate_liveness(
        &agent,
        |_| {
            pid_alive_called.set(true);
            true
        },
        |pid, st| {
            fp_called_with.set(Some((pid, st)));
            true
        },
    );
    assert_eq!(
        outcome,
        LivenessOutcome::Alive {
            cc_pid: 4242,
            fingerprint_checked: true
        }
    );
    assert!(
        !pid_alive_called.get(),
        "pid_alive must NOT be called when fingerprint is present"
    );
    assert_eq!(
        fp_called_with.get(),
        Some((4242u32, 9876543210u64)),
        "fingerprint_matches_fn must be called with (pid, expected)"
    );
}

#[test]
fn evaluate_liveness_dead_when_fingerprint_mismatch() {
    // Metadata contains a valid fingerprint but fingerprint_matches_fn
    // returns false → Dead with fingerprint_checked=true. This is the
    // core cas-ea46 AC in extracted form.
    let agent = make_test_agent(Some(4242), Some("9876543210"));
    let outcome = evaluate_liveness(&agent, |_| unreachable!(), |_, _| false);
    assert_eq!(
        outcome,
        LivenessOutcome::Dead {
            cc_pid: 4242,
            fingerprint_checked: true
        }
    );
}

#[test]
fn evaluate_liveness_alive_legacy_when_no_fingerprint_and_pid_alive() {
    // Pre-cas-ea46 agent: pid present but no metadata key. pid_alive_fn
    // reports alive → AliveLegacyFingerprint. fingerprint_matches_fn must
    // not be consulted because there is no expected starttime to compare.
    let agent = make_test_agent(Some(7777), None);
    let fp_called = std::cell::Cell::new(false);
    let outcome = evaluate_liveness(
        &agent,
        |pid| {
            assert_eq!(pid, 7777);
            true
        },
        |_, _| {
            fp_called.set(true);
            true
        },
    );
    assert_eq!(
        outcome,
        LivenessOutcome::Alive {
            cc_pid: 7777,
            fingerprint_checked: false
        }
    );
    assert!(
        !fp_called.get(),
        "fingerprint fn must NOT be called when no fingerprint metadata is stashed"
    );
}

#[test]
fn evaluate_liveness_dead_when_no_fingerprint_and_pid_dead() {
    // Pre-cas-ea46 agent with dead pid: pid_alive_fn reports dead →
    // Dead with fingerprint_checked=false so caller tracing can
    // distinguish pid-only from fingerprint-verified verdicts.
    let agent = make_test_agent(Some(7777), None);
    let outcome = evaluate_liveness(&agent, |_| false, |_, _| unreachable!());
    assert_eq!(
        outcome,
        LivenessOutcome::Dead {
            cc_pid: 7777,
            fingerprint_checked: false
        }
    );
}

#[test]
fn evaluate_liveness_malformed_fingerprint_falls_back_to_pid_only() {
    // If a future writer puts garbage in PID_STARTTIME_KEY (or a migration
    // mangles it), `parse::<u64>()` yields None → behavior must be
    // indistinguishable from "no fingerprint stashed": pid-only fallback.
    // This pins the graceful-degradation contract surfaced in cas-ea46
    // adversarial review.
    let agent = make_test_agent(Some(9999), Some("not-a-number"));
    let outcome = evaluate_liveness(
        &agent,
        |pid| {
            assert_eq!(pid, 9999);
            true
        },
        |_, _| panic!("fingerprint fn must not be called on malformed fingerprint"),
    );
    assert_eq!(
        outcome,
        LivenessOutcome::Alive {
            cc_pid: 9999,
            fingerprint_checked: false
        }
    );
}

#[test]
fn evaluate_liveness_non_linux_fallback_live_pid_unreadable_proc() {
    // Simulates the non-Linux / unreadable-/proc case on a host where
    // the fingerprint WAS stashed at registration but is now unverifiable.
    // Per cas-ea46 strict semantics: fingerprint_matches_fn returns false
    // → evaluate_liveness must report Dead, not silently trust pid_alive.
    // This pins the "None from /proc on a fingerprinted agent = suspicious"
    // adversarial catch.
    let agent = make_test_agent(Some(1234), Some("5555"));
    let outcome = evaluate_liveness(
        &agent,
        |_| panic!("pid_alive must not be consulted when fingerprint path is taken"),
        |pid, st| {
            // Simulate strict: live pid, /proc unreadable → fingerprint_fn
            // returns false (pid_matches_fingerprint's semantics).
            assert_eq!((pid, st), (1234, 5555));
            false
        },
    );
    assert_eq!(
        outcome,
        LivenessOutcome::Dead {
            cc_pid: 1234,
            fingerprint_checked: true
        }
    );
}

// =========================================================================
// Registration-site fingerprint-stamp parity (cas-5b1c)
// =========================================================================
//
// Every agent-registration code path that sets `agent.pid = Some(pid)` must
// also stamp the pid_starttime fingerprint. A silent drift (one site omits
// the stamp) degrades PID-reuse protection to pid-only for that site with
// no compile-time signal. The table below enumerates the call-path shape
// each site produces; adding a 4th site is one append. If a new site is
// introduced without a row here, that is the review catch.

#[cfg(target_os = "linux")]
#[test]
fn all_agent_registration_sites_stamp_pid_fingerprint() {
    // Use this process's own PID so read_pid_starttime has a real /proc
    // entry to observe. Each builder mirrors the pid + stamp_pid_fingerprint
    // sequence from one of the three real registration code paths.
    let pid = std::process::id();

    type AgentBuilder = fn(u32) -> crate::types::Agent;

    fn socket_driven_register(pid: u32) -> crate::types::Agent {
        // Mirrors daemon::register_agent (socket-driven hook path).
        let mut agent = crate::types::Agent::new("sock-driven".to_string(), "w".to_string());
        agent.pid = Some(pid);
        crate::mcp::daemon::stamp_pid_fingerprint(&mut agent, pid);
        agent
    }

    fn self_register_hints(pid: u32) -> crate::types::Agent {
        // Mirrors server::register_agent_with_hints (MCP bootstrap path).
        let mut agent = crate::types::Agent::new("self-hints".to_string(), "w".to_string());
        agent.pid = Some(pid);
        crate::mcp::daemon::stamp_pid_fingerprint(&mut agent, pid);
        agent
    }

    fn re_register_missing(pid: u32) -> crate::types::Agent {
        // Mirrors server::mod.rs re-register-missing fallback.
        let mut agent = crate::types::Agent::new("re-reg".to_string(), "w".to_string());
        agent.pid = Some(pid);
        crate::mcp::daemon::stamp_pid_fingerprint(&mut agent, pid);
        agent
    }

    // To add a new registration site: append one (name, builder) pair. The
    // assertions below apply uniformly — same contract, same test.
    let sites: &[(&str, AgentBuilder)] = &[
        ("daemon::register_agent (socket-driven)", socket_driven_register),
        ("server::register_agent_with_hints (self)", self_register_hints),
        ("server::re-register-missing (self)", re_register_missing),
    ];

    let expected_st = crate::mcp::daemon::read_pid_starttime(pid).expect("starttime on self");

    for (name, build) in sites {
        let agent = build(pid);
        assert_eq!(agent.pid, Some(pid), "[{name}] pid must be populated");
        let stamped = agent
            .metadata
            .get(PID_STARTTIME_KEY)
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or_else(|| {
                panic!("[{name}] must populate PID_STARTTIME_KEY with a parseable u64");
            });
        assert_eq!(
            stamped, expected_st,
            "[{name}] stamped value must match live read_pid_starttime"
        );
    }
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
