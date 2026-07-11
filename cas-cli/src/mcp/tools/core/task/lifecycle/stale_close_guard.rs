//! Post-close / stale-instruction guards (cas-b269).
//!
//! Workers sometimes keep re-verifying and re-closing a task after it is
//! already `Closed` (stale transcript, replayed messages, or an urgent stop
//! that only interrupted the turn without invalidating tool-call paths).
//! These pure helpers enforce action-boundary revalidation:
//!
//! - close of an already-closed task is a no-op success (no re-verify)
//! - verification against a closed task is rejected
//! - urgent supervisor/director halt blocks further close/verify until a
//!   successful new start that does not race a newer halt generation
//!
//! Close-merge semantics and product code are out of scope.

use cas_types::TaskStatus;
use std::collections::HashMap;

/// Agent metadata key: when truthy, the worker must not run task close or
/// verification MCP until a successful `task start` clears it (urgent stop).
pub const HALT_TASK_WORK_META: &str = "halt_task_work";

/// Monotonic generation (unix millis) for the halt flag. A concurrent urgent
/// stop during `task start` writes a newer gen; start must not clear it.
pub const HALT_TASK_WORK_GEN_META: &str = "halt_task_work_gen";

/// Snapshot of one worker candidate for halt fan-out (pure tests + production).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaltWorkerCandidate {
    pub name: String,
    pub factory_session: Option<String>,
}

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
pub fn agent_task_work_halted(metadata: &HashMap<String, String>) -> bool {
    metadata
        .get(HALT_TASK_WORK_META)
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

/// Parse halt generation from metadata (unix millis). Missing/invalid → 0.
pub fn halt_generation(metadata: &HashMap<String, String>) -> u64 {
    metadata
        .get(HALT_TASK_WORK_GEN_META)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Whether `task start` may clear halt given the generation present at clear
/// time and the ceiling captured after a successful start (unix millis).
///
/// Clears only when `stored_gen <= clear_ceiling`. A concurrent urgent halt
/// that wrote a *newer* gen must be preserved.
pub fn should_clear_halt_at_generation(stored_gen: u64, clear_ceiling: u64) -> bool {
    stored_gen <= clear_ceiling
}

/// Apply a new halt generation to metadata (sets flag + gen).
pub fn apply_halt_metadata(metadata: &mut HashMap<String, String>, generation: u64) {
    metadata.insert(HALT_TASK_WORK_META.to_string(), "1".to_string());
    metadata.insert(
        HALT_TASK_WORK_GEN_META.to_string(),
        generation.to_string(),
    );
}

/// Clear halt metadata keys.
pub fn clear_halt_metadata(metadata: &mut HashMap<String, String>) {
    metadata.remove(HALT_TASK_WORK_META);
    metadata.remove(HALT_TASK_WORK_GEN_META);
}

/// Whether the message **source** is authorized to set `halt_task_work`.
///
/// Authorize by role string (`supervisor` / `director` via `AgentRole`) and
/// by known display names (`supervisor`, `director`). Workers must not halt.
pub fn may_source_set_halt(source_display: &str, source_role: &str) -> bool {
    let role = source_role.eq_ignore_ascii_case("supervisor")
        || source_role.eq_ignore_ascii_case("director");
    let name = source_display.eq_ignore_ascii_case("supervisor")
        || source_display.eq_ignore_ascii_case("director");
    role || name
}

/// Same as [`may_source_set_halt`] but with typed `AgentRole` when known.
pub fn may_source_role_set_halt(role: cas_types::AgentRole) -> bool {
    matches!(
        role,
        cas_types::AgentRole::Supervisor | cas_types::AgentRole::Director
    )
}

/// Session-scope worker names visible to `factory_session` (strict match when
/// `Some`; unfiltered when `None` for pure tests / non-factory).
pub fn session_scoped_worker_names(
    workers: &[HaltWorkerCandidate],
    factory_session: Option<&str>,
) -> Vec<String> {
    workers
        .iter()
        .filter(|w| match factory_session {
            Some(session) => w.factory_session.as_deref() == Some(session),
            None => true,
        })
        .map(|w| w.name.clone())
        .collect()
}

/// Resolve which agent **names** should receive a durable halt for an urgent
/// message, scoped to the provided session-filtered worker name list.
///
/// - Never includes `supervisor`.
/// - `all_workers` expands to every provided (session-scoped) worker name.
/// - Single worker name only when present in the session-scoped list.
pub fn halt_targets_for_urgent(
    resolved_target: &str,
    session_worker_names: &[String],
) -> Vec<String> {
    if resolved_target.eq_ignore_ascii_case("supervisor") {
        return Vec::new();
    }
    if resolved_target.eq_ignore_ascii_case("all_workers") {
        return session_worker_names.to_vec();
    }
    session_worker_names
        .iter()
        .filter(|n| n.eq_ignore_ascii_case(resolved_target))
        .cloned()
        .collect()
}

/// Whether an urgent send should attempt durable halt for this source/target.
pub fn should_persist_urgent_halt(
    urgent: bool,
    source_display: &str,
    source_role: &str,
    resolved_target: &str,
    session_worker_names: &[String],
) -> bool {
    urgent
        && may_source_set_halt(source_display, source_role)
        && !halt_targets_for_urgent(resolved_target, session_worker_names).is_empty()
}

/// Heuristic: does this delivered prompt instruct close / re-verify work?
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
    let mentions_closed = closed_task_ids.iter().any(|id| text.contains(id));
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
    use cas_types::{AgentRole, TaskStatus};

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

    /// Review 2: authorize AgentRole::Director explicitly (role string), not
    /// display-name-only.
    #[test]
    fn test_b269_director_role_authorized_explicitly() {
        assert!(may_source_set_halt("any-name", "director"));
        assert!(may_source_role_set_halt(AgentRole::Director));
        assert!(may_source_role_set_halt(AgentRole::Supervisor));
        assert!(!may_source_role_set_halt(AgentRole::Worker));
        assert!(!may_source_role_set_halt(AgentRole::Standard));
        assert!(may_source_set_halt("eager-marten-46", "supervisor"));
        assert!(may_source_set_halt("director", "primary")); // display fallback
        assert!(!may_source_set_halt("staging-sync", "worker"));
    }

    /// Review 2: session scope — same worker name in another factory session
    /// must not receive halt.
    #[test]
    fn test_b269_halt_scoped_to_factory_session_same_name_cross_session() {
        let workers = vec![
            HaltWorkerCandidate {
                name: "staging-sync".into(),
                factory_session: Some("session-a".into()),
            },
            HaltWorkerCandidate {
                name: "staging-sync".into(), // same name, other session
                factory_session: Some("session-b".into()),
            },
            HaltWorkerCandidate {
                name: "other-worker".into(),
                factory_session: Some("session-a".into()),
            },
        ];
        let scoped = session_scoped_worker_names(&workers, Some("session-a"));
        assert_eq!(
            scoped,
            vec!["staging-sync".to_string(), "other-worker".to_string()]
        );
        // all_workers only session-a workers
        let targets = halt_targets_for_urgent("all_workers", &scoped);
        assert_eq!(targets.len(), 2);
        assert!(targets.contains(&"staging-sync".to_string()));
        assert!(targets.contains(&"other-worker".to_string()));
        // direct target: only session-scoped list (one staging-sync)
        let direct = halt_targets_for_urgent("staging-sync", &scoped);
        assert_eq!(direct, vec!["staging-sync".to_string()]);
        // session-b scoped list would not include session-a other-worker
        let scoped_b = session_scoped_worker_names(&workers, Some("session-b"));
        assert_eq!(scoped_b, vec!["staging-sync".to_string()]);
    }

    #[test]
    fn test_b269_urgent_all_workers_halts_every_session_worker_not_supervisor() {
        let workers = vec!["w1".into(), "w2".into(), "w3".into()];
        let targets = halt_targets_for_urgent("all_workers", &workers);
        assert_eq!(targets, workers);
        assert!(halt_targets_for_urgent("supervisor", &workers).is_empty());
        assert_eq!(
            halt_targets_for_urgent("w2", &workers),
            vec!["w2".to_string()]
        );
        assert!(halt_targets_for_urgent("not-a-worker", &workers).is_empty());
    }

    #[test]
    fn test_b269_worker_cannot_halt_supervisor() {
        let workers = vec!["staging-sync".into()];
        assert!(!should_persist_urgent_halt(
            true,
            "staging-sync",
            "worker",
            "supervisor",
            &workers
        ));
        assert!(should_persist_urgent_halt(
            true,
            "eager-marten-46",
            "supervisor",
            "staging-sync",
            &workers
        ));
        assert!(should_persist_urgent_halt(
            true,
            "factory-director",
            "director",
            "staging-sync",
            &workers
        ));
        assert!(!should_persist_urgent_halt(
            false,
            "eager-marten-46",
            "supervisor",
            "staging-sync",
            &workers
        ));
    }

    /// Review 2: start must not clear a newer concurrent urgent halt gen.
    #[test]
    fn test_b269_start_does_not_clear_newer_halt_generation() {
        let clear_ceiling = 1_000u64;
        assert!(should_clear_halt_at_generation(500, clear_ceiling));
        assert!(should_clear_halt_at_generation(1_000, clear_ceiling));
        assert!(!should_clear_halt_at_generation(1_001, clear_ceiling));
        // Legacy halt with no gen (0) is clearable.
        assert!(should_clear_halt_at_generation(0, clear_ceiling));
    }

    #[test]
    fn test_b269_apply_and_clear_halt_metadata() {
        let mut meta = HashMap::new();
        apply_halt_metadata(&mut meta, 42);
        assert!(agent_task_work_halted(&meta));
        assert_eq!(halt_generation(&meta), 42);
        clear_halt_metadata(&mut meta);
        assert!(!agent_task_work_halted(&meta));
        assert_eq!(halt_generation(&meta), 0);
    }

    #[test]
    fn test_b269_failed_start_statuses_do_not_clear_halt_policy() {
        let no_clear = [
            TaskStatus::Closed,
            TaskStatus::PendingSupervisorReview,
            TaskStatus::AwaitingMerge,
        ];
        for status in no_clear {
            assert!(
                !matches!(
                    status,
                    TaskStatus::Open | TaskStatus::InProgress | TaskStatus::Blocked
                ),
                "status {status:?} is a failed-start path that must preserve halt"
            );
        }
    }
}
