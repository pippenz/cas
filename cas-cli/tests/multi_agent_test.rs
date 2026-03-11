//! Multi-agent concurrent access integration tests
//!
//! Tests concurrent task claiming, lease management, and agent coordination.

use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tempfile::TempDir;

use cas::store::{AgentStore, TaskStore, init_cas_dir, open_agent_store, open_task_store};
use cas::types::{Agent, AgentType, Task, TaskStatus};

fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
}

/// Helper to create a test environment
fn setup_test_env() -> (TempDir, std::path::PathBuf) {
    let _guard = serial_lock();
    let temp = TempDir::new().unwrap();
    let cas_dir = init_cas_dir(temp.path()).unwrap();
    (temp, cas_dir)
}

/// Helper to create and register a test agent
fn create_test_agent(store: &dyn AgentStore, name: &str) -> String {
    let id = Agent::generate_fallback_id();
    let mut agent = Agent::new(id.clone(), name.to_string());
    agent.agent_type = AgentType::Worker;
    store.register(&agent).unwrap();
    id
}

/// Helper to create a test task
fn create_test_task(store: &dyn TaskStore, title: &str) -> String {
    let id = store.generate_id().unwrap();
    let task = Task::new(id.clone(), title.to_string());
    store.add(&task).unwrap();
    id
}

#[path = "multi_agent_test_cases/tests.rs"]
mod tests;
