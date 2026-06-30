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

/// Resolve the effective role for a hook invocation. Prefers the explicit
/// field on `HookInput` (populated by the harness at dispatch time in
/// `cli/hook.rs`). Falls back to the process env `CAS_AGENT_ROLE` when the
/// field is absent OR present-but-blank, so a deserialized payload with
/// `"agent_role": ""` doesn't suppress the env fallback — matches the old
/// `is_ok()` semantics where empty strings never counted as "role set".
fn resolve_role(input: &HookInput) -> Option<String> {
    let field = input
        .agent_role
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    field.or_else(|| {
        std::env::var("CAS_AGENT_ROLE")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    })
}

/// Prefer the role snapshotted into the `HookInput` by the harness
/// (`cli/hook.rs`) over re-reading the process env. Falls back to env when
/// the field is absent or blank — both because legacy call paths haven't
/// been updated yet and because inline constructors (e.g. tests) often
/// leave it unset.
pub fn is_supervisor(input: &HookInput) -> bool {
    resolve_role(input)
        .map(|r| r.eq_ignore_ascii_case("supervisor"))
        .unwrap_or(false)
}

/// Worker counterpart of `is_supervisor`. Same env fallback semantics.
pub fn is_worker(input: &HookInput) -> bool {
    resolve_role(input)
        .map(|r| r.eq_ignore_ascii_case("worker"))
        .unwrap_or(false)
}

/// True when the input carries *any* factory role (supervisor or worker),
/// regardless of which. Replaces the pattern
/// `std::env::var("CAS_AGENT_ROLE").is_ok()` for callers that just need to
/// know "is this a factory-spawned process?".
///
/// Matches the pre-refactor semantics: empty-string and whitespace-only
/// role values are treated as "not a factory agent", consistent with the
/// strict-value expectations documented in `HookInput::agent_role`.
pub fn is_factory_agent(input: &HookInput) -> bool {
    resolve_role(input).is_some()
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

/// Returns the MCP coordination tool name appropriate for the current factory
/// worker's harness. Claude workers use `mcp__cas__coordination`, Codex workers
/// use `mcp__cs__coordination`.
///
/// Use this when building jail/guidance messages that include a suggested tool
/// call the worker can actually execute — the alias depends on which harness
/// is running the MCP server (Claude vs Codex).
///
/// cas-8aaf: Codex MCP servers register under the `cs` alias; Claude MCP
/// servers register under the `cas` alias. Hardcoding `mcp__cas__coordination`
/// in guidance given to Codex workers produces an instruction they cannot follow.
pub fn worker_coordination_tool() -> &'static str {
    if worker_harness_from_env() == SupervisorCli::Codex {
        "mcp__cs__coordination"
    } else {
        "mcp__cas__coordination"
    }
}

/// Returns the MCP verification tool name appropriate for the current
/// supervisor's harness (for embedding in guidance that tells the worker what
/// to ask the supervisor to run).
///
/// Claude supervisors use `mcp__cas__verification`, Codex supervisors use
/// `mcp__cs__verification`.
pub fn supervisor_verification_tool() -> &'static str {
    if supervisor_harness_from_env() == SupervisorCli::Codex {
        "mcp__cs__verification"
    } else {
        "mcp__cas__verification"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ----------------------------------------------------------------------
    // Role-helper tests (field-first, env-fallback).
    // ----------------------------------------------------------------------
    //
    // Most cases don't touch the env at all — they drive agent_role on
    // HookInput. The env-fallback tests serialize through a local mutex to
    // avoid racing with each other within this module.

    fn input_with_role(role: Option<&str>) -> HookInput {
        HookInput {
            agent_role: role.map(str::to_string),
            ..HookInput::default()
        }
    }

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        // Use the shared crate-level env lock so we don't race with sibling
        // test modules that also mutate CAS_AGENT_ROLE / CAS_FACTORY_MODE.
        crate::hooks::test_env_lock()
    }

    struct EnvRoleGuard(Option<String>);
    impl Drop for EnvRoleGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
                    None => std::env::remove_var("CAS_AGENT_ROLE"),
                }
            }
        }
    }
    fn set_env_role(role: Option<&str>) -> EnvRoleGuard {
        let prev = std::env::var("CAS_AGENT_ROLE").ok();
        unsafe {
            match role {
                Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
                None => std::env::remove_var("CAS_AGENT_ROLE"),
            }
        }
        EnvRoleGuard(prev)
    }

    #[test]
    fn is_supervisor_reads_field() {
        assert!(is_supervisor(&input_with_role(Some("supervisor"))));
        assert!(is_supervisor(&input_with_role(Some("SUPERVISOR"))));
        assert!(is_supervisor(&input_with_role(Some("Supervisor"))));
        assert!(!is_supervisor(&input_with_role(Some("worker"))));
        assert!(!is_supervisor(&input_with_role(Some("other"))));
    }

    #[test]
    fn is_worker_reads_field() {
        assert!(is_worker(&input_with_role(Some("worker"))));
        assert!(is_worker(&input_with_role(Some("Worker"))));
        assert!(!is_worker(&input_with_role(Some("supervisor"))));
    }

    #[test]
    fn is_factory_agent_reads_field() {
        // Field-wins path: with any valid role on the input, no env read happens.
        assert!(is_factory_agent(&input_with_role(Some("supervisor"))));
        assert!(is_factory_agent(&input_with_role(Some("worker"))));
    }

    #[test]
    fn blank_field_and_blank_env_is_not_factory_agent() {
        // Empty/whitespace-only values were never valid roles — neither on the
        // field nor in the env. Needs env_lock because the blank-field path
        // falls through to env.
        let _g = env_lock();
        let _role = set_env_role(None);
        assert!(!is_factory_agent(&input_with_role(Some(""))));
        assert!(!is_factory_agent(&input_with_role(Some("   "))));
        assert!(!is_factory_agent(&input_with_role(Some("\t"))));
    }

    #[test]
    fn empty_field_falls_through_to_env() {
        // Regression guard for the P1 correctness fix in cas-18fe review:
        // Some("") must not suppress the env fallback.
        let _g = env_lock();
        let _role = set_env_role(Some("supervisor"));
        assert!(is_supervisor(&input_with_role(Some(""))));
        assert!(is_supervisor(&input_with_role(Some("  "))));
    }

    #[test]
    fn field_wins_over_env() {
        let _g = env_lock();
        let _role = set_env_role(Some("worker"));
        assert!(is_supervisor(&input_with_role(Some("supervisor"))));
        assert!(!is_worker(&input_with_role(Some("supervisor"))));
    }

    #[test]
    fn env_fallback_when_field_absent() {
        // agent_role: None → read CAS_AGENT_ROLE from env.
        let _g = env_lock();
        let _role = set_env_role(Some("worker"));
        assert!(is_worker(&input_with_role(None)));
        assert!(!is_supervisor(&input_with_role(None)));
        assert!(is_factory_agent(&input_with_role(None)));
    }

    #[test]
    fn env_empty_is_not_factory_agent() {
        let _g = env_lock();
        let _role = set_env_role(Some(""));
        assert!(!is_factory_agent(&input_with_role(None)));
    }

    #[test]
    fn env_absent_is_solo_user() {
        let _g = env_lock();
        let _role = set_env_role(None);
        let input = input_with_role(None);
        assert!(!is_supervisor(&input));
        assert!(!is_worker(&input));
        assert!(!is_factory_agent(&input));
    }

    // ----------------------------------------------------------------------
    // Existing matrix tests for verification_policy.
    // ----------------------------------------------------------------------

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

    // ----------------------------------------------------------------------
    // cas-8aaf: MCP alias helpers
    // ----------------------------------------------------------------------

    struct EnvWorkerCliGuard(Option<String>);
    impl Drop for EnvWorkerCliGuard {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(v) => std::env::set_var("CAS_FACTORY_WORKER_CLI", v),
                    None => std::env::remove_var("CAS_FACTORY_WORKER_CLI"),
                }
            }
        }
    }
    fn set_worker_cli(cli: Option<&str>) -> EnvWorkerCliGuard {
        let prev = std::env::var("CAS_FACTORY_WORKER_CLI").ok();
        unsafe {
            match cli {
                Some(v) => std::env::set_var("CAS_FACTORY_WORKER_CLI", v),
                None => std::env::remove_var("CAS_FACTORY_WORKER_CLI"),
            }
        }
        EnvWorkerCliGuard(prev)
    }

    #[test]
    fn worker_coordination_tool_defaults_to_cas_when_unset() {
        let _g = env_lock();
        let _c = set_worker_cli(None);
        assert_eq!(
            super::worker_coordination_tool(),
            "mcp__cas__coordination",
            "no CAS_FACTORY_WORKER_CLI set → default Claude → mcp__cas__coordination"
        );
    }

    #[test]
    fn worker_coordination_tool_returns_cas_for_claude_harness() {
        let _g = env_lock();
        let _c = set_worker_cli(Some("claude"));
        assert_eq!(
            super::worker_coordination_tool(),
            "mcp__cas__coordination",
            "CAS_FACTORY_WORKER_CLI=claude → mcp__cas__coordination"
        );
    }

    #[test]
    fn worker_coordination_tool_returns_cs_for_codex_harness() {
        let _g = env_lock();
        let _c = set_worker_cli(Some("codex"));
        assert_eq!(
            super::worker_coordination_tool(),
            "mcp__cs__coordination",
            "CAS_FACTORY_WORKER_CLI=codex → mcp__cs__coordination"
        );
    }

    #[test]
    fn supervisor_verification_tool_returns_cas_when_supervisor_unset() {
        let _g = env_lock();
        // CAS_FACTORY_SUPERVISOR_CLI unset → defaults to Claude.
        assert_eq!(
            super::supervisor_verification_tool(),
            "mcp__cas__verification",
            "default supervisor harness (Claude) → mcp__cas__verification"
        );
    }
}
