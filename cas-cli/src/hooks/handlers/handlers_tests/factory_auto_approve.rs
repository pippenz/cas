//! Tests for factory-agent filesystem-tool auto-approve in PreToolUse.
//!
//! Root cause: Claude Code 2.1.116 team-mode escalates any "ask" permission
//! decision to the team leader via `Mq4()`, gated on a broken self-check
//! (`UG9()`) that compares the agent's `hP()` to the literal string
//! `"team-lead"`. CAS agents have `agentId = "<name>@<team>"`, so the check
//! always fails, every Write/Edit/Bash escalates, and the supervisor ends
//! up asking itself for approval — self-deadlock.
//!
//! Fix: PreToolUse runs before the classifier. Returning
//! `{permissionDecision:"allow"}` short-circuits the whole decision flow so
//! `Mq4()` never fires. Scope is deliberately narrow — the same filesystem
//! tool list CAS ships in the supervisor/worker `--settings` files.
//!
//! On main, the gate reads role from `CAS_AGENT_ROLE` env (the
//! `HookInput.agent_role` field landed later on the worktree-leak epic).
//! All tests that exercise the gate mutate process env and therefore
//! serialize on a local mutex, matching the pattern in
//! `agent_worktree_block`'s env-fallback test.

use crate::hooks::handlers::handle_pre_tool_use;
use cas_core::hooks::types::HookInput;

fn input_for(tool: &str, file_path: Option<&str>) -> HookInput {
    let tool_input = match file_path {
        Some(p) => serde_json::json!({"file_path": p, "content": "x"}),
        None => serde_json::json!({"command": "echo hi"}),
    };
    HookInput {
        session_id: "test-session".into(),
        cwd: "/test".into(),
        hook_event_name: "PreToolUse".into(),
        tool_name: Some(tool.into()),
        tool_input: Some(tool_input),
        ..HookInput::default()
    }
}

fn allow_reason(out: &cas_core::hooks::types::HookOutput) -> Option<String> {
    let specific = out.hook_specific_output.as_ref()?;
    let value = serde_json::to_value(specific).ok()?;
    let decision = value.get("permissionDecision")?.as_str()?;
    if decision != "allow" {
        return None;
    }
    value
        .get("permissionDecisionReason")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

// ============================================================================
// Positive: factory agents get auto-approve for filesystem tool families.
// ============================================================================

#[test]
fn supervisor_write_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("supervisor"));
    let input = input_for("Write", Some("/tmp/foo.txt"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    let reason = allow_reason(&out).expect("expected allow");
    assert!(
        reason.contains("Factory agent auto-approve"),
        "allow reason should identify the factory bypass: {reason}"
    );
}

#[test]
fn worker_write_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("worker"));
    let input = input_for("Write", Some("/tmp/foo.txt"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(allow_reason(&out).is_some(), "worker Write must auto-approve");
}

#[test]
fn worker_edit_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("worker"));
    let input = input_for("Edit", Some("/tmp/foo.txt"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(allow_reason(&out).is_some(), "worker Edit must auto-approve");
}

#[test]
fn supervisor_bash_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("supervisor"));
    let input = input_for("Bash", None);
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(
        allow_reason(&out).is_some(),
        "supervisor Bash must auto-approve"
    );
}

#[test]
fn supervisor_read_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("supervisor"));
    let input = input_for("Read", Some("/tmp/foo.txt"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(allow_reason(&out).is_some());
}

#[test]
fn supervisor_notebook_edit_is_auto_approved() {
    let _g = env_lock();
    let _role = set_role_env(Some("supervisor"));
    let input = input_for("NotebookEdit", Some("/tmp/n.ipynb"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(allow_reason(&out).is_some());
}

// ============================================================================
// Negatives: must NOT auto-approve outside scope.
// ============================================================================

#[test]
fn solo_user_write_is_not_auto_approved() {
    // CAS_AGENT_ROLE unset — the handler must leave the permission decision
    // to Claude Code's normal flow so user-facing approvals keep working.
    let _g = env_lock();
    let _role = set_role_env(None);
    let input = input_for("Write", Some("/tmp/foo.txt"));
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(
        allow_reason(&out).is_none(),
        "standalone sessions must not get the factory auto-approve"
    );
}

#[test]
fn factory_agent_unknown_tool_is_not_auto_approved() {
    // Tools outside the filesystem allowlist (e.g., Agent, Task, MCP) must
    // fall through so their specialized handling still runs.
    let _g = env_lock();
    let _role = set_role_env(Some("worker"));
    let input = input_for("WebFetch", None);
    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");
    assert!(
        allow_reason(&out).is_none(),
        "WebFetch is not in the factory auto-approve list"
    );
}

// ============================================================================
// Structural invariants documented — not exercised directly in unit tests.
// ============================================================================
//
// The factory auto-approve gate lives AFTER the protection block in
// `handle_pre_tool_use`. That ordering is load-bearing: writing a `.env`
// file (or any file matched by `hooks.pre_tool_use.protection.files` /
// `.patterns`) returns a deny BEFORE the factory auto-approve can fire,
// so the bypass can never silently allow a secret-file write.
//
// The gate also runs AFTER the verification jail, the factory
// `SendMessage` block, the codemap freshness gate, and the
// worktrees_enabled block. All of those short-circuit with deny, so the
// auto-approve never overrides them either.

// ----------------------------------------------------------------------------
// Env helpers — mirror the pattern in agent_worktree_block.rs. Required
// because main's gate reads role from `CAS_AGENT_ROLE`, so every test
// that exercises the gate mutates process env and must serialize.
// ----------------------------------------------------------------------------

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

struct RoleGuard(Option<String>);

impl Drop for RoleGuard {
    fn drop(&mut self) {
        unsafe {
            match &self.0 {
                Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
                None => std::env::remove_var("CAS_AGENT_ROLE"),
            }
        }
    }
}

fn set_role_env(role: Option<&str>) -> RoleGuard {
    let prev = std::env::var("CAS_AGENT_ROLE").ok();
    unsafe {
        match role {
            Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
            None => std::env::remove_var("CAS_AGENT_ROLE"),
        }
    }
    RoleGuard(prev)
}
