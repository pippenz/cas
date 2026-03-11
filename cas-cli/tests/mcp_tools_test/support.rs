use tempfile::TempDir;

use cas::mcp::CasCore;
use cas::store::{
    open_agent_store, open_rule_store, open_skill_store, open_store, open_task_store,
};
use cas::types::Agent;

/// Helper to create an initialized CAS environment.
pub(crate) fn setup_cas() -> (TempDir, CasCore) {
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
