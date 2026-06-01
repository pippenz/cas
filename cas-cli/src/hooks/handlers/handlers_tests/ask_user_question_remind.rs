//! Tests for cas-e603: PreToolUse `AskUserQuestion` role-routing reminder for
//! factory supervisors.
//!
//! `AskUserQuestion` routes to the human user — but factory supervisors
//! occasionally invoke it intending to reach a worker/teammate. This intercept
//! injects a `permissionDecisionReason` reminder at the exact moment of misuse
//! without blocking the call (`decision=allow`), since the supervisor may
//! genuinely need the human's input.
//!
//! Behaviour:
//!   is_factory_agent && role==supervisor && tool_name=="AskUserQuestion"
//!     → decision="allow", reason contains "[role-routing reminder]"
//!   non-supervisor or different tool → no reminder injected (passthrough)

use crate::hooks::handlers::handle_pre_tool_use;
use cas_core::hooks::types::{HookInput, HookOutput};

// ============================================================================
// Env helpers — serialize on the shared process-wide mutex in mod.rs so that
// concurrent tests across sibling modules don't race on CAS_AGENT_ROLE.
// ============================================================================

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

// ============================================================================
// Input builder
// ============================================================================

fn hook_input(tool_name: &str) -> HookInput {
    HookInput {
        session_id: "test-session".into(),
        cwd: "/test".into(),
        hook_event_name: "PreToolUse".into(),
        tool_name: Some(tool_name.into()),
        tool_input: Some(serde_json::json!({
            "question": "Should I proceed?",
            "options": [{"label": "Yes"}, {"label": "No"}]
        })),
        ..HookInput::default()
    }
}

// ============================================================================
// Result extractor — returns the permissionDecisionReason iff decision=="allow"
// AND the reason contains the role-routing sentinel.
// ============================================================================

fn reminder_reason(out: &HookOutput) -> Option<String> {
    let specific = out.hook_specific_output.as_ref()?;
    let value = serde_json::to_value(specific).ok()?;
    let decision = value.get("permissionDecision")?.as_str()?;
    if decision != "allow" {
        return None;
    }
    let reason = value
        .get("permissionDecisionReason")
        .and_then(|v| v.as_str())
        .map(str::to_string)?;
    if reason.contains("role-routing reminder") {
        Some(reason)
    } else {
        None
    }
}

// ============================================================================
// AC-1: supervisor + AskUserQuestion → allow + reminder injected.
// ============================================================================

#[test]
fn supervisor_ask_user_question_gets_reminder() {
    let _g = super::env_lock();
    let _role = set_role_env(Some("supervisor"));

    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&hook_input("AskUserQuestion"), Some(tmp.path()))
        .expect("handler ok");

    let reason = reminder_reason(&out).expect(
        "supervisor AskUserQuestion must get decision=allow with role-routing reminder",
    );

    // Decision is allow (not a block).
    let specific = out.hook_specific_output.as_ref().unwrap();
    let value = serde_json::to_value(specific).unwrap();
    assert_eq!(
        value.get("permissionDecision").and_then(|v| v.as_str()),
        Some("allow"),
        "AskUserQuestion must not be blocked, got: {value:?}"
    );

    // Reason mentions CAS coordination path so the supervisor knows the fix.
    assert!(
        reason.contains("mcp__cas__coordination"),
        "reminder must mention the CAS coordination tool: {reason}"
    );

    // Reminder is ≤500 bytes (AC-1).
    assert!(
        reason.len() <= 500,
        "reminder must be ≤500 bytes, got {} bytes: {reason}",
        reason.len()
    );
}

// ============================================================================
// AC-3a: non-supervisor (worker) + AskUserQuestion → no reminder.
// ============================================================================

#[test]
fn worker_ask_user_question_no_reminder() {
    let _g = super::env_lock();
    let _role = set_role_env(Some("worker"));

    let tmp = tempfile::tempdir().expect("tempdir");
    let out = handle_pre_tool_use(&hook_input("AskUserQuestion"), Some(tmp.path()))
        .expect("handler ok");

    assert!(
        reminder_reason(&out).is_none(),
        "worker AskUserQuestion must not receive a role-routing reminder"
    );
}

// ============================================================================
// AC-3b: supervisor + different tool → no reminder.
// ============================================================================

#[test]
fn supervisor_other_tool_no_reminder() {
    let _g = super::env_lock();
    let _role = set_role_env(Some("supervisor"));

    let tmp = tempfile::tempdir().expect("tempdir");
    // AskFollowupQuestions is not in FACTORY_AUTO_APPROVE_TOOLS and has no
    // special intercept, so it falls through cleanly.
    let input = HookInput {
        session_id: "test-session".into(),
        cwd: "/test".into(),
        hook_event_name: "PreToolUse".into(),
        tool_name: Some("AskFollowupQuestions".into()),
        tool_input: Some(serde_json::json!({"question": "continue?"})),
        ..HookInput::default()
    };
    let out = handle_pre_tool_use(&input, Some(tmp.path())).expect("handler ok");

    assert!(
        reminder_reason(&out).is_none(),
        "supervisor on a non-AskUserQuestion tool must not receive a role-routing reminder"
    );
}
