//! Post-close / stale-instruction guards (cas-b269).
//!
//! Workers sometimes keep re-verifying and re-closing a task after it is
//! already `Closed` (stale transcript, replayed messages, or an urgent stop
//! that only interrupted the turn without invalidating tool-call paths).
//! These pure helpers enforce action-boundary revalidation:
//!
//! - close of an already-closed task is a no-op success (no re-verify)
//! - verification against a closed task is rejected
//! - urgent supervisor halt blocks further close/verify until a new start
//!
//! Close-merge semantics and product code are out of scope.

use cas_types::TaskStatus;

/// Agent metadata key: when `"1"`, the worker must not run task close or
/// verification MCP until a new `task start` clears it (urgent stop).
pub const HALT_TASK_WORK_META: &str = "halt_task_work";

/// Whether the task is in the terminal closed state for close/verify suppression.
pub fn is_terminal_closed(status: TaskStatus) -> bool {
    status == TaskStatus::Closed
}

/// Idempotent response when `task close` targets an already-closed task.
pub fn already_closed_close_message(task_id: &str) -> String {
    format!(
        "ALREADY CLOSED\n\n\
         Task {task_id} is already Closed. Do not re-verify or re-close it.\n\
         Await a new assignment (`task action=mine`) or an explicit new task start.\n\
         (cas-b269)"
    )
}

/// Error when recording verification against a closed task.
pub fn verification_on_closed_message(task_id: &str) -> String {
    format!(
        "Task {task_id} is already Closed — verification is not allowed. \
         Do not re-verify a closed task; await a new assignment (cas-b269)."
    )
}

/// Error when close/verify is attempted under an urgent halt flag.
pub fn halt_blocks_task_work_message(tool: &str) -> String {
    format!(
        "WORK HALTED: supervisor issued an urgent stop. \
         Refusing `{tool}` until a new task is started. \
         Call `task action=mine` and wait for assignment (cas-b269)."
    )
}

/// True when agent metadata marks task work as halted after urgent stop.
pub fn agent_task_work_halted(metadata: &std::collections::HashMap<String, String>) -> bool {
    metadata
        .get(HALT_TASK_WORK_META)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Heuristic: does this delivered prompt instruct close / re-verify work?
/// Used by tests and optional delivery-time rewrite (not required for MCP gates).
pub fn looks_like_close_or_verify_guidance(text: &str) -> bool {
    let lower = text.to_lowercase();
    const MARKERS: &[&str] = &[
        "task action=close",
        "action=close",
        "re-close",
        "reclose",
        "re-verif",
        "reverify",
        "verification required",
        "verify and close",
        "close the task",
        "close id=",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

/// If guidance still tells a worker to close/verify tasks that are already
/// closed, rewrite to an idle instruction. Returns `None` when no rewrite.
pub fn rewrite_stale_close_guidance(
    text: &str,
    closed_task_ids: &[&str],
) -> Option<String> {
    if closed_task_ids.is_empty() || !looks_like_close_or_verify_guidance(text) {
        return None;
    }
    let mentions_closed = closed_task_ids
        .iter()
        .any(|id| text.contains(id));
    if !mentions_closed {
        return None;
    }
    let ids = closed_task_ids.join(", ");
    Some(format!(
        "STALE GUIDANCE SUPPRESSED (cas-b269)\n\n\
         Task(s) {ids} are already Closed. Do not re-verify or re-close.\n\
         Idle and await a new assignment (`task action=mine`).\n\n\
         --- original message (for audit) ---\n{text}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_types::TaskStatus;
    use std::collections::HashMap;

    #[test]
    fn test_b269_terminal_closed_only_for_closed() {
        assert!(is_terminal_closed(TaskStatus::Closed));
        assert!(!is_terminal_closed(TaskStatus::InProgress));
        assert!(!is_terminal_closed(TaskStatus::Open));
        assert!(!is_terminal_closed(TaskStatus::AwaitingMerge));
        assert!(!is_terminal_closed(TaskStatus::PendingSupervisorReview));
    }

    #[test]
    fn test_b269_already_closed_message_forbids_reclose() {
        let msg = already_closed_close_message("cas-a651");
        assert!(msg.contains("ALREADY CLOSED"));
        assert!(msg.contains("cas-a651"));
        assert!(msg.to_lowercase().contains("do not re-verify"));
        assert!(msg.contains("mine"));
    }

    #[test]
    fn test_b269_verification_on_closed_rejected_message() {
        let msg = verification_on_closed_message("cas-a651");
        assert!(msg.contains("Closed"));
        assert!(msg.contains("cas-a651"));
        assert!(msg.to_lowercase().contains("not allowed") || msg.contains("await"));
    }

    #[test]
    fn test_b269_halt_metadata_detection() {
        let mut meta = HashMap::new();
        assert!(!agent_task_work_halted(&meta));
        meta.insert(HALT_TASK_WORK_META.to_string(), "1".to_string());
        assert!(agent_task_work_halted(&meta));
        meta.insert(HALT_TASK_WORK_META.to_string(), "true".to_string());
        assert!(agent_task_work_halted(&meta));
        meta.insert(HALT_TASK_WORK_META.to_string(), "0".to_string());
        assert!(!agent_task_work_halted(&meta));
    }

    #[test]
    fn test_b269_rewrite_stale_close_guidance_when_task_closed() {
        let original = "Please re-verify and task action=close id=cas-a651 reason=\"done\"";
        let rewritten = rewrite_stale_close_guidance(original, &["cas-a651"])
            .expect("must rewrite stale close guidance");
        assert!(rewritten.contains("STALE GUIDANCE SUPPRESSED"));
        assert!(rewritten.contains("cas-a651"));
        assert!(rewritten.to_lowercase().contains("already closed"));
        assert!(rewritten.contains("original message"));
    }

    #[test]
    fn test_b269_no_rewrite_when_task_still_open_guidance() {
        let original = "task action=close id=cas-open reason=x";
        assert!(rewrite_stale_close_guidance(original, &[]).is_none());
        assert!(rewrite_stale_close_guidance("hello idle", &["cas-a651"]).is_none());
        // Closed id not mentioned in text
        assert!(rewrite_stale_close_guidance(
            "task action=close id=cas-other",
            &["cas-a651"]
        )
        .is_none());
    }

    #[test]
    fn test_b269_looks_like_close_or_verify_guidance() {
        assert!(looks_like_close_or_verify_guidance(
            "run task action=close id=cas-x"
        ));
        assert!(looks_like_close_or_verify_guidance("please re-verify the tip"));
        assert!(!looks_like_close_or_verify_guidance("standing by for work"));
    }
}
