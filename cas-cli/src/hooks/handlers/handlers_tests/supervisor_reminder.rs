//! Tests for cas-55ac: Per-turn UserPromptSubmit reminder for factory supervisors.
//!
//! The `UserPromptSubmit` hook now emits a ≤512B role reminder when
//! `is_factory_agent && role==supervisor`. The reminder refreshes Hard Rules
//! every turn to counter mid-session drift. Non-supervisor and non-factory
//! sessions remain unchanged (returns empty).

use cas_core::hooks::types::{HookInput, HookSpecificOutput};

use crate::hooks::handlers::handle_user_prompt_submit;

fn supervisor_input() -> HookInput {
    HookInput {
        session_id: "test-supervisor-session".to_string(),
        cwd: "/test".to_string(),
        hook_event_name: "UserPromptSubmit".to_string(),
        user_prompt: Some("What tasks are ready?".to_string()),
        agent_role: Some("supervisor".to_string()),
        ..HookInput::default()
    }
}

fn worker_input() -> HookInput {
    HookInput {
        session_id: "test-worker-session".to_string(),
        cwd: "/test".to_string(),
        hook_event_name: "UserPromptSubmit".to_string(),
        user_prompt: Some("What tasks are ready?".to_string()),
        agent_role: Some("worker".to_string()),
        ..HookInput::default()
    }
}

fn non_factory_input() -> HookInput {
    HookInput {
        session_id: "test-solo-session".to_string(),
        cwd: "/test".to_string(),
        hook_event_name: "UserPromptSubmit".to_string(),
        user_prompt: Some("What tasks are ready?".to_string()),
        agent_role: None,
        ..HookInput::default()
    }
}

/// AC1: Supervisor receives a non-empty UserPromptSubmit additionalContext.
/// AC4: Output byte length ≤ 512.
/// Both checked together since they require the same setup.
#[test]
fn supervisor_gets_reminder_within_512_bytes() {
    let input = supervisor_input();
    let output = handle_user_prompt_submit(&input, None).unwrap();

    let context = match &output.hook_specific_output {
        Some(HookSpecificOutput::UserPromptSubmit { additional_context }) => additional_context,
        other => panic!(
            "Expected UserPromptSubmit hookSpecificOutput, got: {other:?}"
        ),
    };

    let byte_len = context.as_bytes().len();
    assert!(
        byte_len <= 512,
        "Supervisor reminder exceeds 512 bytes: {byte_len} bytes\n---\n{context}\n---"
    );
}

/// AC1: The reminder contains all 6 Hard Rule keywords.
#[test]
fn supervisor_reminder_contains_all_6_hard_rule_keywords() {
    let input = supervisor_input();
    let output = handle_user_prompt_submit(&input, None).unwrap();

    let context = match &output.hook_specific_output {
        Some(HookSpecificOutput::UserPromptSubmit { additional_context }) => additional_context,
        other => panic!(
            "Expected UserPromptSubmit hookSpecificOutput, got: {other:?}"
        ),
    };

    // All 6 Hard Rule keywords must appear in the reminder.
    let keywords = [
        "AskUserQuestion",
        "mcp__cas__coordination",
        "SendMessage",
        "close",
        "implement",
        "poll",
    ];
    for kw in keywords {
        assert!(
            context.contains(kw),
            "Supervisor reminder missing keyword '{kw}':\n---\n{context}\n---"
        );
    }
}

/// AC2: Worker sessions return empty (no reminder injected).
#[test]
fn worker_does_not_get_supervisor_reminder() {
    let input = worker_input();
    let output = handle_user_prompt_submit(&input, None).unwrap();

    let json = serde_json::to_string(&output).unwrap();
    assert_eq!(
        json, "{}",
        "Worker session must return empty HookOutput, got: {json}"
    );
}

/// AC2: Non-factory (generic Claude) sessions return empty.
#[test]
fn non_factory_does_not_get_supervisor_reminder() {
    let input = non_factory_input();
    let output = handle_user_prompt_submit(&input, None).unwrap();

    let json = serde_json::to_string(&output).unwrap();
    assert_eq!(
        json, "{}",
        "Non-factory session must return empty HookOutput, got: {json}"
    );
}

/// AC1: Supervisor reminder includes identity placeholders (name, team).
/// The reminder text must contain the identity section so the supervisor
/// knows which role it's been assigned.
#[test]
fn supervisor_reminder_contains_identity_context() {
    let input = supervisor_input();
    let output = handle_user_prompt_submit(&input, None).unwrap();

    let context = match &output.hook_specific_output {
        Some(HookSpecificOutput::UserPromptSubmit { additional_context }) => additional_context,
        other => panic!(
            "Expected UserPromptSubmit hookSpecificOutput, got: {other:?}"
        ),
    };

    // The reminder must include an identity line (supervisor of team ...)
    assert!(
        context.contains("supervisor"),
        "Reminder must identify the role as 'supervisor':\n---\n{context}\n---"
    );
}
