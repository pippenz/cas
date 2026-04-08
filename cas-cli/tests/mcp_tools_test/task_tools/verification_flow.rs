use crate::support::*;
use cas::mcp::tools::*;
use cas::store::{open_agent_store, open_task_store, open_verification_store, open_worktree_store};
use cas::types::{Verification, VerificationType, Worktree};
use rmcp::handler::server::wrapper::Parameters;
use std::sync::{Mutex, OnceLock};

/// Serializes tests in this file that depend on the process-wide
/// `CAS_AGENT_ROLE` env var (set by cas-26e1's supervisor-bypass tests and
/// cleared by `setup_cas`). Without this lock, cargo's default parallel
/// runner lets a supervisor-bypass test set `CAS_AGENT_ROLE=supervisor`
/// while a sibling close-path test is mid-flight, which silently flips the
/// sibling into the bypass branch and asserts fail on cross-talk.
fn env_test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[tokio::test]
async fn test_task_close_blocked_without_verification() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");

    // Initialize verification store
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create task
    let req = TaskCreateRequest {
        title: "Task requiring verification".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    let id = extract_task_id(&text).expect("should have task ID");

    // Start task
    let start_req = IdRequest { id: id.to_string() };
    let _ = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    // Try to close task without verification - should be blocked
    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return a result");

    let text = extract_text(result);
    assert!(
        text.contains("VERIFICATION REQUIRED"),
        "Close should be blocked without verification: {text}"
    );
    assert!(
        text.contains("Task(subagent_type=\"task-verifier\""),
        "Close warning must include explicit Task() spawn syntax: {text}"
    );

    // A durable dispatch-request verification row must be persisted so the
    // close attempt is observable (no more fire-and-forget). The verdict
    // row will be written later by the task-verifier subagent.
    let latest = verification_store
        .get_latest_for_task(id)
        .unwrap()
        .expect("dispatch-request verification row should exist after close");
    assert_eq!(
        latest.status,
        cas::types::VerificationStatus::Error,
        "Dispatch-request row should have Error status until the subagent writes a verdict"
    );
    assert!(
        latest.summary.contains("Dispatch requested"),
        "Dispatch-request row summary should identify itself: {}",
        latest.summary
    );
}

#[tokio::test]
async fn test_task_close_sets_assignee_for_worktree_merge_jail() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");

    std::fs::write(
        cas_dir.join("config.toml"),
        r#"[verification]
enabled = false

[worktrees]
enabled = true
require_merge_on_epic_close = true
"#,
    )
    .expect("should write config");

    let req = TaskCreateRequest {
        title: "Task with worktree".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    let id = extract_task_id(&text).expect("should have task ID");

    let worktree_store = open_worktree_store(&cas_dir).expect("open worktree store");
    worktree_store.init().expect("init worktree store");
    let worktree_id = Worktree::generate_id();
    let worktree = Worktree::new(
        worktree_id.clone(),
        "cas/test-worktree".to_string(),
        "main".to_string(),
        temp.path().join("worktree"),
    );
    worktree_store.add(&worktree).expect("should add worktree");

    let task_store = open_task_store(&cas_dir).expect("open task store");
    let mut task = task_store.get(id).expect("task should exist");
    task.worktree_id = Some(worktree_id);
    task_store.update(&task).expect("should update task");

    let close_req = TaskCloseRequest {
        id: task.id.clone(),
        reason: Some("Done".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return result");

    let text = extract_text(result);
    assert!(
        text.contains("WORKTREE MERGE REQUIRED"),
        "Close should be blocked for merge: {text}"
    );

    let task = task_store.get(&task.id).expect("task should exist");
    assert!(
        task.pending_worktree_merge,
        "pending_worktree_merge should be set"
    );

    let agent_store = open_agent_store(&cas_dir).expect("open agent store");
    let agent_id = agent_store
        .list(None)
        .expect("list agents")
        .first()
        .map(|a| a.id.clone())
        .expect("agent should exist");
    assert_eq!(
        task.assignee.as_deref(),
        Some(agent_id.as_str()),
        "assignee should be set to current agent"
    );
}

#[tokio::test]
async fn test_epic_close_requires_epic_verification_type() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");

    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create epic
    let req = TaskCreateRequest {
        title: "Epic requiring epic verification".to_string(),
        description: None,
        priority: 2,
        task_type: "epic".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    let id = extract_task_id(&text).expect("should have task ID");

    // Start epic
    let start_req = IdRequest { id: id.to_string() };
    let _ = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    // Close without verification should be blocked
    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return a result");
    let text = extract_text(result);
    assert!(
        text.contains("VERIFICATION REQUIRED"),
        "Epic close should be blocked without verification: {text}"
    );

    // Add a task-level verification - should NOT unblock epic close
    let task_ver = Verification::approved(
        "ver-epic-task".to_string(),
        id.to_string(),
        "Task-level verification".to_string(),
    );
    verification_store.add(&task_ver).unwrap();

    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return a result");
    let text = extract_text(result);
    assert!(
        text.contains("VERIFICATION REQUIRED"),
        "Epic close should still be blocked with task-level verification: {text}"
    );

    // Add epic-level verification - should unblock
    let mut epic_ver = Verification::approved(
        "ver-epic-ok".to_string(),
        id.to_string(),
        "Epic verification passed".to_string(),
    );
    epic_ver.verification_type = VerificationType::Epic;
    verification_store.add(&epic_ver).unwrap();

    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should succeed");
    let text = extract_text(result);
    assert!(
        text.contains("Closed") || text.contains("closed"),
        "Epic should close with epic verification: {text}"
    );
}

#[tokio::test]
async fn test_task_lifecycle_with_verification() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");

    // Initialize verification store
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create task
    let req = TaskCreateRequest {
        title: "Lifecycle task".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    let id = extract_task_id(&text).expect("should have task ID");

    // Start task
    let start_req = IdRequest { id: id.to_string() };
    let result = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    let text = extract_text(result);
    assert!(text.contains("Started") || text.contains("in_progress"));

    // Create an approved verification record
    let verification = Verification::approved(
        "ver-test".to_string(),
        id.to_string(),
        "All checks passed".to_string(),
    );
    verification_store.add(&verification).unwrap();

    // Close task - should succeed now with verification
    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed successfully".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should succeed");

    let text = extract_text(result);
    assert!(
        text.contains("Closed") || text.contains("closed"),
        "Task should close with verification: {text}"
    );
    assert!(
        text.contains("verified"),
        "Should indicate verification: {text}"
    );
}

#[tokio::test]
async fn test_task_close_blocked_with_rejected_verification() {
    use cas::types::VerificationIssue;

    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");

    // Initialize verification store
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create task
    let req = TaskCreateRequest {
        title: "Task with rejected verification".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };

    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");

    let text = extract_text(result);
    let id = extract_task_id(&text).expect("should have task ID");

    // Start task
    let start_req = IdRequest { id: id.to_string() };
    let _ = service
        .cas_task_start(Parameters(start_req))
        .await
        .expect("task_start should succeed");

    // Create a rejected verification record with issues
    let issues = vec![VerificationIssue::new(
        "src/main.rs".to_string(),
        "todo_comment".to_string(),
        "TODO comment found".to_string(),
    )];
    let verification = Verification::rejected(
        "ver-reject".to_string(),
        id.to_string(),
        "Found incomplete work".to_string(),
        issues,
    );
    verification_store.add(&verification).unwrap();

    // Try to close task - should be blocked due to rejected verification
    let close_req = TaskCloseRequest {
        id: id.to_string(),
        reason: Some("Completed".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return a result");

    let text = extract_text(result);
    assert!(
        text.contains("VERIFICATION FAILED"),
        "Close should be blocked with rejected verification: {text}"
    );
    assert!(text.contains("1 issue"), "Should show issue count: {text}");
}

/// Regression test for cas-7de3: `task.close` must either dispatch a verifier
/// (creating a verification row) or close the task with an explicit skip
/// reason recorded in notes/metadata. The pre-fix behavior returned a
/// `⚠️ VERIFICATION REQUIRED` warning string while leaving the task in
/// `InProgress` with no verification row — a fire-and-forget that silently
/// drops the close attempt. This test fails on main and passes once the
/// dispatch/skip path is wired up.
#[tokio::test]
async fn test_task_close_runs_verifier_or_skips_cleanly() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create + start a task.
    let req = TaskCreateRequest {
        title: "Dispatch-on-close regression task".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };
    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create should succeed");
    let id = extract_task_id(&extract_text(result))
        .expect("should have task ID")
        .to_string();

    let _ = service
        .cas_task_start(Parameters(IdRequest { id: id.clone() }))
        .await
        .expect("task_start should succeed");

    // Close with a clean, acceptance-criteria-satisfying reason. This is the
    // exact shape of close call that triggered the cas-7de3 regression: the
    // handler is supposed to dispatch a verifier (or record a skip), not just
    // print a warning and leave the task open.
    let close_req = TaskCloseRequest {
        id: id.clone(),
        reason: Some("Completed all acceptance criteria. Deployed to prod.".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should return a result");
    let response_text = extract_text(result);

    // Re-read DB state after the call.
    let task_after = task_store.get(&id).expect("task should still exist");
    let verification_row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup should not error");

    let dispatched_verifier = verification_row.is_some();
    let closed_with_skip_reason = task_after.status == cas::types::TaskStatus::Closed
        && (task_after.notes.to_lowercase().contains("verification skipped")
            || task_after
                .close_reason
                .as_deref()
                .map(|r| r.to_lowercase().contains("verification skipped"))
                .unwrap_or(false));

    assert!(
        dispatched_verifier || closed_with_skip_reason,
        "task.close must either dispatch a verifier (create a verification row) \
         or close the task with an explicit skip reason. Got:\n\
         \x20 - response text: {response_text}\n\
         \x20 - task status after close: {:?}\n\
         \x20 - verification row present: {dispatched_verifier}\n\
         \x20 - task notes: {:?}\n\
         \x20 - task close_reason: {:?}\n\
         This is the cas-7de3 regression: the handler returned a fire-and-forget \
         warning without actually running verification or recording a skip.",
        task_after.status,
        task_after.notes,
        task_after.close_reason,
    );
}

// === cas-26e1: supervisor escape hatch ===
//
// These tests lock down the supervisor-close bypass that shipped in
// close_ops.rs lines 64-82 (`assignee_inactive` path). Precedent: gabber-studio
// April 2-3 session `f21e74e7-3c57-4cf6-a295-ca6b8e113e79` closed ~12 worker
// tasks via this hatch after workers wedged (cas-bd17, cas-d6b0, cas-ce02,
// cas-79e9, cas-74b7, cas-6f19, cas-901d, cas-e3a3, cas-80de, cas-c5be,
// cas-ff22, cas-2bf7).
//
// The hatch is STRUCTURAL, not a reason-string match: it fires when BOTH
// `is_supervisor_from_env()` is true AND the task's assignee is missing /
// not-found / heartbeat-expired. The "verification skipped — assignee inactive"
// string is only a display note the handler appends to the success message
// (close_ops.rs:487); the supervisor's close_reason does not gate the hatch.
//
// These tests MUST still pass after cas-4acd narrowed the per-tool
// verification jail at server/mod.rs:646-663 to stop exempting `task.close`
// for factory workers. That narrowing affects the pre-handler jail; the bypass
// itself lives inside close_ops.rs and is unaffected — these tests verify
// that directly.

/// Small RAII guard so CAS_AGENT_ROLE is always cleared on drop, even on
/// panic, to avoid leaking the var into sibling tests that don't set it.
struct ScopedSupervisorEnv;

impl ScopedSupervisorEnv {
    fn new() -> Self {
        // SAFETY: setup_cas documents the same --test-threads=1-or-accept-race
        // contract. We set during the test body only and unconditionally
        // remove on drop.
        unsafe {
            std::env::set_var("CAS_AGENT_ROLE", "supervisor");
        }
        Self
    }
}

impl Drop for ScopedSupervisorEnv {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("CAS_AGENT_ROLE");
        }
    }
}

/// Positive: supervisor closes an orphaned task (no assignee) → bypass fires.
/// Task goes to Closed without running the verifier and without writing a
/// verification row. The close_reason passed by the supervisor is preserved
/// on the task and the response carries the
/// "(verification skipped — assignee inactive)" marker.
#[tokio::test]
async fn test_close_supervisor_bypass_orphaned_task() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Create + start a task, then strip its assignee to simulate the
    // orphaned-worker state the hatch is designed to recover from.
    let req = TaskCreateRequest {
        title: "Orphaned worker task for escape-hatch test".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };
    let create_text = extract_text(
        service
            .cas_task_create(Parameters(req))
            .await
            .expect("task_create should succeed"),
    );
    let id = extract_task_id(&create_text)
        .expect("should have task ID")
        .to_string();

    // Note: cas_task_start would set the assignee to the current test agent,
    // which would then be "alive" and short-circuit the inactive path. We want
    // the orphaned branch (`No assignee at all → orphaned`), so we set status
    // directly and leave assignee = None.
    let mut task = task_store.get(&id).expect("task should exist");
    task.status = cas::types::TaskStatus::InProgress;
    task.assignee = None;
    task_store.update(&task).expect("should update task");

    // Now flip the process into supervisor mode for the close call only.
    let _guard = ScopedSupervisorEnv::new();

    let close_req = TaskCloseRequest {
        id: id.clone(),
        reason: Some("verification skipped — assignee inactive".to_string()),
    };
    let result = service
        .cas_task_close(Parameters(close_req))
        .await
        .expect("task_close should succeed via supervisor bypass");
    let response_text = extract_text(result);

    assert!(
        response_text.contains("Closed"),
        "bypass close should report success: {response_text}"
    );
    assert!(
        response_text.contains("verification skipped — assignee inactive"),
        "response must carry the bypass marker: {response_text}"
    );
    assert!(
        !response_text.contains("VERIFICATION REQUIRED"),
        "bypass must not drop into the jail path: {response_text}"
    );

    let task_after = task_store.get(&id).expect("task should exist");
    assert_eq!(
        task_after.status,
        cas::types::TaskStatus::Closed,
        "supervisor bypass must transition task to Closed"
    );
    assert_eq!(
        task_after.close_reason.as_deref(),
        Some("verification skipped — assignee inactive"),
        "supervisor close_reason must be preserved verbatim"
    );
    assert!(
        task_after.notes.to_lowercase().contains("verification skipped"),
        "close_reason must also appear in the task notes timeline: {}",
        task_after.notes
    );

    // No verification row is written by the bypass path — the whole point of
    // the hatch is to avoid the dispatch/verifier machinery entirely when the
    // assignee is gone.
    let verification_row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup should not error");
    assert!(
        verification_row.is_none(),
        "supervisor bypass must NOT write a dispatch/verdict row, got {verification_row:?}"
    );
}

/// Positive: supervisor closes a task whose assignee points at an agent that
/// does not exist in the agent store. This exercises the "assignee not found →
/// treat as inactive" branch distinct from the None-assignee branch above.
#[tokio::test]
async fn test_close_supervisor_bypass_ghost_assignee() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();

    let req = TaskCreateRequest {
        title: "Task assigned to a ghost agent".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };
    let id = extract_task_id(&extract_text(
        service
            .cas_task_create(Parameters(req))
            .await
            .expect("task_create should succeed"),
    ))
    .expect("should have task ID")
    .to_string();

    let mut task = task_store.get(&id).expect("task should exist");
    task.status = cas::types::TaskStatus::InProgress;
    task.assignee = Some("ghost-agent-does-not-exist".to_string());
    task_store.update(&task).expect("should update task");

    let _guard = ScopedSupervisorEnv::new();

    let close_req = TaskCloseRequest {
        id: id.clone(),
        reason: Some("verification skipped — assignee inactive (ghost agent)".to_string()),
    };
    let response_text = extract_text(
        service
            .cas_task_close(Parameters(close_req))
            .await
            .expect("task_close should succeed via supervisor bypass"),
    );

    assert!(
        response_text.contains("Closed")
            && response_text.contains("verification skipped — assignee inactive"),
        "ghost-assignee bypass should close and mark skipped: {response_text}"
    );

    let task_after = task_store.get(&id).expect("task should exist");
    assert_eq!(task_after.status, cas::types::TaskStatus::Closed);
    assert!(
        verification_store
            .get_latest_for_task(&id)
            .expect("verification lookup should not error")
            .is_none(),
        "ghost-assignee bypass must not write a verification row"
    );
}

/// Negative: supervisor closes a task whose assignee is the currently-alive
/// test agent. `is_heartbeat_expired(300)` is false for a freshly registered
/// agent, so the bypass does NOT fire and close drops into the normal
/// verification path. This pins the bypass to the specific inactive-assignee
/// precondition and proves the hatch isn't a catch-all "supervisor closes
/// anything" escape.
///
/// After cas-4acd narrowed the per-tool jail at server/mod.rs:646-663 to stop
/// exempting `task.close` for factory workers, the jail text returned here
/// comes from `close_ops.rs` (VERIFICATION REQUIRED) — exactly what we assert.
#[tokio::test]
async fn test_close_supervisor_no_bypass_when_assignee_alive() {
    let _env_lock = env_test_lock();
    let (temp, service) = setup_cas();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();

    // Grab the alive test agent registered by setup_cas.
    let agent_store = open_agent_store(&cas_dir).expect("open agent store");
    let alive_agent_id = agent_store
        .list(None)
        .expect("list agents")
        .first()
        .map(|a| a.id.clone())
        .expect("setup_cas should register a test agent");

    let req = TaskCreateRequest {
        title: "Task with an alive assignee".to_string(),
        description: None,
        priority: 2,
        task_type: "task".to_string(),
        labels: None,
        notes: None,
        blocked_by: None,
        design: None,
        acceptance_criteria: None,
        external_ref: None,
        assignee: None,
        demo_statement: None,
        epic: None,
    };
    let id = extract_task_id(&extract_text(
        service
            .cas_task_create(Parameters(req))
            .await
            .expect("task_create should succeed"),
    ))
    .expect("should have task ID")
    .to_string();

    let mut task = task_store.get(&id).expect("task should exist");
    task.status = cas::types::TaskStatus::InProgress;
    task.assignee = Some(alive_agent_id);
    task_store.update(&task).expect("should update task");

    let _guard = ScopedSupervisorEnv::new();

    let close_req = TaskCloseRequest {
        id: id.clone(),
        // Intentionally still use the "verification skipped" phrase to prove
        // the bypass is structural (assignee state), not reason-driven. Even
        // with this phrase, an alive assignee must keep the jail engaged.
        reason: Some("verification skipped — assignee inactive".to_string()),
    };
    let response_text = extract_text(
        service
            .cas_task_close(Parameters(close_req))
            .await
            .expect("task_close should return a result"),
    );

    assert!(
        response_text.contains("VERIFICATION REQUIRED"),
        "alive assignee must NOT trigger the bypass — expected VERIFICATION REQUIRED: {response_text}"
    );
    assert!(
        !response_text.contains("Closed task:"),
        "alive assignee path must not report a closed task: {response_text}"
    );

    let task_after = task_store.get(&id).expect("task should exist");
    assert_ne!(
        task_after.status,
        cas::types::TaskStatus::Closed,
        "alive assignee + supervisor must not transition task to Closed"
    );

    // A dispatch-request verification row should have been persisted for the
    // normal path (cas-7de3 regression coverage). This also confirms the
    // close attempt exercised the dispatch branch, not the bypass branch.
    let verification_row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup should not error")
        .expect("alive-assignee close should persist a dispatch-request row");
    assert_eq!(
        verification_row.status,
        cas::types::VerificationStatus::Error,
        "dispatch-request row should have Error status until a verdict lands"
    );
}
