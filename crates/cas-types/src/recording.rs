//! Terminal recording types for CAS
//!
//! Provides types for tracking terminal recordings and their associated
//! agents for time-travel playback in factory sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A terminal recording session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recording {
    /// Unique identifier (rec-{short_hash})
    pub id: String,

    /// Session ID this recording belongs to
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// When recording started
    pub started_at: DateTime<Utc>,

    /// When recording ended
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,

    /// Duration in milliseconds
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<i64>,

    /// Path to the recording file
    pub file_path: String,

    /// Size of the recording file in bytes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_size: Option<i64>,

    /// Optional title for the recording
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Optional description
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// When the record was created
    pub created_at: DateTime<Utc>,
}

impl Recording {
    /// Generate a unique recording ID
    pub fn generate_id() -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};

        static COUNTER: AtomicU64 = AtomicU64::new(0);

        let mut hasher = DefaultHasher::new();
        Utc::now().timestamp_nanos_opt().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);
        let hash = hasher.finish();
        format!("rec-{:08x}", hash as u32)
    }

    /// Create a new recording
    pub fn new(file_path: String) -> Self {
        let now = Utc::now();
        Self {
            id: Self::generate_id(),
            session_id: None,
            started_at: now,
            ended_at: None,
            duration_ms: None,
            file_path,
            file_size: None,
            title: None,
            description: None,
            created_at: now,
        }
    }

    /// Create a recording for a specific session
    pub fn for_session(session_id: String, file_path: String) -> Self {
        let mut recording = Self::new(file_path);
        recording.session_id = Some(session_id);
        recording
    }

    /// Mark the recording as ended
    pub fn end(&mut self) {
        let now = Utc::now();
        self.ended_at = Some(now);
        self.duration_ms = Some((now - self.started_at).num_milliseconds());
    }
}

/// An agent's participation in a recording
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingAgent {
    /// Auto-incrementing ID
    pub id: i64,

    /// Recording this agent belongs to
    pub recording_id: String,

    /// Name of the agent (e.g., "swift-fox")
    pub agent_name: String,

    /// Type of agent (e.g., "worker", "supervisor")
    pub agent_type: String,

    /// Path to the agent-specific recording file
    pub file_path: String,

    /// When the record was created
    pub created_at: DateTime<Utc>,
}

impl RecordingAgent {
    /// Create a new recording agent entry
    pub fn new(
        recording_id: String,
        agent_name: String,
        agent_type: String,
        file_path: String,
    ) -> Self {
        Self {
            id: 0, // Set by database
            recording_id,
            agent_name,
            agent_type,
            file_path,
            created_at: Utc::now(),
        }
    }
}

/// Event type for recording events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordingEventType {
    /// Task was created
    TaskCreated,
    /// Task was started
    TaskStarted,
    /// Task was completed
    TaskCompleted,
    /// Task was deleted
    TaskDeleted,
    /// Task was blocked
    TaskBlocked,
    /// Memory was created
    MemoryCreated,
    /// Rule was created
    RuleCreated,
    /// Skill was invoked
    SkillInvoked,
    /// Agent joined the session
    AgentJoined,
    /// Agent left the session
    AgentLeft,
    /// Message was sent
    MessageSent,
    /// Custom event type
    Custom,
}

impl std::fmt::Display for RecordingEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecordingEventType::TaskCreated => write!(f, "task_created"),
            RecordingEventType::TaskStarted => write!(f, "task_started"),
            RecordingEventType::TaskCompleted => write!(f, "task_completed"),
            RecordingEventType::TaskDeleted => write!(f, "task_deleted"),
            RecordingEventType::TaskBlocked => write!(f, "task_blocked"),
            RecordingEventType::MemoryCreated => write!(f, "memory_created"),
            RecordingEventType::RuleCreated => write!(f, "rule_created"),
            RecordingEventType::SkillInvoked => write!(f, "skill_invoked"),
            RecordingEventType::AgentJoined => write!(f, "agent_joined"),
            RecordingEventType::AgentLeft => write!(f, "agent_left"),
            RecordingEventType::MessageSent => write!(f, "message_sent"),
            RecordingEventType::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for RecordingEventType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "task_created" => Ok(RecordingEventType::TaskCreated),
            "task_started" => Ok(RecordingEventType::TaskStarted),
            "task_completed" => Ok(RecordingEventType::TaskCompleted),
            "task_deleted" => Ok(RecordingEventType::TaskDeleted),
            "task_blocked" => Ok(RecordingEventType::TaskBlocked),
            "memory_created" => Ok(RecordingEventType::MemoryCreated),
            "rule_created" => Ok(RecordingEventType::RuleCreated),
            "skill_invoked" => Ok(RecordingEventType::SkillInvoked),
            "agent_joined" => Ok(RecordingEventType::AgentJoined),
            "agent_left" => Ok(RecordingEventType::AgentLeft),
            "message_sent" => Ok(RecordingEventType::MessageSent),
            "custom" => Ok(RecordingEventType::Custom),
            _ => Err(format!("Unknown recording event type: {s}")),
        }
    }
}

/// An event within a recording, correlated to CAS entities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingEvent {
    /// Auto-incrementing ID
    pub id: i64,

    /// Recording this event belongs to
    pub recording_id: String,

    /// Timestamp within the recording (milliseconds from start)
    pub timestamp_ms: i64,

    /// Type of event
    pub event_type: RecordingEventType,

    /// Type of CAS entity (e.g., "task", "memory", "rule")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_type: Option<String>,

    /// ID of the CAS entity
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,

    /// Additional metadata as JSON
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

impl RecordingEvent {
    /// Create a new recording event
    pub fn new(recording_id: String, timestamp_ms: i64, event_type: RecordingEventType) -> Self {
        Self {
            id: 0, // Set by database
            recording_id,
            timestamp_ms,
            event_type,
            entity_type: None,
            entity_id: None,
            metadata: None,
        }
    }

    /// Create an event linked to a CAS entity
    pub fn for_entity(
        recording_id: String,
        timestamp_ms: i64,
        event_type: RecordingEventType,
        entity_type: String,
        entity_id: String,
    ) -> Self {
        Self {
            id: 0,
            recording_id,
            timestamp_ms,
            event_type,
            entity_type: Some(entity_type),
            entity_id: Some(entity_id),
            metadata: None,
        }
    }
}

/// Query filter for recordings
#[derive(Debug, Clone, Default)]
pub struct RecordingQuery {
    /// Filter by session ID
    pub session_id: Option<String>,
    /// Filter by date range start
    pub from_date: Option<DateTime<Utc>>,
    /// Filter by date range end
    pub to_date: Option<DateTime<Utc>>,
    /// Filter by agent name
    pub agent_name: Option<String>,
    /// Maximum results to return
    pub limit: Option<usize>,
}

impl RecordingQuery {
    /// Create a new empty query
    pub fn new() -> Self {
        Self::default()
    }

    /// Filter by session
    pub fn for_session(mut self, session_id: String) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Filter by date range
    pub fn in_range(mut self, from: DateTime<Utc>, to: DateTime<Utc>) -> Self {
        self.from_date = Some(from);
        self.to_date = Some(to);
        self
    }

    /// Filter by agent
    pub fn by_agent(mut self, agent_name: String) -> Self {
        self.agent_name = Some(agent_name);
        self
    }

    /// Limit results
    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit);
        self
    }
}

#[cfg(test)]
mod tests {
    use crate::recording::*;

    #[test]
    fn test_recording_id_generation() {
        let id1 = Recording::generate_id();
        let id2 = Recording::generate_id();

        assert!(id1.starts_with("rec-"));
        assert!(id2.starts_with("rec-"));
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_recording_new() {
        let rec = Recording::new("/path/to/recording.bin".to_string());

        assert!(rec.id.starts_with("rec-"));
        assert!(rec.session_id.is_none());
        assert!(rec.ended_at.is_none());
        assert_eq!(rec.file_path, "/path/to/recording.bin");
    }

    #[test]
    fn test_recording_end() {
        let mut rec = Recording::new("/path/to/recording.bin".to_string());
        rec.end();

        assert!(rec.ended_at.is_some());
        assert!(rec.duration_ms.is_some());
        assert!(rec.duration_ms.unwrap() >= 0);
    }

    #[test]
    fn test_recording_event_type_display() {
        assert_eq!(RecordingEventType::TaskCreated.to_string(), "task_created");
        assert_eq!(
            RecordingEventType::MemoryCreated.to_string(),
            "memory_created"
        );
    }

    #[test]
    fn test_recording_event_type_parse() {
        assert_eq!(
            "task_created".parse::<RecordingEventType>().unwrap(),
            RecordingEventType::TaskCreated
        );
        assert_eq!(
            "memory_created".parse::<RecordingEventType>().unwrap(),
            RecordingEventType::MemoryCreated
        );
    }

    #[test]
    fn test_recording_query_builder() {
        let query = RecordingQuery::new()
            .for_session("session-1".to_string())
            .by_agent("swift-fox".to_string())
            .with_limit(10);

        assert_eq!(query.session_id, Some("session-1".to_string()));
        assert_eq!(query.agent_name, Some("swift-fox".to_string()));
        assert_eq!(query.limit, Some(10));
    }
}
