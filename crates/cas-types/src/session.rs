//! Session tracking for analytics
//!
//! Tracks Claude Code session boundaries for enterprise observability.
//! Sessions capture start/end times, aggregate metrics, and outcomes.
//!
//! # Session Outcomes
//!
//! Sessions are classified by their outcome when they end:
//! - **TasksCompleted**: At least one task was closed
//! - **LearningsCreated**: Memories stored but no tasks closed
//! - **Exploration**: Reads/searches only, informational
//! - **Abandoned**: Started work but no productive outcome
//! - **Error**: Session ended with unrecovered errors

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::TypeError;

/// Outcome classification for a completed session
///
/// Used by Factory Signals to understand session productivity patterns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOutcome {
    /// At least one task was closed during the session
    TasksCompleted,
    /// Memories/learnings were stored, but no tasks closed
    LearningsCreated,
    /// Read/search operations only - informational session
    Exploration,
    /// Started work but no productive outcome detected
    Abandoned,
    /// Session ended with unrecovered errors
    Error,
}

impl fmt::Display for SessionOutcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionOutcome::TasksCompleted => write!(f, "tasks_completed"),
            SessionOutcome::LearningsCreated => write!(f, "learnings_created"),
            SessionOutcome::Exploration => write!(f, "exploration"),
            SessionOutcome::Abandoned => write!(f, "abandoned"),
            SessionOutcome::Error => write!(f, "error"),
        }
    }
}

impl FromStr for SessionOutcome {
    type Err = TypeError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "tasks_completed" => Ok(SessionOutcome::TasksCompleted),
            "learnings_created" => Ok(SessionOutcome::LearningsCreated),
            "exploration" => Ok(SessionOutcome::Exploration),
            "abandoned" => Ok(SessionOutcome::Abandoned),
            "error" => Ok(SessionOutcome::Error),
            _ => Err(TypeError::Parse(format!("invalid session outcome: {s}"))),
        }
    }
}

/// A tracked Claude Code session
///
/// Sessions are created on SessionStart hook and closed on Stop/SessionEnd hooks.
/// Duration is computed when the session ends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// Unique session ID from Claude Code
    pub session_id: String,

    /// Working directory at session start
    pub cwd: String,

    /// When the session started
    pub started_at: DateTime<Utc>,

    /// When the session ended (None if still active)
    pub ended_at: Option<DateTime<Utc>>,

    /// Duration in seconds (computed on end)
    pub duration_secs: Option<i64>,

    /// Permission mode (e.g., "plan", "default")
    pub permission_mode: Option<String>,

    /// Number of entries created during session
    pub entries_created: u32,

    /// Number of tasks closed during session
    pub tasks_closed: u32,

    /// Number of tool uses captured
    pub tool_uses: u32,

    /// Project/team identifier (for cloud sync)
    pub team_id: Option<String>,

    /// AI-generated session title (e.g., "Implemented usage tracking dashboard")
    pub title: Option<String>,

    /// Session outcome classification (computed on end)
    pub outcome: Option<SessionOutcome>,

    /// Aggregated friction score from 0.0 (smooth) to 1.0 (high friction)
    pub friction_score: Option<f32>,

    /// Count of positive signals (delight events) during session
    pub delight_count: u32,
}

impl Session {
    /// Create a new session starting now
    pub fn new(session_id: String, cwd: String, permission_mode: Option<String>) -> Self {
        Self {
            session_id,
            cwd,
            started_at: Utc::now(),
            ended_at: None,
            duration_secs: None,
            permission_mode,
            entries_created: 0,
            tasks_closed: 0,
            tool_uses: 0,
            team_id: None,
            title: None,
            outcome: None,
            friction_score: None,
            delight_count: 0,
        }
    }

    /// End the session and compute duration
    pub fn end(&mut self) {
        let now = Utc::now();
        self.ended_at = Some(now);
        self.duration_secs = Some((now - self.started_at).num_seconds());
    }

    /// Check if session is still active
    pub fn is_active(&self) -> bool {
        self.ended_at.is_none()
    }

    /// Increment entries created counter
    pub fn increment_entries(&mut self) {
        self.entries_created += 1;
    }

    /// Increment tasks closed counter
    pub fn increment_tasks(&mut self) {
        self.tasks_closed += 1;
    }

    /// Increment tool uses counter
    pub fn increment_tool_uses(&mut self) {
        self.tool_uses += 1;
    }

    /// Increment delight count
    pub fn increment_delight(&mut self) {
        self.delight_count += 1;
    }

    /// Set the friction score (clamped to 0.0-1.0)
    pub fn set_friction_score(&mut self, score: f32) {
        self.friction_score = Some(score.clamp(0.0, 1.0));
    }

    /// Set the session outcome directly
    pub fn set_outcome(&mut self, outcome: SessionOutcome) {
        self.outcome = Some(outcome);
    }

    /// Compute and set the session outcome based on metrics
    ///
    /// Outcome computation logic:
    /// - TasksCompleted: tasks_closed > 0
    /// - LearningsCreated: entries_created > 0 AND tasks_closed == 0
    /// - Exploration: tool_uses > 0 AND entries_created == 0 AND tasks_closed == 0
    /// - Abandoned: none of the above (started but nothing productive)
    ///
    /// Note: Error outcome should be set explicitly via set_outcome() when errors occur.
    pub fn compute_outcome(&mut self) {
        // Don't overwrite if outcome was set explicitly (e.g., Error)
        if self.outcome.is_some() {
            return;
        }

        self.outcome = Some(if self.tasks_closed > 0 {
            SessionOutcome::TasksCompleted
        } else if self.entries_created > 0 {
            SessionOutcome::LearningsCreated
        } else if self.tool_uses > 0 {
            SessionOutcome::Exploration
        } else {
            SessionOutcome::Abandoned
        });
    }

    /// End the session, compute duration, and determine outcome
    pub fn end_with_outcome(&mut self) {
        self.end();
        self.compute_outcome();
    }
}

impl Default for Session {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            cwd: String::new(),
            started_at: Utc::now(),
            ended_at: None,
            duration_secs: None,
            permission_mode: None,
            entries_created: 0,
            tasks_closed: 0,
            tool_uses: 0,
            team_id: None,
            title: None,
            outcome: None,
            friction_score: None,
            delight_count: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::session::*;

    #[test]
    fn test_new_session() {
        let session = Session::new(
            "test-123".to_string(),
            "/home/user/project".to_string(),
            Some("plan".to_string()),
        );

        assert_eq!(session.session_id, "test-123");
        assert_eq!(session.cwd, "/home/user/project");
        assert_eq!(session.permission_mode, Some("plan".to_string()));
        assert!(session.is_active());
        assert_eq!(session.entries_created, 0);
    }

    #[test]
    fn test_end_session() {
        let mut session = Session::new("test-456".to_string(), "/project".to_string(), None);

        session.end();

        assert!(!session.is_active());
        assert!(session.ended_at.is_some());
        assert!(session.duration_secs.is_some());
        assert!(session.duration_secs.unwrap() >= 0);
    }

    #[test]
    fn test_increment_counters() {
        let mut session = Session::default();

        session.increment_entries();
        session.increment_entries();
        session.increment_tasks();
        session.increment_tool_uses();
        session.increment_tool_uses();
        session.increment_tool_uses();

        assert_eq!(session.entries_created, 2);
        assert_eq!(session.tasks_closed, 1);
        assert_eq!(session.tool_uses, 3);
    }

    #[test]
    fn test_session_outcome_display() {
        assert_eq!(
            SessionOutcome::TasksCompleted.to_string(),
            "tasks_completed"
        );
        assert_eq!(
            SessionOutcome::LearningsCreated.to_string(),
            "learnings_created"
        );
        assert_eq!(SessionOutcome::Exploration.to_string(), "exploration");
        assert_eq!(SessionOutcome::Abandoned.to_string(), "abandoned");
        assert_eq!(SessionOutcome::Error.to_string(), "error");
    }

    #[test]
    fn test_session_outcome_from_str() {
        assert_eq!(
            SessionOutcome::from_str("tasks_completed").unwrap(),
            SessionOutcome::TasksCompleted
        );
        assert_eq!(
            SessionOutcome::from_str("LEARNINGS_CREATED").unwrap(),
            SessionOutcome::LearningsCreated
        );
        assert_eq!(
            SessionOutcome::from_str("exploration").unwrap(),
            SessionOutcome::Exploration
        );
        assert!(SessionOutcome::from_str("invalid").is_err());
    }

    #[test]
    fn test_compute_outcome_tasks_completed() {
        let mut session = Session::default();
        session.increment_tasks();
        session.increment_entries(); // Also has entries, but tasks take priority
        session.compute_outcome();

        assert_eq!(session.outcome, Some(SessionOutcome::TasksCompleted));
    }

    #[test]
    fn test_compute_outcome_learnings_created() {
        let mut session = Session::default();
        session.increment_entries();
        session.increment_tool_uses();
        session.compute_outcome();

        assert_eq!(session.outcome, Some(SessionOutcome::LearningsCreated));
    }

    #[test]
    fn test_compute_outcome_exploration() {
        let mut session = Session::default();
        session.increment_tool_uses();
        session.increment_tool_uses();
        session.compute_outcome();

        assert_eq!(session.outcome, Some(SessionOutcome::Exploration));
    }

    #[test]
    fn test_compute_outcome_abandoned() {
        let mut session = Session::default();
        session.compute_outcome();

        assert_eq!(session.outcome, Some(SessionOutcome::Abandoned));
    }

    #[test]
    fn test_compute_outcome_preserves_explicit() {
        let mut session = Session::default();
        session.set_outcome(SessionOutcome::Error);
        session.increment_tasks(); // Would normally be TasksCompleted
        session.compute_outcome();

        // Should preserve the explicitly set Error outcome
        assert_eq!(session.outcome, Some(SessionOutcome::Error));
    }

    #[test]
    fn test_end_with_outcome() {
        let mut session = Session::default();
        session.increment_tasks();
        session.end_with_outcome();

        assert!(!session.is_active());
        assert!(session.ended_at.is_some());
        assert_eq!(session.outcome, Some(SessionOutcome::TasksCompleted));
    }

    #[test]
    fn test_friction_score_clamped() {
        let mut session = Session::default();

        session.set_friction_score(0.5);
        assert_eq!(session.friction_score, Some(0.5));

        session.set_friction_score(-0.5);
        assert_eq!(session.friction_score, Some(0.0));

        session.set_friction_score(1.5);
        assert_eq!(session.friction_score, Some(1.0));
    }

    #[test]
    fn test_delight_count() {
        let mut session = Session::default();

        session.increment_delight();
        session.increment_delight();
        session.increment_delight();

        assert_eq!(session.delight_count, 3);
    }

    #[test]
    fn test_new_session_has_signal_fields() {
        let session = Session::new("test".to_string(), "/".to_string(), None);

        assert_eq!(session.outcome, None);
        assert_eq!(session.friction_score, None);
        assert_eq!(session.delight_count, 0);
    }
}
