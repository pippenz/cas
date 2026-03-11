use std::str::FromStr;

use cas_mux::SupervisorCli;
use cas_types::TaskType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationMode {
    Required,
    Bypassed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerificationPolicy {
    pub task_mode: VerificationMode,
    pub epic_mode: VerificationMode,
}

impl VerificationPolicy {
    pub fn task_required(self) -> bool {
        self.task_mode == VerificationMode::Required
    }

    pub fn epic_required(self) -> bool {
        self.epic_mode == VerificationMode::Required
    }
}

pub fn parse_harness(value: &str) -> Option<SupervisorCli> {
    SupervisorCli::from_str(value).ok()
}

pub fn worker_harness_from_env() -> SupervisorCli {
    std::env::var("CAS_FACTORY_WORKER_CLI")
        .ok()
        .and_then(|v| parse_harness(&v))
        .unwrap_or(SupervisorCli::Claude)
}

pub fn supervisor_harness_from_env() -> SupervisorCli {
    std::env::var("CAS_FACTORY_SUPERVISOR_CLI")
        .ok()
        .and_then(|v| parse_harness(&v))
        .unwrap_or(SupervisorCli::Claude)
}

pub fn is_supervisor_from_env() -> bool {
    std::env::var("CAS_AGENT_ROLE")
        .map(|r| r.eq_ignore_ascii_case("supervisor"))
        .unwrap_or(false)
}

pub fn is_worker_from_env() -> bool {
    std::env::var("CAS_AGENT_ROLE")
        .map(|r| r.eq_ignore_ascii_case("worker"))
        .unwrap_or(false)
}

/// Factory verification matrix.
///
/// - Subtasks: required only when worker harness supports subagents.
/// - Epics: required only when supervisor harness supports subagents.
pub fn verification_policy(supervisor: SupervisorCli, worker: SupervisorCli) -> VerificationPolicy {
    let task_mode = if worker.capabilities().supports_subagents {
        VerificationMode::Required
    } else {
        VerificationMode::Bypassed
    };

    let epic_mode = if supervisor.capabilities().supports_subagents {
        VerificationMode::Required
    } else {
        VerificationMode::Bypassed
    };

    VerificationPolicy {
        task_mode,
        epic_mode,
    }
}

pub fn verification_required_for_task_type(task_type: TaskType) -> bool {
    let policy = verification_policy(supervisor_harness_from_env(), worker_harness_from_env());
    match task_type {
        TaskType::Epic => policy.epic_required(),
        _ => policy.task_required(),
    }
}

pub fn is_worker_without_subagents_from_env() -> bool {
    is_worker_from_env() && !worker_harness_from_env().capabilities().supports_subagents
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_claude_claude() {
        let p = verification_policy(SupervisorCli::Claude, SupervisorCli::Claude);
        assert!(p.task_required());
        assert!(p.epic_required());
    }

    #[test]
    fn matrix_claude_codex() {
        let p = verification_policy(SupervisorCli::Claude, SupervisorCli::Codex);
        assert!(!p.task_required());
        assert!(p.epic_required());
    }

    #[test]
    fn matrix_codex_claude() {
        let p = verification_policy(SupervisorCli::Codex, SupervisorCli::Claude);
        assert!(p.task_required());
        assert!(!p.epic_required());
    }

    #[test]
    fn matrix_codex_codex() {
        let p = verification_policy(SupervisorCli::Codex, SupervisorCli::Codex);
        assert!(!p.task_required());
        assert!(!p.epic_required());
    }
}
