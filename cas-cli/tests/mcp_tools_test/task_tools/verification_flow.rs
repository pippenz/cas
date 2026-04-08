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

/// Narrowed jail — negative case (bba6fbf cascade fix preserved).
///
/// The same factory worker holding a jailed task must still be able to
/// perform OTHER mutations (here, `task.update` on an unrelated task).
/// Only `task.close` triggers the jail now.
#[tokio::test]
async fn test_factory_worker_non_close_mutation_still_exempt() {
    let (_temp, core) = setup_cas();
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
