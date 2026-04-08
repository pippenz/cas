use crate::support::*;
use cas::mcp::tools::*;
use cas::store::{open_agent_store, open_task_store, open_verification_store, open_worktree_store};
use cas::types::{Verification, VerificationType, Worktree};
use rmcp::handler::server::wrapper::Parameters;

#[tokio::test]
async fn test_task_close_blocked_without_verification() {
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
