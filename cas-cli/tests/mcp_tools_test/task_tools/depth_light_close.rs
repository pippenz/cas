//! cas-6538 (EPIC cas-1255 — per-task depth speed mode): close-gate light-skip.
//!
//! These tests pin the enforcement layer: a `depth=light` task skips the two
//! *rigor* gates at close time — the verification jail and the P0 code-review
//! gate (including the supervisor-review queue hop that IS the P0 gate under
//! `owner = "supervisor"`) — and records an auditable decision note. The
//! regression guard is the whole point: `depth=deep`/unset must arm the jail
//! and pend for supervisor review exactly as today, proven by paired tests
//! that fail if the skip leaks to deep.

use crate::support::*;
use cas::mcp::tools::*;
use cas::mcp::{CasCore, CasService};
use cas::store::open_task_store;
use cas::types::TaskStatus;
use rmcp::handler::server::wrapper::Parameters;
use std::process::Command;

/// Build a `cas_mcp::TaskRequest` from test JSON.
fn task_req(value: serde_json::Value) -> cas_mcp::TaskRequest {
    serde_json::from_value(value).expect("TaskRequest should deserialize from test JSON")
}

/// Typed `TaskCreateRequest` with the given depth (`None` = unset/default).
/// The solo tests drive `cas_task_close` *directly* on the core — bypassing
/// the dispatch-layer MCP jail (`authorize_agent_action`) the same way the
/// existing verification_flow tests do — to isolate the close_ops gate.
fn create_req(title: &str, depth: Option<&str>) -> TaskCreateRequest {
    TaskCreateRequest {
        depth: depth.map(str::to_string),
        title: title.to_string(),
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
    }
}

/// RAII guard that installs factory-worker env vars and clears them on drop.
struct FactoryWorkerGuard;

impl FactoryWorkerGuard {
    fn enter() -> Self {
        unsafe {
            std::env::set_var("CAS_AGENT_ROLE", "worker");
            std::env::set_var("CAS_FACTORY_MODE", "1");
        }
        Self
    }
}

impl Drop for FactoryWorkerGuard {
    fn drop(&mut self) {
        unsafe {
            std::env::remove_var("CAS_AGENT_ROLE");
            std::env::remove_var("CAS_FACTORY_MODE");
        }
    }
}

/// Supervisor-owned review + verification disabled, so a factory-worker close
/// reaches the supervisor-review transition rather than the verification jail.
fn write_supervisor_review_config(cas_dir: &std::path::Path) {
    let toml = r#"
[verification]
enabled = false

[code_review]
owner = "supervisor"
"#;
    std::fs::write(cas_dir.join("config.toml"), toml).expect("config.toml should be writable");
}

/// Init a git repo at `project_root` with one staged Rust change so that
/// `has_reviewable_changes()` returns true.
fn init_git_repo_with_staged_changes(project_root: &std::path::Path) {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(project_root)
            .output()
            .expect("git command should run")
    };
    git(&["init", "-b", "main"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    std::fs::write(project_root.join("base.rs"), "fn main() {}\n").expect("write should succeed");
    git(&["add", "base.rs"]);
    git(&["commit", "-m", "init"]);
    std::fs::write(
        project_root.join("feature.rs"),
        "pub fn feature() -> u32 { 42 }\n",
    )
    .expect("write should succeed");
    git(&["add", "feature.rs"]);
}

// ---------------------------------------------------------------------------
// Solo (non-factory) close — verification jail branch, exercised directly on
// `cas_task_close`.
//
// setup_cas() leaves verification ENABLED (no config written), so the close_ops
// jail arms for deep and skips for light. These call the core method directly
// (bypassing the dispatch-layer MCP jail) to isolate the close_ops gate — the
// same convention every existing verification_flow close test uses.
// ---------------------------------------------------------------------------

/// AC: closing a `depth=light` task skips the verification jail — close
/// succeeds, `pending_verification` stays false, and a decision note records
/// the skip.
#[tokio::test]
async fn test_light_depth_solo_close_skips_verification_jail() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();

    let created = core
        .cas_task_create(Parameters(create_req("light task — solo close", Some("light"))))
        .await
        .expect("task_create should succeed");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    core.cas_task_start(Parameters(IdRequest { id: id.clone() }))
        .await
        .expect("task_start should succeed");

    let close_text = extract_text(
        core.cas_task_close(Parameters(TaskCloseRequest {
            id: id.clone(),
            reason: Some("Feel-driven pass complete.".to_string()),
            bypass_code_review: None,
            code_review_findings: None,
        }))
        .await
        .expect("task_close should return a result"),
    );

    assert!(
        !close_text.contains("VERIFICATION REQUIRED"),
        "light close must not arm the verification jail: {close_text}"
    );
    assert!(
        close_text.contains("Closed"),
        "light close should report success: {close_text}"
    );

    let task = task_store.get(&id).expect("task should exist");
    assert_eq!(
        task.status,
        TaskStatus::Closed,
        "light task must transition to Closed"
    );
    assert!(
        !task.pending_verification,
        "light close must leave pending_verification false"
    );
    // Auditable decision note recording exactly what was skipped and why.
    assert!(
        task.notes.contains("depth=light")
            && task.notes.to_lowercase().contains("decision")
            && task.notes.contains("verification jail")
            && task.notes.contains("code-review gate"),
        "light close must record an auditable decision note naming both skipped \
         gates: {}",
        task.notes
    );
}

/// Regression guard: a `depth=deep` (explicit) task still arms the jail.
/// Fails if the light-skip leaks to deep.
#[tokio::test]
async fn test_deep_depth_solo_close_still_arms_verification_jail() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();

    let created = core
        .cas_task_create(Parameters(create_req("deep task — solo close", Some("deep"))))
        .await
        .expect("task_create should succeed");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    core.cas_task_start(Parameters(IdRequest { id: id.clone() }))
        .await
        .expect("task_start should succeed");

    let close_text = extract_text(
        core.cas_task_close(Parameters(TaskCloseRequest {
            id: id.clone(),
            reason: Some("Done.".to_string()),
            bypass_code_review: None,
            code_review_findings: None,
        }))
        .await
        .expect("task_close should return a result"),
    );

    assert!(
        close_text.contains("VERIFICATION REQUIRED"),
        "deep close must still arm the verification jail: {close_text}"
    );

    let task = task_store.get(&id).expect("task should exist");
    assert!(
        task.pending_verification,
        "deep close must set pending_verification = true"
    );
    assert_ne!(
        task.status,
        TaskStatus::Closed,
        "deep close must NOT close while verification is pending"
    );
    assert!(
        !task.notes.contains("depth=light"),
        "deep close must not write the light-skip decision note: {}",
        task.notes
    );
}

/// Regression guard: an unset-depth (legacy / NULL→Deep) task behaves like
/// deep — the jail arms. Proves the default reads as Deep at the close gate.
#[tokio::test]
async fn test_unset_depth_solo_close_still_arms_verification_jail() {
    let (temp, core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");
    let task_store = open_task_store(&cas_dir).unwrap();

    let created = core
        .cas_task_create(Parameters(create_req("unset-depth task — solo close", None)))
        .await
        .expect("task_create should succeed");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    core.cas_task_start(Parameters(IdRequest { id: id.clone() }))
        .await
        .expect("task_start should succeed");

    let close_text = extract_text(
        core.cas_task_close(Parameters(TaskCloseRequest {
            id: id.clone(),
            reason: Some("Done.".to_string()),
            bypass_code_review: None,
            code_review_findings: None,
        }))
        .await
        .expect("task_close should return a result"),
    );

    assert!(
        close_text.contains("VERIFICATION REQUIRED"),
        "unset-depth close must arm the jail (NULL→Deep): {close_text}"
    );
    assert!(
        task_store
            .get(&id)
            .expect("task should exist")
            .pending_verification,
        "unset-depth close must set pending_verification = true"
    );
}

// ---------------------------------------------------------------------------
// Factory-worker close under supervisor-owned review — P0 gate branch.
//
// Under `owner = "supervisor"` a worker close with reviewable changes pends
// to PendingSupervisorReview (the queue hop IS the P0 gate). depth=light must
// instead close immediately; depth=deep must still pend.
// ---------------------------------------------------------------------------

/// AC: a `depth=light` factory-worker close with reviewable changes treats the
/// P0 code-review gate as satisfied — the task closes immediately rather than
/// pending for supervisor review, with the decision note recorded.
#[tokio::test]
async fn test_light_depth_factory_close_skips_p0_gate_and_closes() {
    let (temp, _core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");

    write_supervisor_review_config(&cas_dir);
    init_git_repo_with_staged_changes(temp.path());

    let core = CasCore::with_daemon(cas_dir.clone(), None, None);
    let task_store = open_task_store(&cas_dir).unwrap();
    let service = CasService::new(core, None);

    let _worker_guard = FactoryWorkerGuard::enter();

    let created = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "light feature task — factory close",
            "priority": 2,
            "task_type": "task",
            "depth": "light",
        }))))
        .await
        .expect("task.create should succeed");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    service
        .task(Parameters(task_req(
            serde_json::json!({ "action": "start", "id": id }),
        )))
        .await
        .expect("task.start should succeed");

    let close_text = extract_text(
        service
            .task(Parameters(task_req(serde_json::json!({
                "action": "close",
                "id": id,
                "reason": "All acceptance criteria met.",
            }))))
            .await
            .expect("task.close should return a result"),
    );

    assert!(
        !close_text.contains("CODE_REVIEW_REQUIRED"),
        "light close must not trip the code-review gate: {close_text}"
    );
    assert!(
        !close_text.contains("pending_supervisor_review")
            && !close_text.contains("supervisor review"),
        "light close must NOT pend for supervisor review — it closes immediately: {close_text}"
    );

    let task = task_store.get(&id).expect("task should exist");
    assert_eq!(
        task.status,
        TaskStatus::Closed,
        "light factory close must transition straight to Closed, not \
         PendingSupervisorReview; got {:?}",
        task.status
    );
    assert!(
        task.notes.contains("depth=light") && task.notes.contains("code-review gate"),
        "light factory close must record the decision note: {}",
        task.notes
    );
}

/// Regression guard: a `depth=deep` factory-worker close with reviewable
/// changes still pends for supervisor review. Fails if the P0-gate skip leaks
/// to deep.
#[tokio::test]
async fn test_deep_depth_factory_close_still_pends_supervisor_review() {
    let (temp, _core) = setup_cas();
    let _env_lock = env_test_lock();
    let cas_dir = temp.path().join(".cas");

    write_supervisor_review_config(&cas_dir);
    init_git_repo_with_staged_changes(temp.path());

    let core = CasCore::with_daemon(cas_dir.clone(), None, None);
    let task_store = open_task_store(&cas_dir).unwrap();
    let service = CasService::new(core, None);

    let _worker_guard = FactoryWorkerGuard::enter();

    let created = service
        .task(Parameters(task_req(serde_json::json!({
            "action": "create",
            "title": "deep feature task — factory close",
            "priority": 2,
            "task_type": "task",
            "depth": "deep",
        }))))
        .await
        .expect("task.create should succeed");
    let id = extract_task_id(&extract_text(created))
        .expect("should have task ID")
        .to_string();

    service
        .task(Parameters(task_req(
            serde_json::json!({ "action": "start", "id": id }),
        )))
        .await
        .expect("task.start should succeed");

    let close_text = extract_text(
        service
            .task(Parameters(task_req(serde_json::json!({
                "action": "close",
                "id": id,
                "reason": "All acceptance criteria met.",
            }))))
            .await
            .expect("task.close should return a result"),
    );

    assert!(
        close_text.contains("supervisor review")
            || close_text.contains("pending_supervisor_review"),
        "deep close must still pend for supervisor review: {close_text}"
    );

    let task = task_store.get(&id).expect("task should exist");
    assert_eq!(
        task.status,
        TaskStatus::PendingSupervisorReview,
        "deep factory close must pend for supervisor review, not close: {:?}",
        task.status
    );
    assert!(
        !task.notes.contains("depth=light"),
        "deep close must not write the light-skip decision note: {}",
        task.notes
    );
}
