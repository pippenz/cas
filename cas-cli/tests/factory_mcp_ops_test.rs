//! Factory MCP Tool Integration Tests
//!
//! Tests the factory MCP tool handlers (`mcp__cas__coordination`) by constructing
//! a `CasService` with a temp CAS directory and calling `factory()` directly.
//! Verifies input validation, queue side effects, and response formatting.
//!
//! # Running
//! Some tests modify process-global environment variables (`CAS_AGENT_ROLE`,
//! `CAS_FACTORY_WORKER_NAMES`). Run single-threaded to avoid races:
//! ```bash
//! cargo test --test factory_mcp_ops_test -- --nocapture --test-threads=1
//! ```

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use cas::mcp::{CasCore, CasService};
use cas::store::{
    AgentStore, EventStore, PromptQueueStore, SpawnQueueStore, TaskStore, init_cas_dir,
    open_agent_store, open_event_store, open_prompt_queue_store, open_spawn_queue_store,
    open_task_store,
};
use cas::types::{
    Agent, AgentStatus, Event, EventEntityType, EventType, Task, TaskStatus, TaskType,
};
use cas_mcp::types::{CoordinationRequest, FactoryRequest};
use cas_types::AgentRole;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::RawContent;
use tempfile::TempDir;

// =============================================================================
// Test Fixture
// =============================================================================

struct FactoryTestEnv {
    _temp: TempDir,
    cas_root: PathBuf,
    service: CasService,
}

impl FactoryTestEnv {
    fn new() -> Self {
        Self::with_agent_id("test-agent-id")
    }

    fn with_agent_id(agent_id: &str) -> Self {
        let temp = TempDir::new().expect("Failed to create temp dir");
        let cas_root = init_cas_dir(temp.path()).expect("Failed to init CAS dir");

        let core = CasCore::with_daemon(cas_root.clone(), None, None);
        core.set_agent_id_for_testing(agent_id.to_string());
        let service = CasService::new(core, None);

        Self {
            _temp: temp,
            cas_root,
            service,
        }
    }

    fn create_epic(&self, title: &str) -> String {
        let store = self.task_store();
        let id = store.generate_id().expect("generate_id");
        let mut task = Task::new(id.clone(), title.to_string());
        task.task_type = TaskType::Epic;
        store.add(&task).expect("add epic");
        id
    }

    fn register_worker(&self, name: &str) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Worker;
        store.register(&agent).expect("register worker");
        id
    }

    fn register_worker_in_session(&self, name: &str, factory_session: &str) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Worker;
        agent.factory_session = Some(factory_session.to_string());
        store.register(&agent).expect("register worker in session");
        id
    }

    fn register_supervisor_in_session(&self, name: &str, factory_session: &str) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Supervisor;
        agent.factory_session = Some(factory_session.to_string());
        store
            .register(&agent)
            .expect("register supervisor in session");
        id
    }

    fn record_worker_file_event(&self, worker_id: &str, summary: &str) {
        let store = self.event_store();
        let event = Event::new(
            EventType::WorkerFileEdited,
            EventEntityType::Agent,
            worker_id,
            summary.to_string(),
        )
        .with_session(worker_id.to_string());
        store.record(&event).expect("record worker activity");
    }

    fn register_worker_with_metadata(
        &self,
        name: &str,
        metadata: HashMap<String, String>,
    ) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Worker;
        agent.metadata = metadata;
        store.register(&agent).expect("register worker");
        id
    }

    /// Register a worker with its `last_heartbeat` backdated so
    /// `factory_worker_status` classifies it as DEAD (elapsed > 30s).
    ///
    /// Used by the cas-5b1c worker_status integration test to drive the
    /// `[DEAD]` label + transcript-path surfacing branch without waiting
    /// 30 seconds of real time.
    fn register_stale_worker_with_clone_path(
        &self,
        name: &str,
        clone_path: &str,
        stale_secs: i64,
    ) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Worker;
        agent
            .metadata
            .insert("clone_path".to_string(), clone_path.to_string());
        // Backdate BOTH last_heartbeat and registered_at so this fixture
        // survives any future change that adds `registered_at` to the
        // stale-criteria set (adversarial cas-5b1c review A5). Current
        // `list_stale(threshold_secs)` keys on last_heartbeat only, but the
        // fixture is a test-stability anchor — backdating both is cheap
        // insurance against silent regression of the prune criteria.
        let staleness = chrono::Duration::seconds(stale_secs);
        agent.last_heartbeat = chrono::Utc::now() - staleness;
        agent.registered_at = chrono::Utc::now() - staleness;
        store.register(&agent).expect("register stale worker");
        id
    }

    fn register_supervisor(&self, name: &str) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Supervisor;
        store.register(&agent).expect("register supervisor");
        id
    }

    fn register_worker_with_status(&self, name: &str, status: AgentStatus) -> String {
        let store = self.agent_store();
        let id = Agent::generate_fallback_id();
        let mut agent = Agent::new(id.clone(), name.to_string());
        agent.role = AgentRole::Worker;
        agent.status = status;
        store.register(&agent).expect("register worker with status");
        id
    }

    fn agent_store(&self) -> Arc<dyn AgentStore> {
        open_agent_store(&self.cas_root).expect("open agent store")
    }

    fn task_store(&self) -> Arc<dyn TaskStore> {
        open_task_store(&self.cas_root).expect("open task store")
    }

    fn event_store(&self) -> Arc<dyn EventStore> {
        open_event_store(&self.cas_root).expect("open event store")
    }

    fn spawn_queue(&self) -> Arc<dyn SpawnQueueStore> {
        open_spawn_queue_store(&self.cas_root).expect("open spawn queue")
    }

    fn prompt_queue(&self) -> Arc<dyn PromptQueueStore> {
        open_prompt_queue_store(&self.cas_root).expect("open prompt queue")
    }
}

/// Mutex to serialize tests that modify environment variables.
/// Env vars are process-global, so concurrent tests would interfere.
static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// RAII guard for environment variables. Acquires ENV_MUTEX.
struct EnvGuard {
    saved: Vec<(String, Option<String>)>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

impl EnvGuard {
    fn set(vars: &[(&str, &str)]) -> Self {
        let lock = ENV_MUTEX.lock().unwrap();
        let mut saved = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            let key = (*key).to_string();
            let prev = std::env::var(&key).ok();
            unsafe { std::env::set_var(&key, value) };
            saved.push((key, prev));
        }
        Self { saved, _lock: lock }
    }

    fn set_optional(vars: &[(&str, Option<&str>)]) -> Self {
        let lock = ENV_MUTEX.lock().unwrap();
        let mut saved = Vec::with_capacity(vars.len());
        for (key, value) in vars {
            let key = (*key).to_string();
            let prev = std::env::var(&key).ok();
            match value {
                Some(value) => unsafe { std::env::set_var(&key, value) },
                None => unsafe { std::env::remove_var(&key) },
            }
            saved.push((key, prev));
        }
        Self { saved, _lock: lock }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, prev) in self.saved.drain(..) {
            match prev {
                Some(val) => unsafe { std::env::set_var(&key, val) },
                None => unsafe { std::env::remove_var(&key) },
            }
        }
        // _lock drops here, releasing the mutex
    }
}

fn factory_req(action: &str) -> FactoryRequest {
    FactoryRequest {
        action: action.to_string(),
        id: None,
        count: None,
        worker_names: None,
        task_id: None,
        target: None,
        message: None,
        force: None,
        clear: None,
        branch: None,
        older_than_secs: None,
        isolate: None,
        remind_message: None,
        remind_delay_secs: None,
        remind_event: None,
        remind_filter: None,
        remind_id: None,
        remind_ttl_secs: None,
        cli: None,
        model: None,
        effort: None,
    }
}

fn coord_req(action: &str) -> CoordinationRequest {
    CoordinationRequest {
        action: action.to_string(),
        id: None,
        task_id: None,
        target: None,
        message: None,
        summary: None,
        urgent: None,
        force: None,
        clear: None,
        limit: None,
        name: None,
        agent_type: None,
        parent_id: None,
        session_id: None,
        prompt: None,
        max_iterations: None,
        completion_promise: None,
        reason: None,
        stale_threshold_secs: None,
        supervisor_id: None,
        event_type: None,
        payload: None,
        priority: None,
        notification_id: None,
        count: None,
        worker_names: None,
        branch: None,
        older_than_secs: None,
        isolate: None,
        cli: None,
        model: None,
        effort: None,
        remind_message: None,
        remind_delay_secs: None,
        remind_event: None,
        remind_filter: None,
        remind_id: None,
        remind_ttl_secs: None,
        all: None,
        status: None,
        orphans: None,
        dry_run: None,
    }
}

fn get_text(result: &rmcp::model::CallToolResult) -> String {
    result
        .content
        .iter()
        .filter_map(|c| match &c.raw {
            RawContent::Text(t) => Some(t.text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_session_metadata(session_name: &str, epic_id: Option<&str>) {
    let path = cas::ui::factory::metadata_path(session_name);
    std::fs::create_dir_all(path.parent().expect("metadata parent")).unwrap();
    let metadata = cas::ui::factory::create_metadata(
        session_name,
        12345,
        "supervisor",
        &[],
        epic_id,
        Some("/tmp/project"),
        None,
    );
    std::fs::write(path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();
}

fn read_session_metadata(session_name: &str) -> cas::ui::factory::SessionMetadata {
    let path = cas::ui::factory::metadata_path(session_name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn test_focus_epic_pins_valid_epic_and_records_activity() {
    let home = TempDir::new().expect("home tempdir");
    let home_path = home.path().to_str().unwrap();
    let _guard = EnvGuard::set(&[
        ("CAS_FACTORY_SESSION", "session-focus-pin"),
        ("HOME", home_path),
    ]);
    let env = FactoryTestEnv::new();
    let epic_id = env.create_epic("Focused Epic");
    write_session_metadata("session-focus-pin", Some("cas-session"));

    let mut req = factory_req("focus_epic");
    req.id = Some(epic_id.clone());
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("focus_epic should succeed");

    let text = get_text(&result);
    assert!(
        text.contains(&epic_id),
        "response should name pinned epic: {text}"
    );
    let metadata = read_session_metadata("session-focus-pin");
    assert_eq!(metadata.epic_id, Some("cas-session".to_string()));
    assert_eq!(metadata.pinned_epic_id, Some(epic_id.clone()));

    let events = env.event_store().list_recent(10).unwrap();
    assert!(
        events.iter().any(|event| {
            event.event_type == EventType::SupervisorInjected
                && event.entity_id == epic_id
                && event.session_id.as_deref() == Some("session-focus-pin")
        }),
        "focus_epic should record an activity event"
    );
}

#[tokio::test]
async fn test_focus_epic_rejects_missing_and_non_epic_without_mutation() {
    let home = TempDir::new().expect("home tempdir");
    let home_path = home.path().to_str().unwrap();
    let _guard = EnvGuard::set(&[
        ("CAS_FACTORY_SESSION", "session-focus-invalid"),
        ("HOME", home_path),
    ]);
    let env = FactoryTestEnv::new();
    write_session_metadata("session-focus-invalid", Some("cas-session"));

    let mut missing = factory_req("focus_epic");
    missing.id = None;
    assert!(
        env.service.factory(Parameters(missing)).await.is_err(),
        "missing id without clear=true should fail"
    );
    assert_eq!(
        read_session_metadata("session-focus-invalid").pinned_epic_id,
        None
    );

    let mut nonexistent = factory_req("focus_epic");
    nonexistent.id = Some("cas-does-not-exist".to_string());
    assert!(
        env.service.factory(Parameters(nonexistent)).await.is_err(),
        "nonexistent id should fail"
    );
    assert_eq!(
        read_session_metadata("session-focus-invalid").pinned_epic_id,
        None
    );

    let store = env.task_store();
    let task_id = store.generate_id().expect("generate_id");
    let task = Task::new(task_id.clone(), "Regular Task".to_string());
    store.add(&task).expect("add task");

    let mut non_epic = factory_req("focus_epic");
    non_epic.id = Some(task_id);
    assert!(
        env.service.factory(Parameters(non_epic)).await.is_err(),
        "non-epic id should fail"
    );
    assert_eq!(
        read_session_metadata("session-focus-invalid").pinned_epic_id,
        None
    );
}

#[tokio::test]
async fn test_focus_epic_rejects_closed_epic_without_mutation() {
    let home = TempDir::new().expect("home tempdir");
    let home_path = home.path().to_str().unwrap();
    let _guard = EnvGuard::set(&[
        ("CAS_FACTORY_SESSION", "session-focus-closed"),
        ("HOME", home_path),
    ]);
    let env = FactoryTestEnv::new();
    let epic_id = env.create_epic("Closed Epic");
    let store = env.task_store();
    let mut epic = store.get(&epic_id).expect("get epic");
    epic.status = TaskStatus::Closed;
    store.update(&epic).expect("close epic");
    write_session_metadata("session-focus-closed", Some("cas-session"));

    let mut req = factory_req("focus_epic");
    req.id = Some(epic_id);
    assert!(
        env.service.factory(Parameters(req)).await.is_err(),
        "closed epic id should fail"
    );

    let metadata = read_session_metadata("session-focus-closed");
    assert_eq!(metadata.epic_id, Some("cas-session".to_string()));
    assert_eq!(metadata.pinned_epic_id, None);
}

#[tokio::test]
async fn test_focus_epic_clear_removes_pin_and_preserves_session_default() {
    let home = TempDir::new().expect("home tempdir");
    let home_path = home.path().to_str().unwrap();
    let _guard = EnvGuard::set(&[
        ("CAS_FACTORY_SESSION", "session-focus-clear"),
        ("HOME", home_path),
    ]);
    let env = FactoryTestEnv::new();
    let epic_id = env.create_epic("Focused Epic");
    write_session_metadata("session-focus-clear", Some("cas-session"));

    let mut pin = factory_req("focus_epic");
    pin.id = Some(epic_id);
    env.service
        .factory(Parameters(pin))
        .await
        .expect("pin should succeed");

    let mut clear = factory_req("focus_epic");
    clear.clear = Some(true);
    env.service
        .factory(Parameters(clear))
        .await
        .expect("clear should succeed");

    let metadata = read_session_metadata("session-focus-clear");
    assert_eq!(metadata.epic_id, Some("cas-session".to_string()));
    assert_eq!(metadata.pinned_epic_id, None);

    let events = env.event_store().list_recent(10).unwrap();
    assert!(
        events.iter().any(|event| {
            event.event_type == EventType::SupervisorInjected
                && event.entity_id == "session-focus-clear"
                && event.session_id.as_deref() == Some("session-focus-clear")
        }),
        "clear=true should record a supervisor activity event"
    );
}

#[tokio::test]
async fn test_coordination_focus_epic_routes_clear_field() {
    let home = TempDir::new().expect("home tempdir");
    let home_path = home.path().to_str().unwrap();
    let _guard = EnvGuard::set(&[
        ("CAS_FACTORY_SESSION", "session-focus-coordination"),
        ("HOME", home_path),
    ]);
    let env = FactoryTestEnv::new();
    let epic_id = env.create_epic("Coordination Epic");
    write_session_metadata("session-focus-coordination", Some("cas-session"));

    let mut pin = coord_req("focus_epic");
    pin.id = Some(epic_id.clone());
    env.service
        .coordination(Parameters(pin))
        .await
        .expect("coordination focus_epic should pin");
    assert_eq!(
        read_session_metadata("session-focus-coordination").pinned_epic_id,
        Some(epic_id)
    );

    let mut clear = coord_req("focus_epic");
    clear.clear = Some(true);
    env.service
        .coordination(Parameters(clear))
        .await
        .expect("coordination focus_epic should forward clear=true");
    let metadata = read_session_metadata("session-focus-coordination");
    assert_eq!(metadata.epic_id, Some("cas-session".to_string()));
    assert_eq!(metadata.pinned_epic_id, None);
}

// =============================================================================
// spawn_workers tests
// =============================================================================

#[tokio::test]
async fn test_spawn_workers_requires_epic() {
    let env = FactoryTestEnv::new();

    let req = factory_req("spawn_workers");
    let result = env.service.factory(Parameters(req)).await;

    assert!(result.is_err(), "Should fail without epic");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("No active EPIC"),
        "Error should mention missing EPIC: {}",
        err.message
    );
}

#[tokio::test]
async fn test_spawn_workers_enqueues_with_epic() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(3);
    req.worker_names = Some("alpha,beta,gamma".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok(), "Should succeed with epic");

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("alpha, beta, gamma"),
        "Should list worker names: {text}"
    );

    // Verify queue
    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1, "Should have 1 spawn queue entry");
    assert_eq!(entries[0].action, cas_store::SpawnAction::Spawn);
    assert_eq!(entries[0].worker_names, vec!["alpha", "beta", "gamma"]);
}

#[tokio::test]
async fn test_spawn_workers_isolate_flag() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(2);
    req.isolate = Some(true);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    assert!(entries[0].isolate, "Should have isolate=true");
}

/// cas-6913 AC3: `task_id` on a single-worker spawn request must carry
/// through to the queued `SpawnRequest`, ready for `finish_worker_spawn` to
/// pick up once the daemon actually spawns the worker (unit-tested
/// separately in epic_workers.rs — this test covers the MCP-to-queue leg).
#[tokio::test]
async fn test_spawn_workers_task_id_enqueues_for_single_worker() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");
    let task_store = env.task_store();
    let task_id = task_store.generate_id().expect("generate_id");
    task_store
        .add(&Task::new(task_id.clone(), "Pre-assign me".to_string()))
        .expect("add task");

    let mut req = factory_req("spawn_workers");
    req.count = Some(1);
    req.task_id = Some(task_id.clone());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok(), "single-worker spawn with task_id should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains(&task_id),
        "response should mention the pre-assigned task: {text}"
    );

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].task_id.as_deref(), Some(task_id.as_str()));
}

/// cas-6913 AC3: task_id with a single explicit worker_names entry is also
/// a valid "single worker" request (not just count=1).
#[tokio::test]
async fn test_spawn_workers_task_id_enqueues_for_single_named_worker() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");
    let task_store = env.task_store();
    let task_id = task_store.generate_id().expect("generate_id");
    task_store
        .add(&Task::new(task_id.clone(), "Pre-assign me".to_string()))
        .expect("add task");

    let mut req = factory_req("spawn_workers");
    req.worker_names = Some("swift-fox".to_string());
    req.task_id = Some(task_id.clone());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok(), "single named-worker spawn with task_id should succeed: {result:?}");

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].task_id.as_deref(), Some(task_id.as_str()));
}

/// cas-6913: task_id must be rejected (not silently ignored or applied to
/// only one of several) when the spawn request is ambiguous about which
/// worker "the" spawned worker is.
#[tokio::test]
async fn test_spawn_workers_task_id_rejects_multi_worker_count() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");
    let task_store = env.task_store();
    let task_id = task_store.generate_id().expect("generate_id");
    task_store
        .add(&Task::new(task_id.clone(), "Ambiguous".to_string()))
        .expect("add task");

    let mut req = factory_req("spawn_workers");
    req.count = Some(3);
    req.task_id = Some(task_id);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "task_id with count>1 must be rejected");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("single-worker"),
        "error should explain the single-worker requirement: {}",
        err.message
    );

    assert!(
        env.spawn_queue().peek(10).expect("peek").is_empty(),
        "rejected request must not enqueue anything"
    );
}

/// cas-6913: same ambiguity guard, via worker_names listing more than one name.
#[tokio::test]
async fn test_spawn_workers_task_id_rejects_multi_worker_names() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");
    let task_store = env.task_store();
    let task_id = task_store.generate_id().expect("generate_id");
    task_store
        .add(&Task::new(task_id.clone(), "Ambiguous".to_string()))
        .expect("add task");

    let mut req = factory_req("spawn_workers");
    req.worker_names = Some("alpha,beta".to_string());
    req.task_id = Some(task_id);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "task_id with 2 worker_names must be rejected");
}

/// cas-6913: task_id referencing a task that doesn't exist must fail fast
/// with a clear error, not silently queue a spawn request that can never
/// resolve the assignment.
#[tokio::test]
async fn test_spawn_workers_task_id_rejects_unknown_task() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(1);
    req.task_id = Some("cas-doesnotexist".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "unknown task_id must be rejected");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("not found"),
        "error should say the task wasn't found: {}",
        err.message
    );
}

/// cas-6913: task_id referencing an already-closed task must be rejected —
/// pre-assigning a spawned worker to dead work is never useful.
#[tokio::test]
async fn test_spawn_workers_task_id_rejects_closed_task() {
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");
    let task_store = env.task_store();
    let task_id = task_store.generate_id().expect("generate_id");
    let mut task = Task::new(task_id.clone(), "Already done".to_string());
    task.status = TaskStatus::Closed;
    task_store.add(&task).expect("add closed task");

    let mut req = factory_req("spawn_workers");
    req.count = Some(1);
    req.task_id = Some(task_id);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "closed task_id must be rejected");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("closed"),
        "error should say the task is closed: {}",
        err.message
    );
}

#[tokio::test]
async fn test_spawn_workers_closed_epic_not_counted() {
    let env = FactoryTestEnv::new();

    // Create an epic and close it
    let epic_id = env.create_epic("Closed Epic");
    let store = env.task_store();
    let mut task = store.get(&epic_id).expect("get epic");
    task.status = TaskStatus::Closed;
    store.update(&task).expect("close epic");

    let req = factory_req("spawn_workers");
    let result = env.service.factory(Parameters(req)).await;

    assert!(result.is_err(), "Closed epic should not count as active");
}

// cas-2992: spawn_workers with cli/model/effort overrides
#[tokio::test]
async fn test_spawn_workers_cli_codex_enqueues_spec() {
    // Given a spawn_workers request with cli=codex,
    // the queued SpawnRequest.worker_spec should contain "codex".
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(1);
    req.cli = Some("codex".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(
        result.is_ok(),
        "spawn_workers with cli=codex should succeed"
    );

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1, "should have one queue entry");

    let spec_json = entries[0]
        .worker_spec
        .as_deref()
        .expect("worker_spec should be set when cli override given");
    assert!(
        spec_json.contains("codex"),
        "spec JSON should mention 'codex': {spec_json}"
    );
}

#[tokio::test]
async fn test_spawn_workers_invalid_cli_returns_error() {
    // An unrecognised cli value should return an MCP error, not silently use defaults.
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(1);
    req.cli = Some("openai".to_string()); // invalid

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "invalid cli should return error");
    let err = result.unwrap_err();
    assert!(
        err.message.contains("openai") || err.message.contains("cli"),
        "error should mention the invalid value: {}",
        err.message
    );
}

#[tokio::test]
async fn test_spawn_workers_no_cli_override_queues_safe_worker_spec() {
    // Without cli/model/effort fields, worker_spec resolves to the safe worker
    // floor instead of inheriting the supervisor session defaults.
    let env = FactoryTestEnv::new();
    env.create_epic("Test Epic");

    let mut req = factory_req("spawn_workers");
    req.count = Some(2);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    let spec_json = entries[0]
        .worker_spec
        .as_deref()
        .expect("no cli/model/effort should still queue a resolved worker_spec");
    let spec: cas_mux::WorkerSpec = serde_json::from_str(spec_json).expect("valid WorkerSpec");
    assert_eq!(spec.cli, cas_mux::SupervisorCli::Codex);
    assert_eq!(spec.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(spec.effort, Some(cas_mux::Effort::Medium));
}

// =============================================================================
// shutdown_workers tests
// =============================================================================

#[tokio::test]
async fn test_shutdown_workers_validates_existence() {
    let env = FactoryTestEnv::new();
    env.register_worker("alice");

    let mut req = factory_req("shutdown_workers");
    req.worker_names = Some("alice,charlie".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err(), "Should fail for nonexistent worker");

    let err = result.unwrap_err();
    assert!(
        err.message.contains("charlie"),
        "Error should mention missing worker: {}",
        err.message
    );
}

#[tokio::test]
async fn test_shutdown_workers_enqueues() {
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();
    env.register_worker("alice");
    env.register_worker("bob");

    let mut req = factory_req("shutdown_workers");
    req.worker_names = Some("alice,bob".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(text.contains("alice, bob"), "Should list workers: {text}");

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].action, cas_store::SpawnAction::Shutdown);
    assert!(entries[0].worker_names.contains(&"alice".to_string()));
    assert!(entries[0].worker_names.contains(&"bob".to_string()));
}

#[tokio::test]
async fn test_shutdown_workers_all() {
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();
    env.register_worker("alice");

    let mut req = factory_req("shutdown_workers");
    req.count = Some(0);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(text.contains("ALL workers"), "Should say ALL: {text}");
}

#[tokio::test]
async fn test_shutdown_workers_supervisor_scoping() {
    let env = FactoryTestEnv::new();
    env.register_worker("owned-1");
    env.register_worker("other-1");

    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_FACTORY_WORKER_NAMES", "owned-1"),
    ]);

    // Empty worker_names should auto-scope to owned workers
    let req = factory_req("shutdown_workers");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].worker_names, vec!["owned-1"]);
}

// =============================================================================
// worker_status tests
// =============================================================================

#[tokio::test]
async fn test_worker_status_empty() {
    let env = FactoryTestEnv::new();

    let req = factory_req("worker_status");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("No active agents"),
        "Should report no agents: {text}"
    );
}

#[tokio::test]
async fn test_worker_status_shows_agents() {
    // Acquire env mutex to prevent concurrent tests from setting CAS_AGENT_ROLE=supervisor
    // which would activate supervisor scoping and filter out our test workers.
    let _guard = EnvGuard::set(&[]);

    let env = FactoryTestEnv::new();
    env.register_supervisor("sup-1");

    let mut meta = HashMap::new();
    meta.insert("clone_path".to_string(), "/tmp/worktree/wolf".to_string());
    meta.insert("worker_model".to_string(), "sonnet".to_string());
    meta.insert("worker_effort".to_string(), "high".to_string());
    env.register_worker_with_metadata("wolf", meta);
    env.register_worker("fox");

    let req = factory_req("worker_status");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Workers (2)"),
        "Should show 2 workers: {text}"
    );
    assert!(text.contains("wolf"), "Should list wolf: {text}");
    assert!(text.contains("fox"), "Should list fox: {text}");
    assert!(
        text.contains("/tmp/worktree/wolf"),
        "Should show clone path: {text}"
    );
    assert!(text.contains("model: sonnet"), "Should show model: {text}");
    assert!(text.contains("effort: high"), "Should show effort: {text}");
}

#[tokio::test]
async fn test_worker_status_scopes_agents_to_factory_session() {
    let _guard = EnvGuard::set_optional(&[("CAS_FACTORY_SESSION", None)]);
    let env = FactoryTestEnv::new();

    env.register_supervisor_in_session("sup-a", "session-a");
    env.register_worker_in_session("worker-a", "session-a");
    env.register_worker_in_session("worker-b", "session-b");

    let mut plain = Agent::new("plain-agent".to_string(), "plain-worker".to_string());
    plain.role = AgentRole::Worker;
    plain.factory_session = None;
    let agent_store = env.agent_store();
    agent_store.register(&plain).expect("register plain worker");

    unsafe { std::env::set_var("CAS_FACTORY_SESSION", "session-a") };
    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status should succeed");

    let text = get_text(&result);
    assert!(
        text.contains("worker-a"),
        "same-session worker visible: {text}"
    );
    assert!(
        !text.contains("worker-b"),
        "other-session worker must be hidden: {text}"
    );
    assert!(
        !text.contains("plain-worker"),
        "NULL-session plain CC worker must be hidden from factory director: {text}"
    );
}

/// cas-3e56: heartbeat past WORKER_STALE_SECS but registered harness PID still
/// alive → worker_status must keep the worker listed as active with the
/// "[alive — heartbeat stale]" dual-signal, never omit as "None active".
///
/// This is the supervision-truth residual after Grok liveness work: false
/// "None active" while a Grok worker is mid-turn nearly caused a re-spawn.
#[tokio::test]
async fn test_worker_status_keeps_heartbeat_stale_process_alive_worker() {
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();

    let store = env.agent_store();
    let id = Agent::generate_fallback_id();
    let mut agent = Agent::new(id.clone(), "mid-turn-grok".to_string());
    agent.role = AgentRole::Worker;
    agent
        .metadata
        .insert("worker_cli".to_string(), "grok".to_string());
    // Heartbeat is "stale" by the 30s prune threshold.
    let staleness = chrono::Duration::seconds(40);
    agent.last_heartbeat = chrono::Utc::now() - staleness;
    agent.registered_at = chrono::Utc::now() - staleness;
    // Registered PID = this test process (alive). No fingerprint → pid-only
    // liveness (kill 0) still proves the process is up.
    agent.pid = Some(std::process::id());
    store
        .register(&agent)
        .expect("register heartbeat-stale process-alive worker");

    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("mid-turn-grok"),
        "process-alive worker must stay in Active listing despite stale heartbeat. Got:\n{text}"
    );
    assert!(
        !text.contains("Workers: None active"),
        "must not report empty roster when a process-alive worker exists. Got:\n{text}"
    );
    assert!(
        text.contains("alive") && text.contains("heartbeat stale"),
        "must surface dual-signal '[alive — heartbeat stale]'. Got:\n{text}"
    );
    assert!(
        !text.contains("Filtered stale agent record(s)"),
        "process-alive worker must not be pruned. Got:\n{text}"
    );
}

/// cas-5b1c integration coverage: a worker whose heartbeat is older than
/// `WORKER_STALE_SECS` (30s) is pruned out of the Active listing on the
/// next `factory_worker_status` call and reported in the "Filtered stale
/// agent record(s)" footer, while a live worker from the same call stays
/// visible. This pins the supervisor-facing UX contract that stale
/// workers disappear promptly once past the threshold.
///
/// Implementation note: `factory_worker_status` does its opportunistic
/// prune BEFORE rendering the Active list, so in the common path a
/// stale Worker transitions out of Active and never hits the `[DEAD]`
/// label / transcript-path render branch. The render-time DEAD branch
/// only fires when `mark_stale` fails (DB lock, etc.) — that code path
/// is cheap unit coverage at the `resolve_transcript` / `render_transcript_block`
/// level (see the `mcp::tools::service::factory_ops::tests` module),
/// now with glob-based resolution landed via cas-900b. Here we test the
/// prune-success integration.
#[tokio::test]
async fn test_worker_status_prunes_stale_worker_and_keeps_live_one() {
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();

    // Live worker: default heartbeat = now, stays in Active.
    env.register_worker("live-fox");
    // Stale worker: heartbeat backdated 40s so list_stale(30) catches it.
    let stale_id =
        env.register_stale_worker_with_clone_path("dead-wolf", "/tmp/cas-worktrees/dead-wolf", 40);

    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status call should succeed");
    let text = get_text(&result);

    // Live worker must appear; stale must not.
    assert!(
        text.contains("live-fox"),
        "live worker must appear in Active listing. Got:\n{text}"
    );
    assert!(
        !text.contains("dead-wolf"),
        "stale worker must be pruned out of the Active listing. Got:\n{text}"
    );
    assert!(
        !text.contains(&stale_id),
        "stale worker's id must not appear in render. Got:\n{text}"
    );

    // The footer must account for the prune so operators can see the
    // pruned count at a glance.
    assert!(
        text.contains("Filtered stale agent record(s): 1"),
        "prune summary must report exactly 1 stale record filtered. Got:\n{text}"
    );
    assert!(
        text.contains("30s heartbeat age"),
        "footer must reference the 30s worker threshold. Got:\n{text}"
    );
}

#[tokio::test]
async fn test_worker_status_prune_skips_stale_workers_in_other_factory_sessions() {
    let _guard = EnvGuard::set(&[("CAS_FACTORY_SESSION", "session-a")]);
    let env = FactoryTestEnv::new();

    env.register_worker_in_session("live-a", "session-a");

    let store = env.agent_store();
    let stale_b_id = Agent::generate_fallback_id();
    let mut stale_b = Agent::new(stale_b_id.clone(), "stale-b".to_string());
    stale_b.role = AgentRole::Worker;
    stale_b.status = AgentStatus::Active;
    stale_b.factory_session = Some("session-b".to_string());
    let staleness = chrono::Duration::seconds(40);
    stale_b.last_heartbeat = chrono::Utc::now() - staleness;
    stale_b.registered_at = chrono::Utc::now() - staleness;
    store
        .register(&stale_b)
        .expect("register stale session-b worker");

    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("live-a"),
        "same-session live worker should appear: {text}"
    );
    assert!(
        !text.contains("stale-b"),
        "other-session stale worker should remain hidden: {text}"
    );
    let stale_after = store
        .get(&stale_b_id)
        .expect("session-b stale worker should still exist");
    assert_eq!(
        stale_after.status,
        AgentStatus::Active,
        "session-a worker_status prune must not mark stale workers in session-b"
    );
}

/// cas-9829: a worker holding an in-progress task lease whose last observed
/// activity is at/past the configured `stall_threshold_secs` must render
/// `⚠ STALLED`, not the soft "may be investigating or idle" hedge — that
/// hedge is exactly what let a genuinely stalled worker go unnoticed in the
/// reported bug (worker printed a plan, then produced nothing for 10+
/// minutes while heartbeating fine). A worker with NO claimed task must
/// never be marked STALLED — that's the pre-existing WorkerIdle state, a
/// different signal entirely.
///
/// `stall_threshold_secs` is set to `0` via `.cas/config.toml` so the
/// claim's own registration-time activity event (which is necessarily
/// "0s ago" in a synchronous test) already counts as past-threshold —
/// this deterministically exercises the render wiring without needing to
/// fabricate a real time gap.
#[tokio::test]
async fn test_9829_worker_status_marks_stalled_worker_with_in_progress_task() {
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();
    std::fs::write(
        env.cas_root.join("config.toml"),
        "[factory]\nstall_threshold_secs = 0\n",
    )
    .expect("write config.toml");

    let busy_id = env.register_worker("busy-badger");
    let task_store = env.task_store();
    let task = Task::new("cas-0b7d".to_string(), "Stalled task".to_string());
    task_store.add(&task).expect("add task");
    env.agent_store()
        .try_claim("cas-0b7d", &busy_id, 600, None)
        .expect("claim task")
        .is_success();

    env.register_worker("idle-ibis"); // no claimed task — must stay soft-worded

    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status call should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("busy-badger"),
        "busy worker must appear in the listing. Got:\n{text}"
    );
    // Find the busy-badger's own row/block for a precise assertion (avoid a
    // STALLED marker from the wrong worker satisfying the check).
    let badger_block = text
        .split("• ")
        .find(|block| block.starts_with("busy-badger"))
        .expect("busy-badger row must be present");
    assert!(
        badger_block.contains("⚠ STALLED"),
        "worker with an in-progress task past the stall threshold must be marked STALLED. Got:\n{badger_block}"
    );

    // A worker with no claimed task is never "stalled" in this sense — an
    // idle worker with no task is the pre-existing WorkerIdle state, not a
    // stall, regardless of how fresh/stale its activity looks.
    let ibis_block = text
        .split("• ")
        .find(|block| block.starts_with("idle-ibis"))
        .expect("idle-ibis row must be present");
    assert!(
        !ibis_block.contains("⚠ STALLED"),
        "a worker with no claimed task must never be marked STALLED. Got:\n{ibis_block}"
    );
}

// =============================================================================
// worker_activity tests
// =============================================================================

#[tokio::test]
async fn test_worker_activity_empty() {
    let env = FactoryTestEnv::new();

    let req = factory_req("worker_activity");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("No recent worker activity"),
        "Should report no activity: {text}"
    );
}

#[tokio::test]
async fn test_worker_activity_scopes_session_and_honors_target_filter() {
    let _guard = EnvGuard::set(&[("CAS_FACTORY_SESSION", "session-a")]);
    let env = FactoryTestEnv::new();

    let worker_a = env.register_worker_in_session("worker-a", "session-a");
    let worker_b = env.register_worker_in_session("worker-b", "session-b");
    env.record_worker_file_event(&worker_a, "worker-a edited src/lib.rs");
    env.record_worker_file_event(&worker_b, "worker-b edited src/lib.rs");

    let mut req = factory_req("worker_activity");
    req.target = Some("worker-a".to_string());
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_activity should succeed");

    let text = get_text(&result);
    assert!(
        text.contains("worker-a edited"),
        "targeted same-session activity should be visible: {text}"
    );
    assert!(
        !text.contains("worker-b edited"),
        "other-session activity must be hidden: {text}"
    );

    let mut req = factory_req("worker_activity");
    req.target = Some("worker-b".to_string());
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_activity should succeed for hidden target");
    let text = get_text(&result);
    assert!(
        text.contains("No recent worker activity"),
        "target outside caller session should produce no activity: {text}"
    );
}

#[tokio::test]
async fn test_worker_activity_includes_idle_workers() {
    let _guard = EnvGuard::set(&[("CAS_FACTORY_SESSION", "session-a")]);
    let env = FactoryTestEnv::new();

    let store = env.agent_store();
    let idle_id = Agent::generate_fallback_id();
    let mut idle_worker = Agent::new(idle_id.clone(), "idle-worker".to_string());
    idle_worker.role = AgentRole::Worker;
    idle_worker.status = AgentStatus::Idle;
    idle_worker.factory_session = Some("session-a".to_string());
    store
        .register(&idle_worker)
        .expect("register idle worker in session");
    env.record_worker_file_event(&idle_id, "idle-worker edited src/lib.rs");

    let mut req = factory_req("worker_activity");
    req.target = Some("idle-worker".to_string());
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_activity should include idle worker");

    let text = get_text(&result);
    assert!(
        text.contains("idle-worker edited"),
        "Idle workers with recent events should still report activity: {text}"
    );
}

// =============================================================================
// clear_context tests
// =============================================================================

#[tokio::test]
async fn test_clear_context_enqueues() {
    let env = FactoryTestEnv::with_agent_id("test-sup");

    let store = env.agent_store();
    let agent = Agent::new("test-sup".to_string(), "supervisor".to_string());
    store.register(&agent).expect("register");

    let mut req = factory_req("clear_context");
    req.target = Some("wolf".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].target, "wolf");
    assert_eq!(prompts[0].prompt, "/clear");
}

#[tokio::test]
async fn test_clear_context_all_workers() {
    let env = FactoryTestEnv::with_agent_id("test-sup");

    let store = env.agent_store();
    let agent = Agent::new("test-sup".to_string(), "supervisor".to_string());
    store.register(&agent).expect("register");

    let mut req = factory_req("clear_context");
    req.target = Some("all_workers".to_string());

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("all workers"),
        "Should mention all workers: {text}"
    );

    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].target, "all_workers");
    assert_eq!(prompts[0].prompt, "/clear");
}

// =============================================================================
// my_context tests
// =============================================================================

#[tokio::test]
async fn test_my_context_shows_agent_info() {
    let env = FactoryTestEnv::with_agent_id("ctx-agent-id");

    let store = env.agent_store();
    let mut agent = Agent::new("ctx-agent-id".to_string(), "ctx-supervisor".to_string());
    agent.role = AgentRole::Supervisor;
    store.register(&agent).expect("register");

    let req = factory_req("my_context");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(text.contains("ctx-supervisor"), "Should show name: {text}");
    assert!(text.contains("Supervisor"), "Should show role: {text}");
    assert!(text.contains("ctx-agent-id"), "Should show ID: {text}");
    assert!(text.contains("None (idle)"), "Should show no tasks: {text}");
}

// =============================================================================
// gc_report tests
// =============================================================================

#[tokio::test]
async fn test_gc_report_empty() {
    let env = FactoryTestEnv::new();

    let req = factory_req("gc_report");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Stale agents: 0"),
        "Should show 0 stale: {text}"
    );
    assert!(
        text.contains("Pending prompts: 0"),
        "Should show 0 prompts: {text}"
    );
}

#[tokio::test]
async fn test_gc_report_shows_pending_prompts() {
    let env = FactoryTestEnv::new();

    // Add some pending prompts
    let pq = env.prompt_queue();
    pq.enqueue("src", "wolf", "do stuff").expect("enqueue");
    pq.enqueue("src", "fox", "do other stuff").expect("enqueue");

    let req = factory_req("gc_report");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Pending prompts: 2"),
        "Should show 2 prompts: {text}"
    );
}

// =============================================================================
// gc_cleanup tests
// =============================================================================

#[tokio::test]
async fn test_gc_cleanup_without_force() {
    let env = FactoryTestEnv::new();

    // Add pending prompts
    let pq = env.prompt_queue();
    pq.enqueue("src", "wolf", "test").expect("enqueue");

    let req = factory_req("gc_cleanup");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Prompt queue entries cleared: 0"),
        "Should NOT clear prompts without force: {text}"
    );

    // Prompts should still be pending
    assert_eq!(pq.pending_count().expect("count"), 1);
}

#[tokio::test]
async fn test_gc_cleanup_with_force() {
    let env = FactoryTestEnv::new();

    let pq = env.prompt_queue();
    pq.enqueue("src", "wolf", "test1").expect("enqueue");
    pq.enqueue("src", "fox", "test2").expect("enqueue");

    let mut req = factory_req("gc_cleanup");
    req.force = Some(true);

    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Prompt queue entries cleared: 2"),
        "Should clear prompts with force: {text}"
    );

    assert_eq!(pq.pending_count().expect("count"), 0);
}

#[tokio::test]
async fn test_gc_cleanup_purges_stale_and_shutdown_worker_records() {
    let env = FactoryTestEnv::new();

    let stale_id = env.register_worker_with_status("stale-wolf", AgentStatus::Stale);
    let shutdown_id = env.register_worker_with_status("shutdown-fox", AgentStatus::Shutdown);

    let req = factory_req("gc_cleanup");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Dead agent records purged: 2"),
        "Should report purged dead agent records: {text}"
    );

    let store = env.agent_store();
    assert!(
        store.get(&stale_id).is_err(),
        "stale worker should be purged"
    );
    assert!(
        store.get(&shutdown_id).is_err(),
        "shutdown worker should be purged"
    );
}

#[tokio::test]
async fn test_gc_cleanup_preserves_stale_supervisors() {
    let env = FactoryTestEnv::new();

    let supervisor_id = env.register_supervisor("stale-supervisor");
    let store = env.agent_store();
    let mut supervisor = store.get(&supervisor_id).expect("get supervisor");
    supervisor.status = AgentStatus::Stale;
    store.update(&supervisor).expect("mark supervisor stale");

    let req = factory_req("gc_cleanup");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    let text = get_text(&result.unwrap());
    assert!(
        text.contains("Dead agent records purged: 0"),
        "Should not purge supervisor/director records: {text}"
    );
    assert!(
        store.get(&supervisor_id).is_ok(),
        "stale supervisor record should be preserved"
    );
}

// =============================================================================
// Sequence tests
// =============================================================================

#[tokio::test]
async fn test_spawn_then_shutdown_sequence() {
    let _guard = EnvGuard::set_optional(&[("CAS_FACTORY_SESSION", None)]);
    let env = FactoryTestEnv::new();
    env.create_epic("Sequence Epic");
    env.register_worker("alpha");

    // Spawn
    let mut req = factory_req("spawn_workers");
    req.count = Some(2);
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    // Shutdown
    let mut req = factory_req("shutdown_workers");
    req.worker_names = Some("alpha".to_string());
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_ok());

    // Both should be in queue
    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 2, "Should have 2 queue entries");
    assert_eq!(entries[0].action, cas_store::SpawnAction::Spawn);
    assert_eq!(entries[1].action, cas_store::SpawnAction::Shutdown);
}

#[tokio::test]
async fn test_unknown_action() {
    let env = FactoryTestEnv::new();

    let req = factory_req("invalid_action");
    let result = env.service.factory(Parameters(req)).await;
    assert!(result.is_err());

    let err = result.unwrap_err();
    assert!(
        err.message.contains("Unknown factory action"),
        "Should report unknown action: {}",
        err.message
    );
}

// =============================================================================
// cas-c931: urgent (interrupt-and-redirect) message routing
// =============================================================================

/// Minimal CoordinationRequest for the `message`/`interrupt` actions.
fn coord_msg(
    action: &str,
    target: &str,
    message: &str,
    urgent: Option<bool>,
) -> CoordinationRequest {
    CoordinationRequest {
        action: action.to_string(),
        id: None,
        task_id: None,
        target: Some(target.to_string()),
        message: Some(message.to_string()),
        summary: Some("test".to_string()),
        urgent,
        force: None,
        clear: None,
        limit: None,
        name: None,
        agent_type: None,
        parent_id: None,
        session_id: None,
        prompt: None,
        max_iterations: None,
        completion_promise: None,
        reason: None,
        stale_threshold_secs: None,
        supervisor_id: None,
        event_type: None,
        payload: None,
        priority: None,
        notification_id: None,
        count: None,
        worker_names: None,
        branch: None,
        older_than_secs: None,
        isolate: None,
        cli: None,
        model: None,
        effort: None,
        remind_message: None,
        remind_delay_secs: None,
        remind_event: None,
        remind_filter: None,
        remind_id: None,
        remind_ttl_secs: None,
        all: None,
        status: None,
        orphans: None,
        dry_run: None,
    }
}

/// Default (non-urgent) coordination message must enqueue with urgent=false —
/// the unchanged inbox/queue delivery path. Regression guard.
#[tokio::test]
async fn test_coordination_message_default_is_not_urgent() {
    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_AGENT_NAME", "supervisor"),
    ]);
    let env = FactoryTestEnv::new();
    env.register_worker("swift-fox");

    let req = coord_msg("message", "swift-fox", "FYI: status update", None);
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "message should succeed: {result:?}");

    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].target, "swift-fox");
    assert!(!prompts[0].urgent, "default message must not be urgent");
}

/// `urgent=true` on the message action enqueues urgent + Critical priority and
/// the response advertises interrupt-and-redirect delivery.
#[tokio::test]
async fn test_coordination_message_urgent_flag_enqueues_urgent() {
    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_AGENT_NAME", "supervisor"),
    ]);
    let env = FactoryTestEnv::new();
    env.register_worker("swift-fox");

    let req = coord_msg("message", "swift-fox", "STOP — wrong file", Some(true));
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "urgent message should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains("URGENT"),
        "response should mark URGENT: {text}"
    );

    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].urgent,
        "urgent=true must persist on the queue row"
    );
    assert_eq!(
        prompts[0].priority,
        cas_store::NotificationPriority::Critical,
        "urgent with no explicit priority defaults to Critical so it jumps the queue"
    );
}

/// cas-6913 AC2: a message to a target that IS registered must say so
/// honestly — not just "queued", but "queued for next poll (target is
/// registered)". Regression guard against re-collapsing the two cases.
#[tokio::test]
async fn test_coordination_message_to_registered_target_reports_delivery_status() {
    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_AGENT_NAME", "supervisor"),
    ]);
    let env = FactoryTestEnv::new();
    env.register_worker("swift-fox");

    let req = coord_msg("message", "swift-fox", "status update", None);
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "message should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains("target is registered"),
        "response should confirm the target is registered: {text}"
    );
    assert!(
        !text.contains("not yet registered"),
        "a registered target must not read as unregistered: {text}"
    );
}

/// cas-6913 AC2: the defect this task exists to fix — "Message queued" reads
/// as delivery confirmation even when the target name isn't in the agent
/// store yet (the common spawn-then-immediately-assign race). The ack must
/// say so honestly instead of implying success either way.
#[tokio::test]
async fn test_coordination_message_to_unregistered_target_reports_queued_pending_registration() {
    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_AGENT_NAME", "supervisor"),
    ]);
    let env = FactoryTestEnv::new();
    // Deliberately do NOT register "not-born-yet" — simulates a message
    // addressed to a worker name the supervisor already knows (e.g. from an
    // explicit spawn_workers worker_names= request) before the daemon has
    // finished spawning it.

    let req = coord_msg("message", "not-born-yet", "start with task cas-abc1", None);
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "message should still enqueue: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains("not yet registered"),
        "response must honestly flag the target as not yet registered: {text}"
    );
    assert!(
        !text.contains("target is registered"),
        "an unregistered target must not read as registered: {text}"
    );

    // The message still lands in the queue — this is about honest
    // reporting, not blocking the send. cas-7e20/daemon polling handles
    // eventual delivery once the name is registered.
    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts[0].target, "not-born-yet");
}

/// cas-6913 AC1: "queue-before-register -> register -> poll sees it".
/// A message queued to a worker name before that worker exists in the
/// agent store must be delivered into the worker's OWN prompt loop at
/// registration time (surfaced directly in the register response text —
/// no PTY-injection timing dependency), and must remain pollable
/// afterward (at-least-once, matching this queue's existing philosophy).
#[tokio::test]
async fn test_agent_register_surfaces_pending_prompt_queue_mail() {
    let env = FactoryTestEnv::new();

    // Step 1: queue-before-register.
    env.prompt_queue()
        .enqueue_urgent(
            "supervisor",
            "not-born-yet",
            "start with task cas-abc1",
            None,
            Some("assignment"),
            None,
            false,
        )
        .expect("enqueue pre-registration message");

    // Step 2: register.
    let mut req = coord_req("register");
    req.name = Some("not-born-yet".to_string());
    req.session_id = Some("session-not-born-yet".to_string());
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "register should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains("start with task cas-abc1"),
        "registration response should surface the pending message: {text}"
    );
    assert!(
        text.contains("waiting for you"),
        "response should explain why the message appears: {text}"
    );

    // Step 3: poll sees it — surfacing in the register response must not
    // consume the message. The daemon's normal poll loop still delivers it.
    let still_pending = env
        .prompt_queue()
        .poll_for_target("not-born-yet", 10)
        .expect("poll");
    assert_eq!(
        still_pending.len(),
        1,
        "message must remain pollable after being surfaced at registration"
    );
}

/// Codex workers register via `action=session_start`, not `action=register`
/// (see cas-e7c8 / the ToolSearch two-step guidance) — this is the literal
/// path the source bug doc's repro hit ("Worker zealous-hawk-40 (codex
/// CLI)"). Must get the same treatment as the Claude `register` path.
#[tokio::test]
async fn test_agent_session_start_surfaces_pending_prompt_queue_mail() {
    let env = FactoryTestEnv::new();

    env.prompt_queue()
        .enqueue_urgent(
            "supervisor",
            "codex-worker-1",
            "branch base: epic/foo. proof command: cargo test.",
            None,
            Some("assignment"),
            None,
            false,
        )
        .expect("enqueue pre-registration message");

    let mut req = coord_req("session_start");
    req.name = Some("codex-worker-1".to_string());
    req.session_id = Some("session-codex-worker-1".to_string());
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "session_start should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        text.contains("branch base: epic/foo"),
        "session_start response should surface the pending message: {text}"
    );
}

/// No pending mail must add no noise — registration stays a clean,
/// unchanged response for the overwhelmingly common case.
#[tokio::test]
async fn test_agent_register_with_no_pending_mail_stays_unchanged() {
    let env = FactoryTestEnv::new();

    let mut req = coord_req("register");
    req.name = Some("fresh-worker".to_string());
    req.session_id = Some("session-fresh-worker".to_string());
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "register should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        !text.contains("waiting for you"),
        "no pending mail should mean no pending-mail section: {text}"
    );
}

/// A message queued for a DIFFERENT worker must never leak into this
/// worker's registration response.
#[tokio::test]
async fn test_agent_register_does_not_leak_other_agents_mail() {
    let env = FactoryTestEnv::new();

    env.prompt_queue()
        .enqueue_urgent(
            "supervisor",
            "someone-else",
            "top secret instructions for someone-else",
            None,
            Some("assignment"),
            None,
            false,
        )
        .expect("enqueue message for a different worker");

    let mut req = coord_req("register");
    req.name = Some("fresh-worker".to_string());
    req.session_id = Some("session-fresh-worker".to_string());
    let result = env.service.coordination(Parameters(req)).await;
    assert!(result.is_ok(), "register should succeed: {result:?}");
    let text = get_text(&result.unwrap());
    assert!(
        !text.contains("top secret instructions"),
        "another agent's queued message must not leak into this registration: {text}"
    );
}

/// `action=interrupt` is sugar for `message` with urgent=true.
#[tokio::test]
async fn test_coordination_interrupt_action_is_urgent() {
    let _guard = EnvGuard::set(&[
        ("CAS_AGENT_ROLE", "supervisor"),
        ("CAS_AGENT_NAME", "supervisor"),
    ]);
    let env = FactoryTestEnv::new();
    env.register_worker("swift-fox");

    // urgent intentionally None — the action alone must force urgent.
    let req = coord_msg(
        "interrupt",
        "swift-fox",
        "abort and re-read the ticket",
        None,
    );
    let result = env.service.coordination(Parameters(req)).await;
    assert!(
        result.is_ok(),
        "interrupt action should succeed: {result:?}"
    );

    let prompts = env.prompt_queue().peek_all(10).expect("peek");
    assert_eq!(prompts.len(), 1);
    assert!(
        prompts[0].urgent,
        "action=interrupt must enqueue urgent even when the urgent flag is omitted"
    );
}

// =============================================================================
// cas-efc4: Heterogeneous Claude+Codex smoke regression tests
//
// Covers the surfaces landed by cas-8aaf / cas-a3ca / cas-4491 / cas-dbbb:
// heterogeneous spawn config (AC1+AC2), model/effort spec propagation (AC2),
// and worker_status metadata for both harness types (AC4).
// The prompt-layer heterogeneous tests (AC3, AC5) live in director/prompts.rs.
// =============================================================================

/// cas-efc4 AC1+AC2: Spawning a Codex worker followed by a Claude worker in the
/// same supervisor session must queue two distinct SpawnRequests.  The Codex
/// entry must carry a worker_spec that encodes the harness; the default-Claude
/// entry must have no spec (session defaults apply).
///
/// This pins the spawn-queue contract for heterogeneous sessions so a
/// regression in `build_spawn_spec_json` or the `spawn_workers` handler is
/// caught at test time, not at factory-start time.
#[tokio::test]
async fn test_efc4_heterogeneous_codex_then_claude_spawn_queued_correctly() {
    let env = FactoryTestEnv::new();
    env.create_epic("Heterogeneous Smoke Epic");

    // --- Codex worker with model + effort overrides ---
    let mut codex_req = factory_req("spawn_workers");
    codex_req.count = Some(1);
    codex_req.cli = Some("codex".to_string());
    codex_req.model = Some("o3".to_string());
    codex_req.effort = Some("high".to_string());
    codex_req.worker_names = Some("codex-alpha".to_string());
    env.service
        .factory(Parameters(codex_req))
        .await
        .expect("codex spawn should succeed");

    // --- Claude worker with no overrides (session defaults) ---
    let mut claude_req = factory_req("spawn_workers");
    claude_req.count = Some(1);
    claude_req.worker_names = Some("claude-beta".to_string());
    env.service
        .factory(Parameters(claude_req))
        .await
        .expect("claude spawn should succeed");

    let entries = env.spawn_queue().peek(10).expect("peek spawn queue");
    assert_eq!(
        entries.len(),
        2,
        "should have exactly 2 spawn queue entries (one Codex, one Claude)"
    );

    // First entry: Codex with spec
    let codex_entry = &entries[0];
    let spec_json = codex_entry
        .worker_spec
        .as_deref()
        .expect("cas-efc4 AC2: Codex spawn entry must carry a worker_spec");
    assert!(
        spec_json.contains("codex"),
        "cas-efc4 AC1: worker_spec must encode the Codex harness: {spec_json}"
    );
    assert!(
        spec_json.contains("o3"),
        "cas-efc4 AC2: worker_spec must encode the model override: {spec_json}"
    );
    assert!(
        spec_json.contains("high"),
        "cas-efc4 AC2: worker_spec must encode the effort override: {spec_json}"
    );

    // Second entry: omitted overrides now resolves to the safe Codex worker floor
    // instead of inheriting the supervisor/session defaults.
    let claude_entry = &entries[1];
    let spec_json = claude_entry
        .worker_spec
        .as_deref()
        .expect("cas-23dc: omitted overrides must still queue a resolved worker_spec");
    let spec: cas_mux::WorkerSpec = serde_json::from_str(spec_json).expect("valid WorkerSpec");
    assert_eq!(spec.cli, cas_mux::SupervisorCli::Codex);
    assert_eq!(spec.model.as_deref(), Some("gpt-5.5"));
    assert_eq!(spec.effort, Some(cas_mux::Effort::Medium));
}

/// cas-efc4 AC2: Model and effort overrides must reach the spawn-queue spec
/// for both Codex and Claude harnesses.  Tests the cross-product so that a
/// future change to `build_spawn_spec_json` for one harness doesn't silently
/// break the other.
#[tokio::test]
async fn test_efc4_model_and_effort_reach_spawn_spec_for_each_harness() {
    let env = FactoryTestEnv::new();
    env.create_epic("Spec Propagation Epic");

    // Codex with model+effort
    let mut codex_req = factory_req("spawn_workers");
    codex_req.count = Some(1);
    codex_req.cli = Some("codex".to_string());
    codex_req.model = Some("o4-mini".to_string());
    codex_req.effort = Some("xhigh".to_string());
    env.service
        .factory(Parameters(codex_req))
        .await
        .expect("codex+model+effort spawn should succeed");

    // Claude with model+effort
    let mut claude_req = factory_req("spawn_workers");
    claude_req.count = Some(1);
    claude_req.cli = Some("claude".to_string());
    claude_req.model = Some("claude-opus-4-5".to_string());
    claude_req.effort = Some("medium".to_string());
    env.service
        .factory(Parameters(claude_req))
        .await
        .expect("claude+model+effort spawn should succeed");

    let entries = env.spawn_queue().peek(10).expect("peek");
    assert_eq!(entries.len(), 2, "expected 2 spec-carrying entries");

    let codex_spec = entries[0]
        .worker_spec
        .as_deref()
        .expect("codex entry must have spec");
    assert!(
        codex_spec.contains("codex"),
        "codex harness in spec: {codex_spec}"
    );
    assert!(
        codex_spec.contains("o4-mini"),
        "codex model in spec: {codex_spec}"
    );
    assert!(
        codex_spec.contains("xhigh"),
        "codex effort in spec: {codex_spec}"
    );

    let claude_spec = entries[1]
        .worker_spec
        .as_deref()
        .expect("claude entry must have spec when cli given");
    assert!(
        claude_spec.contains("claude"),
        "claude harness in spec: {claude_spec}"
    );
    assert!(
        claude_spec.contains("claude-opus-4-5"),
        "claude model in spec: {claude_spec}"
    );
    assert!(
        claude_spec.contains("medium"),
        "claude effort in spec: {claude_spec}"
    );
}

/// cas-efc4 AC4: `worker_status` must surface worktree/git metadata
/// (`clone_path`) for workers of **both** harness types registered in the same
/// session.  Exercises the cas-4491 rendering path across harnesses so that a
/// regression only affecting one type is caught here.
#[tokio::test]
async fn test_efc4_worker_status_shows_clone_path_for_both_harnesses() {
    // Acquire env mutex — prevents concurrent tests that set CAS_AGENT_ROLE
    // from activating supervisor scoping and filtering our test workers out.
    let _guard = EnvGuard::set(&[]);
    let env = FactoryTestEnv::new();

    // Claude worker with clone_path metadata
    let mut claude_meta = HashMap::new();
    claude_meta.insert(
        "clone_path".to_string(),
        "/tmp/cas-worktrees/claude-worker".to_string(),
    );
    env.register_worker_with_metadata("claude-worker", claude_meta);

    // Codex worker with clone_path metadata
    let mut codex_meta = HashMap::new();
    codex_meta.insert(
        "clone_path".to_string(),
        "/tmp/cas-worktrees/codex-worker".to_string(),
    );
    env.register_worker_with_metadata("codex-worker", codex_meta);

    let req = factory_req("worker_status");
    let result = env
        .service
        .factory(Parameters(req))
        .await
        .expect("worker_status should succeed");
    let text = get_text(&result);

    assert!(
        text.contains("Workers (2)"),
        "cas-efc4 AC4: should report 2 active workers: {text}"
    );
    assert!(
        text.contains("claude-worker"),
        "cas-efc4 AC4: Claude worker must appear in status: {text}"
    );
    assert!(
        text.contains("codex-worker"),
        "cas-efc4 AC4: Codex worker must appear in status: {text}"
    );
    assert!(
        text.contains("/tmp/cas-worktrees/claude-worker"),
        "cas-efc4 AC4: Claude worker clone_path must be rendered: {text}"
    );
    assert!(
        text.contains("/tmp/cas-worktrees/codex-worker"),
        "cas-efc4 AC4: Codex worker clone_path must be rendered: {text}"
    );
}
