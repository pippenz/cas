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
