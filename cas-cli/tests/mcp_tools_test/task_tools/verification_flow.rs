use crate::support::*;
use cas::mcp::tools::*;
use cas::store::{open_agent_store, open_task_store, open_verification_store, open_worktree_store};
use cas::types::{Verification, VerificationType, Worktree};
use rmcp::handler::server::wrapper::Parameters;

// cas-3bd4: env_test_lock() now lives in `support.rs` so `setup_cas()`
// can hold it while clearing factory env vars. Tests that need to set
// `CAS_AGENT_ROLE=supervisor` via `ScopedSupervisorEnv` MUST call
// `setup_cas()` FIRST and then acquire `env_test_lock()` — see the
// support.rs docs. Acquiring before calling `setup_cas` would deadlock
// because std `Mutex` is not re-entrant.

#[tokio::test]
async fn test_task_close_blocked_without_verification() {
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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

    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
    // cas-3bd4: orphaned (no-assignee) closes now cite the accurate
    // reason — "orphaned task, no assignee" — instead of the catch-all
    // "assignee inactive" phrase that was always emitted regardless of
    // actual assignee state.
    assert!(
        response_text.contains("verification skipped — orphaned task, no assignee"),
        "response must carry the orphaned-task bypass marker: {response_text}"
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

    // Per cas-82d6: the bypass path MUST write a durable `Skipped`
    // verification row so downstream workers that inherit a BlockedBy on
    // this task are not jailed by `check_pending_verification` (which used
    // to only accept `Approved`). The row is the audit trail for "closed
    // without running the verifier".
    let verification_row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup should not error")
        .expect("supervisor bypass must write a Skipped verification row");
    assert_eq!(
        verification_row.status,
        cas::types::VerificationStatus::Skipped,
        "bypass row must be Skipped, got {:?}",
        verification_row.status
    );
}

/// Positive: supervisor closes a task whose assignee points at an agent that
/// does not exist in the agent store. This exercises the "assignee not found →
/// treat as inactive" branch distinct from the None-assignee branch above.
#[tokio::test]
async fn test_close_supervisor_bypass_ghost_assignee() {
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
    };
    let response_text = extract_text(
        service
            .cas_task_close(Parameters(close_req))
            .await
            .expect("task_close should succeed via supervisor bypass"),
    );

    // cas-3bd4: a ghost assignee (agent row missing from the store) is
    // now reported as "assignee unknown" — the pre-cas-3bd4 path
    // always said "assignee inactive" regardless of the true state,
    // because `agent_store.get(name)` unwrap_or(true) collapsed every
    // lookup failure into the same bucket. The new path keeps the
    // supervisor bypass behavior but cites the real reason.
    assert!(
        response_text.contains("Closed")
            && response_text.contains("verification skipped — assignee unknown"),
        "ghost-assignee bypass should close and mark skipped: {response_text}"
    );

    let task_after = task_store.get(&id).expect("task should exist");
    assert_eq!(task_after.status, cas::types::TaskStatus::Closed);
    // Per cas-82d6: bypass now writes a Skipped row so downstream
    // BlockedBy consumers don't hit the MCP jail.
    let row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup should not error")
        .expect("ghost-assignee bypass must write a Skipped verification row");
    assert_eq!(row.status, cas::types::VerificationStatus::Skipped);
}

/// cas-3bd4 regression: a factory worker's `task.assignee` stores the agent's
/// display *name* (e.g. `"mighty-viper-52"`), not its session id. The pre-fix
/// `agent_store.get(task.assignee)` therefore always failed, `unwrap_or(true)`
/// treated the assignee as inactive, and supervisor closes silently succeeded
/// with the misleading message `"verification skipped — assignee inactive"`
/// even when the worker was demonstrably alive and holding a fresh lease.
///
/// Post-fix, the close path resolves liveness from the task's active lease
/// (`TaskLease.agent_id` is the real session id), which survives the name/id
/// mismatch. A supervisor closing such a task without `bypass_code_review=true`
/// must now drop into the normal verification path; with the flag set, the
/// close proceeds but the audit message cites "supervisor bypass", never
/// "assignee inactive".
#[tokio::test]
async fn test_close_supervisor_active_worker_assignee_by_name() {
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();
    let agent_store = open_agent_store(&cas_dir).expect("open agent store");

    // Register a fresh, alive agent with a distinct display name so the
    // id-vs-name mismatch is unambiguous.
    let mut worker = cas::types::Agent::new(
        "test-worker-by-name".to_string(),
        "mighty-viper-99".to_string(),
    );
    worker.agent_type = cas::types::AgentType::Worker;
    worker.role = cas::types::AgentRole::Worker;
    worker.heartbeat(); // ensure fresh last_heartbeat + Active status
    agent_store.register(&worker).expect("register worker");

    let create_req = TaskCreateRequest {
        title: "Task held by a by-name assignee".to_string(),
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
        execution_note: None,
        epic: None,
    };
    let id = extract_task_id(&extract_text(
        service
            .cas_task_create(Parameters(create_req))
            .await
            .expect("task_create should succeed"),
    ))
    .expect("task id")
    .to_string();

    // Store the assignee as the NAME (production bug shape) and put the
    // task in-progress, then claim it on behalf of the worker so the lease
    // carries the real session id.
    let mut task = task_store.get(&id).expect("task exists");
    task.status = cas::types::TaskStatus::InProgress;
    task.assignee = Some("mighty-viper-99".to_string());
    task_store.update(&task).expect("update task");
    agent_store
        .try_claim(&id, &worker.id, 600, Some("worker lease for cas-3bd4 repro"))
        .expect("worker claim should succeed");

    // Flip the caller to supervisor for the close attempt.
    let _guard = ScopedSupervisorEnv::new();

    // --- Attempt 1: no bypass flag. The close MUST drop into the normal
    //     verification path (worker is alive + holding a lease), not the
    //     bypass branch. Pre-fix this path falsely reported the worker as
    //     inactive and closed the task.
    let close_req = TaskCloseRequest {
        id: id.clone(),
        reason: Some("worker finished, asking supervisor to close".to_string()),
        bypass_code_review: None,
        code_review_findings: None,
    };
    let response_text = extract_text(
        service
            .cas_task_close(Parameters(close_req))
            .await
            .expect("task_close returns a result"),
    );
    assert!(
        response_text.contains("VERIFICATION REQUIRED"),
        "active-by-name assignee must NOT trigger inactive bypass — got: {response_text}"
    );
    assert!(
        !response_text.contains("Closed task:"),
        "no bypass flag + active assignee must not transition to Closed: {response_text}"
    );
    assert!(
        !response_text.contains("assignee inactive"),
        "active assignee must never be reported as inactive: {response_text}"
    );
    assert_ne!(
        task_store.get(&id).expect("task exists").status,
        cas::types::TaskStatus::Closed,
        "active assignee + no bypass must leave the task open"
    );

    // --- Attempt 2: with bypass_code_review=true. The close proceeds but
    //     the audit message must cite "supervisor bypass", not "assignee
    //     inactive".
    let close_req = TaskCloseRequest {
        id: id.clone(),
        reason: Some("supervisor forced close after alignment".to_string()),
        bypass_code_review: Some(true),
        code_review_findings: None,
    };
    let response_text = extract_text(
        service
            .cas_task_close(Parameters(close_req))
            .await
            .expect("task_close returns a result"),
    );
    assert!(
        response_text.contains("Closed task:"),
        "supervisor + bypass_code_review must close the task: {response_text}"
    );
    assert!(
        response_text.contains("verification skipped — supervisor bypass"),
        "audit suffix must cite supervisor bypass, not assignee inactive: {response_text}"
    );
    assert!(
        !response_text.contains("assignee inactive"),
        "active assignee must never be reported as inactive even with bypass: {response_text}"
    );
    assert_eq!(
        task_store.get(&id).expect("task exists").status,
        cas::types::TaskStatus::Closed,
        "supervisor bypass must transition task to Closed"
    );

    // Audit trail: the Skipped verification row must record the real
    // reason, not the legacy "assignee inactive or orphaned task" string.
    let row = verification_store
        .get_latest_for_task(&id)
        .expect("verification lookup")
        .expect("supervisor bypass must write a Skipped row");
    assert_eq!(row.status, cas::types::VerificationStatus::Skipped);
    let summary_lc = row.summary.to_lowercase();
    assert!(
        summary_lc.contains("supervisor bypass") && summary_lc.contains("bypass_code_review"),
        "Skipped row summary must name the real reason: {}",
        row.summary
    );
    assert!(
        !summary_lc.contains("inactive") && !summary_lc.contains("orphaned"),
        "Skipped row summary must not inherit the legacy inactive/orphaned wording: {}",
        row.summary
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
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
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
        execution_note: None,
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
        bypass_code_review: None,
code_review_findings: None,
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
// =============================================================================
// cas-9a3a: task-verifier spawn regression
//
// These tests lock in the post-cas-4acd contract between the three layers
// involved in verifier dispatch:
//
//   1. `authorize_agent_action` (cas-cli/src/mcp/server/mod.rs) — the narrowed
//      factory-worker exemption. All mutations EXCEPT `task.close` remain
//      exempt for workers; `task.close` falls through to
//      `check_pending_verification`. This preserves the bba6fbf fix for the
//      mutation-cascade problem while restoring the jail lever on the one
//      action that actually triggers verifier dispatch.
//   2. `cas_task_close` (close_ops.rs) — writes a durable dispatch-request
//      Verification row and returns a warning with explicit
//      `Task(subagent_type="task-verifier", prompt="Verify task <id>")` syntax.
//   3. The pre_tool hook (pre_tool.rs:164-242) — on a Task/Agent spawn with
//      subagent_type="task-verifier", clears `pending_verification` for the
//      current agent's jailed tasks. The hook path is exercised end-to-end by
//      `cas-cli/tests/e2e/hook_e2e/jail_core.rs::test_agent_tool_spawns_task_verifier_and_unjails`
//      (feature-gated behind `claude_rs_e2e`; see docs/verifier-dispatch-trace.md).
//      The tests below simulate the post-hook state by clearing
//      `pending_verification` directly and writing an approved Verification
//      row, which is what the hook + task-verifier subagent would have done.
// =============================================================================

/// Guard that installs factory-worker env vars for the duration of a test
/// and clears them on drop. Matches the pattern in `setup_cas()` —
/// cargo test is single-threaded or accepts the race on env vars.
struct FactoryWorkerEnv;

impl FactoryWorkerEnv {
    fn enter() -> Self {
        // SAFETY: see setup_cas() comment — tests accept the race on env vars.
        unsafe {
            std::env::set_var("CAS_AGENT_ROLE", "worker");
            std::env::set_var("CAS_FACTORY_MODE", "1");
        }
        Self
    }
}

impl Drop for FactoryWorkerEnv {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("CAS_AGENT_ROLE");
            std::env::remove_var("CAS_FACTORY_MODE");
        }
    }
}

/// Build a TaskRequest with only the fields a test needs, via JSON so we
/// don't have to list every Optional field on the struct.
fn task_req(value: serde_json::Value) -> cas_mcp::TaskRequest {
    serde_json::from_value(value).expect("TaskRequest should deserialize from test JSON")
}

/// Narrowed jail — positive case.
///
/// A factory worker who holds an in-progress task with no approved
/// verification must be blocked by `authorize_agent_action` when they
/// attempt `task.close`. Before cas-4acd this path was exempt and the
/// worker saw a passive warning from close_ops instead; after the fix the
/// MCP layer itself rejects the call with `VERIFICATION_JAIL_BLOCKED` and
/// explicit Task() spawn instructions.
#[tokio::test]
async fn test_factory_worker_close_hits_narrowed_jail() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let _cas_dir = temp.path().join(".cas");
    let service = CasService::new(core);
    let _env = FactoryWorkerEnv::enter();

    // Create and start a task so it's leased + InProgress with no verification.
    let create = task_req(serde_json::json!({
        "action": "create",
        "title": "Factory worker close-path jail regression",
        "priority": 2,
        "task_type": "task",
    }));
    let created = service
        .task(Parameters(create))
        .await
        .expect("task.create should succeed for factory worker");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    let start = task_req(serde_json::json!({ "action": "start", "id": id }));
    service
        .task(Parameters(start))
        .await
        .expect("task.start should succeed — not jailed yet");

    // Attempt to close. Must hit the narrowed jail in authorize_agent_action
    // with an explicit McpError — NOT a soft warning from close_ops.
    let close = task_req(serde_json::json!({
        "action": "close",
        "id": id,
        "reason": "Completed all acceptance criteria. Deployed to prod.",
    }));
    let err = service
        .task(Parameters(close))
        .await
        .expect_err("close must be blocked by the narrowed MCP jail for factory workers");
    let msg = err.message.to_string();
    assert!(
        msg.contains("VERIFICATION_JAIL_BLOCKED"),
        "narrowed jail must return VERIFICATION_JAIL_BLOCKED, got: {msg}"
    );
    assert!(
        msg.contains("Task(subagent_type=\"task-verifier\""),
        "jail error must include explicit Task() spawn syntax, got: {msg}"
    );
}

/// cas-82d6: a `Skipped` verification row (supervisor bypass audit trail)
/// must satisfy both the MCP jail (`check_pending_verification`) and the
/// close_ops verification gate. Without this, downstream workers that pick
/// up the same task via resumption — or anyone re-closing a task already
/// bypassed — would be trapped by `VERIFICATION_JAIL_BLOCKED`.
#[tokio::test]
async fn test_skipped_verification_row_satisfies_jail_and_close() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let verification_store = open_verification_store(&cas_dir).unwrap();
    let service = CasService::new(core);
    let _env = FactoryWorkerEnv::enter();

    // Create + start a task so it's leased + InProgress.
    let created = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "Task with a pre-existing Skipped verification row",
            "priority": 2,
            "task_type": "task",
        }))))
        .await
        .expect("create");
    let id = extract_task_id(&extract_text(created))
        .expect("id")
        .to_string();
    service
        .task(Parameters(task_req(serde_json::json!({
            "action": "start",
            "id": id.clone(),
        }))))
        .await
        .expect("start");

    // Insert a Skipped verification row as if a supervisor had previously
    // closed this task via the orphaned-assignee bypass and then it got
    // resumed/reopened.
    let ver_id = verification_store.generate_id().expect("gen ver id");
    let mut row = cas::types::Verification::skipped(
        ver_id,
        id.clone(),
        "cas-82d6 test fixture — supervisor bypass audit row".to_string(),
    );
    row.verification_type = VerificationType::Task;
    verification_store.add(&row).expect("add skipped row");

    // Close as factory worker. Without the cas-82d6 fix this would hit the
    // narrowed MCP jail (check_pending_verification only accepted Approved)
    // OR the close_ops gate (only accepted Approved). With the fix, Skipped
    // is treated as "has verification record → proceed".
    let result = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "close",
            "id": id.clone(),
            "reason": "Completed all acceptance criteria.",
        }))))
        .await
        .expect("close must succeed when a Skipped row exists");
    let text = extract_text(result);
    assert!(
        text.contains("Closed"),
        "close should succeed with Skipped row present, got: {text}"
    );
    assert!(
        !text.contains("VERIFICATION REQUIRED"),
        "Skipped row must satisfy close_ops gate, got: {text}"
    );
    assert!(
        !text.contains("VERIFICATION_JAIL_BLOCKED"),
        "Skipped row must satisfy MCP jail, got: {text}"
    );
}

/// Narrowed jail — negative case (bba6fbf cascade fix preserved).
///
/// The same factory worker holding a jailed task must still be able to
/// perform OTHER mutations (here, `task.update` on an unrelated task).
/// Only `task.close` triggers the jail now.
#[tokio::test]
async fn test_factory_worker_non_close_mutation_still_exempt() {
    let (_temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let service = CasService::new(core);
    let _env = FactoryWorkerEnv::enter();

    // Task A: will be leased + jailed (no verification record).
    let jailed = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "Jailed task A",
            "priority": 2,
            "task_type": "task",
        }))))
        .await
        .expect("create A");
    let jailed_id = extract_task_id(&extract_text(jailed))
        .expect("A id")
        .to_string();
    service
        .task(Parameters(task_req(serde_json::json!({
            "action": "start",
            "id": jailed_id.clone(),
        }))))
        .await
        .expect("start A");

    // Task B: unrelated, should still be mutable.
    let other = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "Unrelated task B",
            "priority": 2,
            "task_type": "task",
        }))))
        .await
        .expect("create B");
    let other_id = extract_task_id(&extract_text(other))
        .expect("B id")
        .to_string();

    // An update on task B is a mutating action. With the narrowed jail it
    // must still be allowed for a factory worker even though task A is
    // blocking a hypothetical close.
    let update = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "update",
            "id": other_id,
            "priority": 1,
        }))))
        .await
        .expect("non-close mutation must remain exempt from the narrowed jail");
    let update_text = extract_text(update);
    assert!(
        !update_text.contains("VERIFICATION_JAIL_BLOCKED"),
        "update on unrelated task must not be blocked: {update_text}"
    );
}

/// Full happy path: hook clears jail, verifier writes approved row, close
/// succeeds.
///
/// This simulates the post-pre_tool-hook state. The hook path itself is
/// covered by the e2e test noted in the section header; here we lock in
/// that close_ops.rs correctly observes hook-clearance + approved row and
/// completes the close cleanly.
#[tokio::test]
async fn test_task_close_succeeds_after_verifier_clearance() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();
    let verification_store = open_verification_store(&cas_dir).unwrap();
    let service = CasService::new(core);
    let _env = FactoryWorkerEnv::enter();

    let created = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "Post-hook clearance happy path",
            "priority": 2,
            "task_type": "task",
        }))))
        .await
        .expect("create");
    let id = extract_task_id(&extract_text(created))
        .expect("id")
        .to_string();
    service
        .task(Parameters(task_req(serde_json::json!({
            "action": "start",
            "id": id.clone(),
        }))))
        .await
        .expect("start");

    // Simulate the pre_tool hook: clear pending_verification on the agent's
    // jailed task. (The real hook sets this flag first when close is
    // attempted; here we bypass that attempt since it's covered by
    // test_factory_worker_close_hits_narrowed_jail above.)
    let mut task = task_store.get(&id).expect("task fetch");
    task.pending_verification = false;
    task.updated_at = chrono::Utc::now();
    task_store.update(&task).expect("clear pending_verification");

    // Simulate the task-verifier subagent writing an approved verification
    // row via mcp__cas__verification add. This is what the hook+subagent
    // sequence produces on a successful verification run.
    let ver = Verification::approved(
        "ver-9a3a-cleared".to_string(),
        id.clone(),
        "Simulated: hook cleared jail, subagent approved work".to_string(),
    );
    verification_store.add(&ver).expect("record approval");

    // Close must now succeed cleanly — the narrowed jail sees an approved
    // verification and lets it through, close_ops records the closure.
    let closed = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "close",
            "id": id.clone(),
            "reason": "Completed after verifier clearance.",
        }))))
        .await
        .expect("close should succeed after hook cleared jail + approved row");
    let close_text = extract_text(closed);
    assert!(
        close_text.to_lowercase().contains("closed"),
        "successful close response must mention closure: {close_text}"
    );

    let final_task = task_store.get(&id).expect("task after close");
    assert_eq!(
        final_task.status,
        cas::types::TaskStatus::Closed,
        "task must be persisted as Closed after the successful close"
    );
}

/// cas-c29a: verification jail within-task deadlock.
///
/// A task enters `pending_verification` on the first close attempt and the
/// dispatch-request row is persisted in `Error` status. If the task-verifier
/// subagent crashes or is never spawned, that row stays stale forever and
/// every close retry returns `VERIFICATION REQUIRED` in a loop.
///
/// This test fabricates a dispatch-request row with a `created_at` older than
/// the 10-minute jail timeout, then calls close again. Expected: close
/// auto-escalates — returns `VERIFICATION TIMED OUT`, clears
/// `pending_verification`, and replaces the stale row with a timeout diagnostic.
#[tokio::test]
async fn test_close_auto_escalates_stale_verification_dispatch() {
    let (temp, service) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");

    let verification_store = open_verification_store(&cas_dir).unwrap();
    let task_store = open_task_store(&cas_dir).unwrap();

    // Create + start task.
    let req = TaskCreateRequest {
        title: "Stuck in verification jail".to_string(),
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
        execution_note: None,
        epic: None,
    };
    let result = service
        .cas_task_create(Parameters(req))
        .await
        .expect("task_create");
    let id = extract_task_id(&extract_text(result))
        .expect("task id")
        .to_string();
    let _ = service
        .cas_task_start(Parameters(IdRequest { id: id.clone() }))
        .await
        .expect("task_start");

    // First close — sets pending_verification and writes dispatch-request row.
    let _ = service
        .cas_task_close(Parameters(TaskCloseRequest {
            id: id.clone(),
            reason: Some("Completed".to_string()),
            bypass_code_review: None,
code_review_findings: None,
        }))
        .await
        .expect("first close returns a result");

    let task_after_first = task_store.get(&id).expect("task exists");
    assert!(
        task_after_first.pending_verification,
        "first close must set pending_verification"
    );

    // Age the dispatch row beyond the 10-minute jail timeout.
    let mut dispatch = verification_store
        .get_latest_for_task(&id)
        .expect("get dispatch row")
        .expect("dispatch row exists");
    assert_eq!(dispatch.status, cas::types::VerificationStatus::Error);
    assert!(dispatch.summary.starts_with("Dispatch requested"));
    dispatch.created_at = chrono::Utc::now() - chrono::Duration::seconds(700);
    verification_store
        .update(&dispatch)
        .expect("age dispatch row");

    // Second close — should auto-escalate instead of looping.
    let result = service
        .cas_task_close(Parameters(TaskCloseRequest {
            id: id.clone(),
            reason: Some("Completed".to_string()),
            bypass_code_review: None,
code_review_findings: None,
        }))
        .await
        .expect("second close returns a result");
    let text = extract_text(result);
    assert!(
        text.contains("VERIFICATION TIMED OUT"),
        "retry after timeout must report escalation, got: {text}"
    );
    assert!(
        !text.contains("VERIFICATION REQUIRED"),
        "escalation must not fall back to the standard jail message"
    );

    // pending_verification must be cleared so the task is no longer jailed.
    let task_after_escalation = task_store.get(&id).expect("task exists");
    assert!(
        !task_after_escalation.pending_verification,
        "auto-escalation must clear pending_verification"
    );

    // The dispatch row should have been updated with a timeout diagnostic.
    let timed_out = verification_store
        .get_latest_for_task(&id)
        .expect("get row")
        .expect("row exists");
    assert_eq!(timed_out.status, cas::types::VerificationStatus::Error);
    assert!(
        timed_out.summary.contains("timed out"),
        "stale dispatch row must be rewritten with timeout diagnostic: {}",
        timed_out.summary
    );
}
