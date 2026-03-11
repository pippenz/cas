use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Unique identifier for a factory session
pub type SessionId = String;

/// Session lifecycle state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// Session is currently active and running
    Active,
    /// Session is paused (can be resumed)
    Paused,
    /// Session has been archived (read-only history)
    Archived,
}

impl fmt::Display for SessionState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionState::Active => write!(f, "active"),
            SessionState::Paused => write!(f, "paused"),
            SessionState::Archived => write!(f, "archived"),
        }
    }
}

/// Type of session determining its capabilities and behavior
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionType {
    /// Multi-agent factory session with supervisor and workers
    #[default]
    Factory,
    /// Single-agent managed session with auto-save features
    Managed,
    /// Session recording/playback mode
    Recording,
}

impl fmt::Display for SessionType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionType::Factory => write!(f, "factory"),
            SessionType::Managed => write!(f, "managed"),
            SessionType::Recording => write!(f, "recording"),
        }
    }
}

/// State of an individual agent within a session
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent is actively processing
    #[default]
    Active,
    /// Agent is idle, waiting for work
    Idle,
    /// Agent is blocked on a dependency or user input
    Blocked,
    /// Agent has exited or been shut down
    Exited,
}

impl fmt::Display for AgentState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentState::Active => write!(f, "active"),
            AgentState::Idle => write!(f, "idle"),
            AgentState::Blocked => write!(f, "blocked"),
            AgentState::Exited => write!(f, "exited"),
        }
    }
}

/// Unified session summary with full metadata for Tauri commands.
///
/// This struct provides a serializable view of session state that can be
/// used across different session types (Factory, Managed, Recording).
/// Designed for use in desktop UI and API responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSummary {
    /// Unique session identifier
    pub id: SessionId,

    /// Type of session (Factory, Managed, Recording)
    pub session_type: SessionType,

    /// Current session state (Active, Paused, Archived)
    pub state: SessionState,

    /// Number of active workers in this session
    pub worker_count: usize,

    /// Name of the supervisor agent (for Factory sessions)
    pub supervisor_name: Option<String>,

    /// When the session was created
    pub created_at: DateTime<Utc>,

    /// Project directory for this session
    pub project_dir: PathBuf,

    /// Whether session recording is enabled
    pub recording_enabled: bool,

    /// Associated epic ID (if working on an epic)
    pub epic_id: Option<String>,

    /// Timestamp of the last activity in this session
    pub last_activity: DateTime<Utc>,

    /// Total bytes of output captured across all agents
    pub total_output_bytes: u64,

    /// State of each agent by name
    #[serde(default)]
    pub agent_states: HashMap<String, AgentState>,
}

impl SessionSummary {
    /// Create a new session summary with default values
    pub fn new(id: SessionId, session_type: SessionType, project_dir: PathBuf) -> Self {
        let now = Utc::now();
        Self {
            id,
            session_type,
            state: SessionState::Active,
            worker_count: 0,
            supervisor_name: None,
            created_at: now,
            project_dir,
            recording_enabled: false,
            epic_id: None,
            last_activity: now,
            total_output_bytes: 0,
            agent_states: HashMap::new(),
        }
    }

    /// Create a factory session summary
    pub fn factory(id: SessionId, project_dir: PathBuf, supervisor_name: Option<String>) -> Self {
        let mut summary = Self::new(id, SessionType::Factory, project_dir);
        summary.supervisor_name = supervisor_name;
        summary
    }

    /// Create a managed session summary
    pub fn managed(id: SessionId, project_dir: PathBuf) -> Self {
        Self::new(id, SessionType::Managed, project_dir)
    }

    /// Create a recording session summary
    pub fn recording(id: SessionId, project_dir: PathBuf) -> Self {
        let mut summary = Self::new(id, SessionType::Recording, project_dir);
        summary.recording_enabled = true;
        summary
    }

    /// Update the last activity timestamp
    pub fn touch(&mut self) {
        self.last_activity = Utc::now();
    }

    /// Set the epic ID
    pub fn set_epic(&mut self, epic_id: impl Into<String>) {
        self.epic_id = Some(epic_id.into());
    }

    /// Add or update an agent's state
    pub fn set_agent_state(&mut self, name: impl Into<String>, state: AgentState) {
        let name = name.into();
        if state == AgentState::Exited {
            self.agent_states.remove(&name);
        } else {
            self.agent_states.insert(name, state);
        }
        self.worker_count = self
            .agent_states
            .values()
            .filter(|s| **s != AgentState::Exited)
            .count();
    }

    /// Add bytes to the total output count
    pub fn add_output_bytes(&mut self, bytes: u64) {
        self.total_output_bytes = self.total_output_bytes.saturating_add(bytes);
    }

    /// Check if this is a factory session
    pub fn is_factory(&self) -> bool {
        self.session_type == SessionType::Factory
    }

    /// Check if this is a managed session
    pub fn is_managed(&self) -> bool {
        self.session_type == SessionType::Managed
    }

    /// Check if this is a recording session
    pub fn is_recording(&self) -> bool {
        self.session_type == SessionType::Recording
    }

    /// Get count of active agents (not exited)
    pub fn active_agent_count(&self) -> usize {
        self.agent_states
            .values()
            .filter(|s| **s != AgentState::Exited)
            .count()
    }
}

impl Default for SessionSummary {
    fn default() -> Self {
        Self::new(
            generate_session_id(),
            SessionType::Factory,
            PathBuf::from("."),
        )
    }
}

/// Thread-safe cache for session summaries.
///
/// Provides concurrent access to session data for multiple readers
/// with exclusive write access. Suitable for use in Tauri commands
/// and multi-threaded contexts.
///
/// # Example
///
/// ```ignore
/// use cas_factory::session::{SessionCache, SessionSummary, SessionType};
/// use std::path::PathBuf;
///
/// let cache = SessionCache::new();
/// let summary = SessionSummary::factory(
///     "session-1".to_string(),
///     PathBuf::from("/project"),
///     Some("supervisor".to_string()),
/// );
///
/// cache.insert(summary);
/// assert!(cache.get("session-1").is_some());
/// ```
#[derive(Debug, Clone, Default)]
pub struct SessionCache {
    inner: Arc<RwLock<HashMap<SessionId, SessionSummary>>>,
}

impl SessionCache {
    /// Create a new empty session cache
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a session cache with initial capacity
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::with_capacity(capacity))),
        }
    }

    fn read_guard(&self) -> std::sync::RwLockReadGuard<'_, HashMap<SessionId, SessionSummary>> {
        self.inner
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn write_guard(&self) -> std::sync::RwLockWriteGuard<'_, HashMap<SessionId, SessionSummary>> {
        self.inner
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Get a session by ID
    ///
    /// Returns a clone of the session summary if found.
    pub fn get(&self, id: &str) -> Option<SessionSummary> {
        self.read_guard().get(id).cloned()
    }

    /// Insert a session into the cache
    ///
    /// If a session with the same ID already exists, it is replaced.
    /// Returns the previous session if one existed.
    pub fn insert(&self, session: SessionSummary) -> Option<SessionSummary> {
        self.write_guard().insert(session.id.clone(), session)
    }

    /// Update a session in the cache
    ///
    /// Calls the provided function with a mutable reference to the session.
    /// Returns true if the session was found and updated, false otherwise.
    pub fn update<F>(&self, id: &str, f: F) -> bool
    where
        F: FnOnce(&mut SessionSummary),
    {
        let mut guard = self.write_guard();
        if let Some(session) = guard.get_mut(id) {
            f(session);
            true
        } else {
            false
        }
    }

    /// Remove a session from the cache
    ///
    /// Returns the removed session if it existed.
    pub fn remove(&self, id: &str) -> Option<SessionSummary> {
        self.write_guard().remove(id)
    }

    /// Check if a session exists in the cache
    pub fn contains(&self, id: &str) -> bool {
        self.read_guard().contains_key(id)
    }

    /// Get the number of sessions in the cache
    pub fn len(&self) -> usize {
        self.read_guard().len()
    }

    /// Check if the cache is empty
    pub fn is_empty(&self) -> bool {
        self.read_guard().is_empty()
    }

    /// List all sessions in the cache
    ///
    /// Returns clones of all session summaries.
    pub fn list(&self) -> Vec<SessionSummary> {
        self.read_guard().values().cloned().collect()
    }

    /// List all session IDs
    pub fn ids(&self) -> Vec<SessionId> {
        self.read_guard().keys().cloned().collect()
    }

    /// Filter sessions by type
    ///
    /// Returns all sessions matching the given session type.
    pub fn filter_by_type(&self, session_type: SessionType) -> Vec<SessionSummary> {
        self.read_guard()
            .values()
            .filter(|s| s.session_type == session_type)
            .cloned()
            .collect()
    }

    /// Filter sessions by state
    ///
    /// Returns all sessions matching the given session state.
    pub fn filter_by_state(&self, state: SessionState) -> Vec<SessionSummary> {
        self.read_guard()
            .values()
            .filter(|s| s.state == state)
            .cloned()
            .collect()
    }

    /// Get all factory sessions
    pub fn factory_sessions(&self) -> Vec<SessionSummary> {
        self.filter_by_type(SessionType::Factory)
    }

    /// Get all managed sessions
    pub fn managed_sessions(&self) -> Vec<SessionSummary> {
        self.filter_by_type(SessionType::Managed)
    }

    /// Get all recording sessions
    pub fn recording_sessions(&self) -> Vec<SessionSummary> {
        self.filter_by_type(SessionType::Recording)
    }

    /// Get all active sessions
    pub fn active_sessions(&self) -> Vec<SessionSummary> {
        self.filter_by_state(SessionState::Active)
    }

    /// Clear all sessions from the cache
    pub fn clear(&self) {
        self.write_guard().clear();
    }

    /// Get or insert a session
    ///
    /// If the session exists, returns a clone. Otherwise, calls the
    /// provided function to create a new session, inserts it, and
    /// returns a clone.
    pub fn get_or_insert_with<F>(&self, id: &str, f: F) -> SessionSummary
    where
        F: FnOnce() -> SessionSummary,
    {
        // Try read first
        {
            let guard = self.read_guard();
            if let Some(session) = guard.get(id) {
                return session.clone();
            }
        }

        // Need to write
        let mut guard = self.write_guard();
        // Double-check after acquiring write lock
        if let Some(session) = guard.get(id) {
            return session.clone();
        }

        let session = f();
        let result = session.clone();
        guard.insert(id.to_string(), session);
        result
    }
}

/// Metadata about a factory session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Unique session identifier
    pub id: SessionId,
    /// Human-readable session name
    pub name: String,
    /// Project directory for this session
    pub project_dir: PathBuf,
    /// Current session state
    pub state: SessionState,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// When the session was last active
    pub last_active_at: DateTime<Utc>,
    /// When the session was paused (if paused)
    pub paused_at: Option<DateTime<Utc>>,
    /// When the session was archived (if archived)
    pub archived_at: Option<DateTime<Utc>>,
    /// Supervisor name
    pub supervisor_name: Option<String>,
    /// Worker names
    pub worker_names: Vec<String>,
    /// Associated epic ID (if any)
    pub epic_id: Option<String>,
    /// Total active duration in seconds (excluding paused time)
    pub total_active_secs: i64,
    /// Custom metadata
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl SessionInfo {
    /// Create a new session with the given name and project directory
    pub fn new(name: impl Into<String>, project_dir: impl Into<PathBuf>) -> Self {
        let now = Utc::now();
        Self {
            id: generate_session_id(),
            name: name.into(),
            project_dir: project_dir.into(),
            state: SessionState::Active,
            created_at: now,
            last_active_at: now,
            paused_at: None,
            archived_at: None,
            supervisor_name: None,
            worker_names: Vec::new(),
            epic_id: None,
            total_active_secs: 0,
            metadata: HashMap::new(),
        }
    }

    /// Check if session is active
    pub fn is_active(&self) -> bool {
        self.state == SessionState::Active
    }

    /// Check if session is paused
    pub fn is_paused(&self) -> bool {
        self.state == SessionState::Paused
    }

    /// Check if session is archived
    pub fn is_archived(&self) -> bool {
        self.state == SessionState::Archived
    }

    /// Update last active timestamp and accumulate active time
    pub fn touch(&mut self) {
        let now = Utc::now();
        if self.state == SessionState::Active {
            // Accumulate active time since last touch
            let elapsed = (now - self.last_active_at).num_seconds();
            if elapsed > 0 && elapsed < 3600 {
                // Cap at 1 hour to avoid counting long idle periods
                self.total_active_secs += elapsed;
            }
        }
        self.last_active_at = now;
    }

    /// Set the supervisor name
    pub fn set_supervisor(&mut self, name: impl Into<String>) {
        self.supervisor_name = Some(name.into());
    }

    /// Add a worker name
    pub fn add_worker(&mut self, name: impl Into<String>) {
        self.worker_names.push(name.into());
    }

    /// Remove a worker name
    pub fn remove_worker(&mut self, name: &str) {
        self.worker_names.retain(|n| n != name);
    }

    /// Set the associated epic
    pub fn set_epic(&mut self, epic_id: impl Into<String>) {
        self.epic_id = Some(epic_id.into());
    }

    /// Add custom metadata
    pub fn set_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
    }
}

/// Errors from session operations
#[derive(Error, Debug)]
pub enum SessionError {
    /// Session not found
    #[error("Session '{0}' not found")]
    NotFound(SessionId),

    /// Session already exists
    #[error("Session '{0}' already exists")]
    AlreadyExists(SessionId),

    /// Invalid state transition
    #[error("Cannot {0} session in {1} state")]
    InvalidTransition(String, SessionState),

    /// IO error
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    /// Serialization error
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Result type for session operations
pub type Result<T> = std::result::Result<T, SessionError>;

pub(crate) fn generate_session_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();

    // Format: fs-<timestamp>-<random>
    format!("fs-{:x}-{:04x}", timestamp, rand_u16())
}

/// Generate a random u16 for session ID uniqueness
fn rand_u16() -> u16 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::time::Instant;

    let mut hasher = DefaultHasher::new();
    Instant::now().hash(&mut hasher);
    std::process::id().hash(&mut hasher);
    hasher.finish() as u16
}
