use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::FactoryConfig;
use crate::core::{FactoryCore, FactoryError, FactoryEvent};
use crate::session::{
    AgentState, SessionCache, SessionId, SessionState, SessionSummary, SessionType,
};

/// Configuration for creating a unified session.
#[derive(Debug, Clone)]
pub struct UnifiedSessionConfig {
    /// Working directory for the session.
    pub cwd: PathBuf,
    /// Number of workers to spawn initially (0 = supervisor only).
    pub worker_count: usize,
    /// Custom worker names (optional).
    pub worker_names: Vec<String>,
    /// Custom supervisor name (optional).
    pub supervisor_name: Option<String>,
    /// Enable worktree-based isolation for workers.
    pub enable_worktrees: bool,
    /// Enable session recording.
    pub record: bool,
    /// Session type (Factory, Managed, Recording).
    pub session_type: SessionType,
}

impl Default for UnifiedSessionConfig {
    fn default() -> Self {
        Self {
            cwd: PathBuf::from("."),
            worker_count: 0,
            worker_names: Vec::new(),
            supervisor_name: None,
            enable_worktrees: true,
            record: false,
            session_type: SessionType::Factory,
        }
    }
}

impl UnifiedSessionConfig {
    /// Create a new config for the given working directory.
    pub fn new(cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            ..Default::default()
        }
    }

    /// Set the session type.
    pub fn with_type(mut self, session_type: SessionType) -> Self {
        self.session_type = session_type;
        self
    }

    /// Set the supervisor name.
    pub fn with_supervisor(mut self, name: impl Into<String>) -> Self {
        self.supervisor_name = Some(name.into());
        self
    }

    /// Set the number of workers.
    pub fn with_workers(mut self, count: usize) -> Self {
        self.worker_count = count;
        self
    }

    /// Set custom worker names.
    pub fn with_worker_names(mut self, names: Vec<String>) -> Self {
        self.worker_names = names;
        self
    }

    /// Enable or disable worktrees.
    pub fn with_worktrees(mut self, enabled: bool) -> Self {
        self.enable_worktrees = enabled;
        self
    }

    /// Enable or disable recording.
    pub fn with_recording(mut self, enabled: bool) -> Self {
        self.record = enabled;
        self
    }
}

/// Unified session manager that wraps FactoryCore instances and maintains
/// a SessionCache for all session state.
///
/// This manager provides a single API for creating, managing, and querying
/// sessions across all session types (Factory, Managed, Recording).
///
/// # Example
///
/// ```ignore
/// use cas_factory::session::{UnifiedSessionManager, UnifiedSessionConfig};
/// use std::path::PathBuf;
///
/// let manager = UnifiedSessionManager::new();
/// let config = UnifiedSessionConfig::new("/project")
///     .with_supervisor("my-supervisor")
///     .with_workers(2);
///
/// let session_id = manager.create_session(config)?;
/// manager.spawn_supervisor(&session_id, None)?;
/// manager.spawn_worker(&session_id, "worker-1", None)?;
/// ```
#[derive(Clone)]
pub struct UnifiedSessionManager {
    /// Thread-safe cache of session summaries.
    cache: SessionCache,
    /// Active FactoryCore instances by session ID.
    /// Uses Mutex instead of RwLock because FactoryCore is Send but not Sync
    /// (contains PTY handles with raw pointers). Mutex<T>: Sync where T: Send.
    cores: Arc<Mutex<HashMap<SessionId, FactoryCore>>>,
    /// Counter for generating session IDs.
    next_id: Arc<Mutex<u64>>,
}

impl Default for UnifiedSessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl UnifiedSessionManager {
    fn cores_guard(&self) -> std::sync::MutexGuard<'_, HashMap<SessionId, FactoryCore>> {
        self.cores
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn next_id_guard(&self) -> std::sync::MutexGuard<'_, u64> {
        self.next_id
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Create a new unified session manager.
    pub fn new() -> Self {
        Self {
            cache: SessionCache::new(),
            cores: Arc::new(Mutex::new(HashMap::new())),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    /// Create a new unified session manager with initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            cache: SessionCache::with_capacity(capacity),
            cores: Arc::new(Mutex::new(HashMap::with_capacity(capacity))),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    /// Get a reference to the session cache.
    pub fn cache(&self) -> &SessionCache {
        &self.cache
    }

    /// Generate a new unique session ID.
    pub(crate) fn generate_id(&self) -> SessionId {
        let mut id_guard = self.next_id_guard();
        let id = format!("unified-{}", *id_guard);
        *id_guard += 1;
        id
    }

    /// Create a new session with the given configuration.
    ///
    /// This creates a FactoryCore, initializes a SessionSummary in the cache,
    /// but does not spawn any agents yet. Call `spawn_supervisor()` to start.
    ///
    /// # Returns
    /// The session ID on success.
    pub fn create_session(
        &self,
        config: UnifiedSessionConfig,
    ) -> std::result::Result<SessionId, FactoryError> {
        let session_id = self.generate_id();

        // Create FactoryConfig from UnifiedSessionConfig
        let factory_config = FactoryConfig {
            cwd: config.cwd.clone(),
            workers: config.worker_count,
            worker_names: config.worker_names.clone(),
            supervisor_name: config.supervisor_name.clone(),
            enable_worktrees: config.enable_worktrees,
            record: config.record,
            session_id: Some(session_id.clone()),
            ..Default::default()
        };

        // Create the FactoryCore
        let mut core = FactoryCore::new(factory_config)?;
        core.set_cas_root(config.cwd.clone());

        // Create session summary and add to cache
        let mut summary = match config.session_type {
            SessionType::Factory => SessionSummary::factory(
                session_id.clone(),
                config.cwd.clone(),
                config.supervisor_name,
            ),
            SessionType::Managed => SessionSummary::managed(session_id.clone(), config.cwd.clone()),
            SessionType::Recording => {
                SessionSummary::recording(session_id.clone(), config.cwd.clone())
            }
        };
        summary.recording_enabled = config.record;

        // Insert into cache and cores
        self.cache.insert(summary);
        self.cores_guard().insert(session_id.clone(), core);

        Ok(session_id)
    }

    /// Get a session summary by ID.
    pub fn get_session(&self, id: &str) -> Option<SessionSummary> {
        self.cache.get(id)
    }

    /// Check if a session exists.
    pub fn contains(&self, id: &str) -> bool {
        self.cache.contains(id)
    }

    /// List all sessions.
    pub fn list_sessions(&self) -> Vec<SessionSummary> {
        self.cache.list()
    }

    /// List session IDs.
    pub fn list_session_ids(&self) -> Vec<SessionId> {
        self.cache.ids()
    }

    /// Filter sessions by type.
    pub fn filter_by_type(&self, session_type: SessionType) -> Vec<SessionSummary> {
        self.cache.filter_by_type(session_type)
    }

    /// Filter sessions by state.
    pub fn filter_by_state(&self, state: SessionState) -> Vec<SessionSummary> {
        self.cache.filter_by_state(state)
    }

    /// Get all factory sessions.
    pub fn factory_sessions(&self) -> Vec<SessionSummary> {
        self.cache.factory_sessions()
    }

    /// Get all active sessions.
    pub fn active_sessions(&self) -> Vec<SessionSummary> {
        self.cache.active_sessions()
    }

    /// Get the number of sessions.
    pub fn len(&self) -> usize {
        self.cache.len()
    }

    /// Check if there are no sessions.
    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    /// Spawn the supervisor for a session.
    ///
    /// Updates the cache with supervisor name and agent state.
    ///
    /// # Arguments
    /// * `session_id` - The session to spawn supervisor in.
    /// * `name` - Optional name override (uses config name if None).
    pub fn spawn_supervisor(
        &self,
        session_id: &str,
        name: Option<&str>,
    ) -> std::result::Result<String, FactoryError> {
        let pane_id = {
            let mut cores = self.cores_guard();
            let core = cores
                .get_mut(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound(session_id.to_string()))?;
            core.spawn_supervisor(name)?
        };

        // Update cache with supervisor info
        self.cache.update(session_id, |summary| {
            summary.supervisor_name = Some(pane_id.clone());
            summary.set_agent_state(&pane_id, AgentState::Active);
            summary.touch();
        });

        Ok(pane_id)
    }

    /// Spawn a worker in a session.
    ///
    /// Updates the cache with worker count and agent state.
    ///
    /// # Arguments
    /// * `session_id` - The session to spawn worker in.
    /// * `name` - Worker name.
    /// * `cwd` - Optional working directory for the worker.
    pub fn spawn_worker(
        &self,
        session_id: &str,
        name: &str,
        cwd: Option<PathBuf>,
    ) -> std::result::Result<String, FactoryError> {
        let pane_id = {
            let mut cores = self.cores_guard();
            let core = cores
                .get_mut(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound(session_id.to_string()))?;
            core.spawn_worker(name, cwd)?
        };

        // Update cache with worker info
        self.cache.update(session_id, |summary| {
            summary.set_agent_state(&pane_id, AgentState::Active);
            summary.touch();
        });

        Ok(pane_id)
    }

    /// Shutdown a worker in a session.
    ///
    /// Updates the cache to reflect the removed worker.
    pub fn shutdown_worker(
        &self,
        session_id: &str,
        name: &str,
    ) -> std::result::Result<(), FactoryError> {
        {
            let mut cores = self.cores_guard();
            let core = cores
                .get_mut(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound(session_id.to_string()))?;
            core.shutdown_worker(name)?;
        }

        // Update cache - mark agent as exited (removes from agent_states)
        self.cache.update(session_id, |summary| {
            summary.set_agent_state(name, AgentState::Exited);
            summary.touch();
        });

        Ok(())
    }

    /// Shutdown the supervisor in a session.
    pub fn shutdown_supervisor(&self, session_id: &str) -> std::result::Result<(), FactoryError> {
        let supervisor_name = {
            let mut cores = self.cores_guard();
            let core = cores
                .get_mut(session_id)
                .ok_or_else(|| FactoryError::SessionNotFound(session_id.to_string()))?;
            let name = core.supervisor_name().map(String::from);
            core.shutdown_supervisor()?;
            name
        };

        // Update cache
        self.cache.update(session_id, |summary| {
            if let Some(name) = &supervisor_name {
                summary.set_agent_state(name, AgentState::Exited);
            }
            summary.supervisor_name = None;
            summary.touch();
        });

        Ok(())
    }

    /// Close a session entirely.
    ///
    /// Shuts down all workers and supervisor, removes from cache and cores.
    pub fn close_session(&self, session_id: &str) -> std::result::Result<(), FactoryError> {
        // Shutdown all workers and supervisor
        {
            let mut cores = self.cores_guard();
            if let Some(core) = cores.get_mut(session_id) {
                // Shutdown workers
                let worker_names: Vec<String> = core.worker_names().to_vec();
                for name in worker_names {
                    let _ = core.shutdown_worker(&name);
                }
                // Shutdown supervisor
                let _ = core.shutdown_supervisor();
                // Stop recordings
                let _ = core.stop_all_recordings();
            }
            // Remove from cores
            cores.remove(session_id);
        }

        // Update cache state to archived, then remove
        self.cache.update(session_id, |summary| {
            summary.state = SessionState::Archived;
            summary.agent_states.clear();
            summary.worker_count = 0;
        });
        self.cache.remove(session_id);

        Ok(())
    }

    /// Poll events from a session's FactoryCore.
    pub fn poll_events(&self, session_id: &str) -> Vec<FactoryEvent> {
        let mut cores = self.cores_guard();
        if let Some(core) = cores.get_mut(session_id) {
            let events = core.poll_events();

            // Update cache based on events
            for event in &events {
                match event {
                    FactoryEvent::PaneOutput { data, .. } => {
                        // Update output bytes and last activity
                        self.cache.update(session_id, |summary| {
                            summary.add_output_bytes(data.len() as u64);
                            summary.touch();
                        });
                    }
                    FactoryEvent::PaneExited { pane_id, .. } => {
                        // Mark agent as exited
                        self.cache.update(session_id, |summary| {
                            summary.set_agent_state(pane_id, AgentState::Exited);
                        });
                    }
                    _ => {}
                }
            }

            events
        } else {
            Vec::new()
        }
    }

    /// Poll all sessions for events.
    pub fn poll_all_events(&self) -> Vec<(SessionId, FactoryEvent)> {
        let session_ids = self.list_session_ids();
        let mut all_events = Vec::new();

        for session_id in session_ids {
            let events = self.poll_events(&session_id);
            for event in events {
                all_events.push((session_id.clone(), event));
            }
        }

        all_events
    }

    /// Update a session's agent state.
    pub fn set_agent_state(&self, session_id: &str, agent_name: &str, state: AgentState) {
        self.cache.update(session_id, |summary| {
            summary.set_agent_state(agent_name, state);
            summary.touch();
        });
    }

    /// Update a session's state (Active, Paused, Archived).
    pub fn set_session_state(&self, session_id: &str, state: SessionState) {
        self.cache.update(session_id, |summary| {
            summary.state = state;
            summary.touch();
        });
    }

    /// Set the epic ID for a session.
    pub fn set_epic(&self, session_id: &str, epic_id: &str) {
        self.cache.update(session_id, |summary| {
            summary.set_epic(epic_id);
        });
    }

    /// Touch a session to update its last activity timestamp.
    pub fn touch(&self, session_id: &str) {
        self.cache.update(session_id, |summary| {
            summary.touch();
        });
    }

    /// Execute a function with the FactoryCore for a session.
    ///
    /// Provides direct access to the underlying FactoryCore for
    /// operations not exposed through the manager API.
    pub fn with_core<F, R>(&self, session_id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut FactoryCore) -> R,
    {
        let mut cores = self.cores_guard();
        cores.get_mut(session_id).map(f)
    }

    /// Execute a read-only function with the FactoryCore for a session.
    pub fn with_core_ref<F, R>(&self, session_id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&FactoryCore) -> R,
    {
        let cores = self.cores_guard();
        cores.get(session_id).map(f)
    }

    /// Get supervisor name for a session.
    pub fn supervisor_name(&self, session_id: &str) -> Option<String> {
        self.cache.get(session_id).and_then(|s| s.supervisor_name)
    }

    /// Get worker names for a session.
    pub fn worker_names(&self, session_id: &str) -> Vec<String> {
        self.with_core_ref(session_id, |core| core.worker_names().to_vec())
            .unwrap_or_default()
    }

    /// Check if a session has active workers.
    pub fn has_active_workers(&self, session_id: &str) -> bool {
        self.cache
            .get(session_id)
            .map(|s| s.worker_count > 0)
            .unwrap_or(false)
    }

    /// Check if recording is enabled for a session.
    pub fn is_recording(&self, session_id: &str) -> bool {
        self.cache
            .get(session_id)
            .map(|s| s.recording_enabled)
            .unwrap_or(false)
    }
}

// Shared type alias for use in Tauri state
pub type SharedUnifiedSessionManager = Arc<UnifiedSessionManager>;

/// Create a new shared unified session manager.
pub fn new_shared_unified_manager() -> SharedUnifiedSessionManager {
    Arc::new(UnifiedSessionManager::new())
}
