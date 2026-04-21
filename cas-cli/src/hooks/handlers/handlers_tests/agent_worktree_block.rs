//! Tests for supervisor Agent(isolation="worktree") block (cas-483b / EPIC cas-7c88).
//!
//! The gate fires only when ALL of:
//!   - `CAS_AGENT_ROLE=supervisor` in the process env, AND
//!   - `tool_name == "Agent"`, AND
//!   - `tool_input.isolation == "worktree"`.
//!
//! Any other combination must pass through untouched so that workers, solo users,
//! and non-worktree Agent calls (Explore, code review personas, task-verifier)
//! keep working.

use crate::hooks::handlers::handle_pre_tool_use;
use cas_core::hooks::types::HookInput;

/// Serialize env-mutating tests within THIS module so `CAS_AGENT_ROLE` flips
/// don't race between parallel tests in `agent_worktree_block`. Note: this lock
/// does NOT protect against races with other test modules that also mutate
/// `CAS_AGENT_ROLE` (e.g. `close_ops`) — those modules hold a separate static.
/// Cross-module unification is tracked as a follow-up (flaky if parallel).
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::{Mutex, OnceLock};
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// RAII guard that restores `CAS_AGENT_ROLE` to its prior value on drop —
/// panic-safe (unlike a manual restore after the closure body, which is
/// skipped on unwind).
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

fn set_role(role: Option<&str>) -> RoleGuard {
    let prev = std::env::var("CAS_AGENT_ROLE").ok();
    unsafe {
        match role {
            Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
            None => std::env::remove_var("CAS_AGENT_ROLE"),
        }
    }
    RoleGuard(prev)
}

/// Run `f` with `CAS_AGENT_ROLE` set to the given value for the duration of
/// the call. Panic-safe via `RoleGuard`'s Drop impl.
fn with_role<F, R>(role: Option<&str>, f: F) -> R
where
    F: FnOnce() -> R,
{
    let _guard = set_role(role);
    f()
}

fn agent_input(isolation: Option<&str>) -> HookInput {
    let tool_input = match isolation {
        Some(iso) => serde_json::json!({
            "subagent_type": "general-purpose",
            "prompt": "do work",
            "isolation": iso,
        }),
        None => serde_json::json!({
            "subagent_type": "Explore",
            "prompt": "explore",
        }),
    };
    HookInput {
        session_id: "test-session".to_string(),
        cwd: "/test".to_string(),
        hook_event_name: "PreToolUse".to_string(),
        tool_name: Some("Agent".to_string()),
        tool_input: Some(tool_input),
        tool_response: None,
        transcript_path: None,
        permission_mode: None,
        tool_use_id: None,
        user_prompt: None,
        source: None,
        reason: None,
        subagent_type: None,
        subagent_prompt: None,
    }
}

/// Extract the pre-tool deny reason from a HookOutput, or None if not a deny.
fn deny_reason(out: &cas_core::hooks::types::HookOutput) -> Option<String> {
    let specific = out.hook_specific_output.as_ref()?;
    let value = serde_json::to_value(specific).ok()?;
    let decision = value.get("permissionDecision")?.as_str()?;
    if decision != "deny" {
        return None;
    }
    value
        .get("permissionDecisionReason")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

// ============================================================================
// Positive: the one case that must be blocked.
// ============================================================================

#[test]
fn supervisor_agent_worktree_is_denied() {
    let _g = env_lock();
    let input = agent_input(Some("worktree"));
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    let reason = deny_reason(&out).expect("expected deny");
    // Message must point at the coordination escape hatch and explain the leak.
    assert!(
        reason.contains("mcp__cas__coordination"),
        "deny message should direct to coordination: {reason}"
    );
    assert!(
        reason.contains("spawn_workers"),
        "deny message should name spawn_workers action: {reason}"
    );
    assert!(
        reason.contains("leak") || reason.contains("cleaned up"),
        "deny message should explain the leak reason: {reason}"
    );
}

// ============================================================================
// Negatives: must NOT fire.
// ============================================================================

#[test]
fn worker_agent_worktree_is_allowed() {
    let _g = env_lock();
    let input = agent_input(Some("worktree"));
    let out = with_role(Some("worker"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "worker should not hit the supervisor gate"
    );
}

#[test]
fn supervisor_agent_without_isolation_is_allowed() {
    let _g = env_lock();
    let input = agent_input(None);
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "supervisor Agent() without isolation must still work (Explore, reviews, verifier)"
    );
}

#[test]
fn supervisor_agent_isolation_other_value_is_allowed() {
    let _g = env_lock();
    // Only the literal string "worktree" triggers the block. Other values (future
    // isolation modes, bogus strings) pass through.
    let input = agent_input(Some("sandbox"));
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "non-worktree isolation values are out of scope for this gate"
    );
}

#[test]
fn solo_user_agent_worktree_is_allowed() {
    let _g = env_lock();
    // No CAS_AGENT_ROLE at all — plain `claude` session outside factory.
    let input = agent_input(Some("worktree"));
    let out = with_role(None, || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "solo users have no supervisor discipline to enforce"
    );
}

#[test]
fn supervisor_task_verifier_with_worktree_is_allowed() {
    let _g = env_lock();
    // Task-verifier is the only supervisor-initiated Agent subagent that may
    // need isolation without being caught by the leak gate. Blocking it would
    // strand the supervisor in pending_verification.
    let input = HookInput {
        session_id: "test-session".to_string(),
        cwd: "/test".to_string(),
        hook_event_name: "PreToolUse".to_string(),
        tool_name: Some("Agent".to_string()),
        tool_input: Some(serde_json::json!({
            "subagent_type": "task-verifier",
            "prompt": "Verify task cas-xyz",
            "isolation": "worktree",
        })),
        tool_response: None,
        transcript_path: None,
        permission_mode: None,
        tool_use_id: None,
        user_prompt: None,
        source: None,
        reason: None,
        subagent_type: None,
        subagent_prompt: None,
    };
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "task-verifier must be exempt from the worktree gate so verification-jail unjail paths work"
    );
}

#[test]
fn supervisor_agent_isolation_non_string_is_allowed() {
    // A JSON number, bool, or object in `isolation` must not collapse into
    // `Some("worktree")`. Documents that the match is string-literal only.
    let _g = env_lock();
    let input = HookInput {
        session_id: "test".into(),
        cwd: "/test".into(),
        hook_event_name: "PreToolUse".into(),
        tool_name: Some("Agent".into()),
        tool_input: Some(serde_json::json!({
            "subagent_type": "general-purpose",
            "prompt": "do work",
            "isolation": 42,
        })),
        tool_response: None,
        transcript_path: None,
        permission_mode: None,
        tool_use_id: None,
        user_prompt: None,
        source: None,
        reason: None,
        subagent_type: None,
        subagent_prompt: None,
    };
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "non-string isolation values fall through via and_then(as_str)"
    );
}

#[test]
fn supervisor_agent_missing_tool_input_is_allowed() {
    // `tool_input: None` must not panic or deny — the `and_then` chain
    // gracefully yields `None` for isolation.
    let _g = env_lock();
    let input = HookInput {
        session_id: "test".into(),
        cwd: "/test".into(),
        hook_event_name: "PreToolUse".into(),
        tool_name: Some("Agent".into()),
        tool_input: None,
        tool_response: None,
        transcript_path: None,
        permission_mode: None,
        tool_use_id: None,
        user_prompt: None,
        source: None,
        reason: None,
        subagent_type: None,
        subagent_prompt: None,
    };
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "missing tool_input must pass through without denying"
    );
}

#[test]
fn supervisor_non_agent_tool_is_allowed() {
    let _g = env_lock();
    // Even with isolation="worktree" in tool_input, non-Agent tools must pass.
    let mut input = agent_input(Some("worktree"));
    input.tool_name = Some("Bash".to_string());
    let out = with_role(Some("supervisor"), || {
        handle_pre_tool_use(&input, None).expect("handler ok")
    });
    assert!(
        deny_reason(&out).is_none(),
        "gate is Agent-scoped; Bash etc. must not be affected"
    );
}
