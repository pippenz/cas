use std::str::FromStr;

use cas_core::hooks::types::HookInput;
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

/// Prefer the role snapshotted into the `HookInput` by the harness
/// (`cli/hook.rs`) over re-reading the process env. Falls back to env when
/// the field is absent — both because legacy call paths haven't been updated
/// yet and because inline constructors (e.g. tests) often leave it unset.
pub fn is_supervisor(input: &HookInput) -> bool {
    match input.agent_role.as_deref() {
        Some(role) => role.eq_ignore_ascii_case("supervisor"),
        None => is_supervisor_from_env(),
    }
}

/// Worker counterpart of `is_supervisor`. Same env fallback semantics.
pub fn is_worker(input: &HookInput) -> bool {
    match input.agent_role.as_deref() {
        Some(role) => role.eq_ignore_ascii_case("worker"),
        None => is_worker_from_env(),
    }
}

/// True when the input carries *any* factory role (supervisor or worker),
/// regardless of which. Replaces the pattern
/// `std::env::var("CAS_AGENT_ROLE").is_ok()` for callers that just need to
/// know "is this a factory-spawned process?".
pub fn is_factory_agent(input: &HookInput) -> bool {
    match input.agent_role.as_deref() {
        Some(role) => !role.trim().is_empty(),
        None => std::env::var("CAS_AGENT_ROLE")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false),
    }
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
