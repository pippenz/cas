use std::sync::{Mutex, MutexGuard, OnceLock};
use tempfile::TempDir;

use cas::mcp::CasCore;
use cas::store::{
    open_agent_store, open_rule_store, open_skill_store, open_store, open_task_store,
};
use cas::types::Agent;

/// Shared process-wide lock for tests that mutate environment variables
/// (`CAS_AGENT_ROLE`, factory harness vars, etc.). Cargo runs integration
/// tests concurrently by default and env vars are a global, so without
/// this lock one test's `ScopedSupervisorEnv::new()` can be clobbered by
/// another test's `setup_cas()` mid-flight, silently flipping the first
/// test into the non-supervisor branch and producing nondeterministic
/// failures (cas-3bd4: the bypass close test was flaking as the Skipped
/// verification row lost the race to the dispatch Error row).
///
/// All env-sensitive tests must acquire this guard for the full duration
/// of their test body, **after** calling `setup_cas` — see the doc on
/// [`setup_cas`] for the ordering contract.
///
/// The guard also recovers from poisoning: a prior panic must not prevent
/// subsequent tests from acquiring the lock.
pub(crate) fn env_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Helper to create an initialized CAS environment.
///
/// **Ordering contract with `env_test_lock()`:** Tests that need to hold
/// the env lock MUST call `setup_cas()` first and acquire
/// `env_test_lock()` immediately afterwards. `setup_cas` briefly acquires
/// the lock itself for its env mutations, so acquiring it in the test
/// body before calling `setup_cas` would deadlock (std `Mutex` is not
/// re-entrant). With the setup-then-lock order, the two acquisitions
/// happen in series and tests composing `setup_cas` with
/// `ScopedSupervisorEnv` stay race-free relative to other tests that
/// also call `setup_cas`.
pub(crate) fn setup_cas() -> (TempDir, CasCore) {
    // Clear factory env vars that leak from parent process (e.g., running
    // inside a factory supervisor session). Without this, is_supervisor_from_env()
    // returns true and the assignee_inactive bypass skips verification checks.
    //
    // cas-3bd4: acquire the shared env lock for the duration of these
    // mutations. Other tests that also call `setup_cas` will serialize
    // through this brief critical section, and any test that holds the
    // lock for its body (see `env_test_lock` docs) will block a
    // competing `setup_cas` from clearing env vars mid-test.
    {
        let _env_guard = env_test_lock();
        // SAFETY: we hold the process-wide env lock for the duration of
        // this block; no other test thread can observe a torn env read.
        unsafe {
            std::env::remove_var("CAS_AGENT_ROLE");
            std::env::remove_var("CAS_FACTORY_MODE");
            std::env::remove_var("CAS_FACTORY_SUPERVISOR_CLI");
            std::env::remove_var("CAS_FACTORY_WORKER_CLI");
        }
    }

    let temp = TempDir::new().expect("temp dir should be created");
    let cas_dir = temp.path().join(".cas");
    std::fs::create_dir_all(&cas_dir).expect(".cas dir should be created");

    let store = open_store(&cas_dir).expect("entry store should open");
    store.init().expect("entry store should initialize");

    let task_store = open_task_store(&cas_dir).expect("task store should open");
    task_store.init().expect("task store should initialize");

    let rule_store = open_rule_store(&cas_dir).expect("rule store should open");
    rule_store.init().expect("rule store should initialize");

    let skill_store = open_skill_store(&cas_dir).expect("skill store should open");
    skill_store.init().expect("skill store should initialize");

    let agent_store = open_agent_store(&cas_dir).expect("agent store should open");
    agent_store.init().expect("agent store should initialize");

    // In production, daemon setup handles session registration.
    let session_id = format!("test-session-{}", std::process::id());
    let agent = Agent::new(session_id.clone(), "test-agent".to_string());
    agent_store
        .register(&agent)
        .expect("test agent should register");

    let core = CasCore::with_daemon(cas_dir.clone(), None, None);
    core.set_agent_id_for_testing(session_id);

    (temp, core)
}

/// Extract text from a tool result.
pub(crate) fn extract_text(result: rmcp::model::CallToolResult) -> String {
    result
        .content
        .into_iter()
        .filter_map(|content| match content.raw {
            rmcp::model::RawContent::Text(text) => Some(text.text),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Extract entry ID from "Created entry: {id} - {preview}" format.
pub(crate) fn extract_entry_id(text: &str) -> Option<&str> {
    text.split("Created entry: ")
        .nth(1)
        .and_then(|part| part.split(" - ").next())
}

/// Extract task ID from "Created task: {id} - {title}" output.
pub(crate) fn extract_task_id(text: &str) -> Option<&str> {
    text.split("Created task: ")
        .nth(1)
        .and_then(|part| part.split(" - ").next())
        .or_else(|| {
            text.split('[')
                .nth(1)
                .and_then(|part| part.split(']').next())
        })
}

/// Extract rule ID from output.
pub(crate) fn extract_rule_id(text: &str) -> Option<String> {
    text.split('[')
        .nth(1)
        .and_then(|part| part.split(']').next())
        .map(ToString::to_string)
        .or_else(|| {
            text.split("rule-")
                .nth(1)
                .and_then(|part| part.split(|c: char| !c.is_alphanumeric()).next())
                .map(|id| format!("rule-{id}"))
        })
}

/// Extract skill ID from output.
pub(crate) fn extract_skill_id(text: &str) -> Option<String> {
    text.split("Created skill: ")
        .nth(1)
        .and_then(|part| part.split(" - ").next())
        .filter(|id| id.starts_with("cas-"))
        .map(ToString::to_string)
        .or_else(|| {
            text.split('[')
                .nth(1)
                .and_then(|part| part.split(']').next())
                .map(ToString::to_string)
        })
}
