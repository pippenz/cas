use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::session::{Result, SessionError, SessionId, SessionInfo, SessionState};

/// Manages factory session lifecycle and persistence.
///
/// Sessions are persisted as JSON files in `~/.cas/sessions/`.
/// Each session has a unique ID and can be paused, resumed, or archived.
pub struct SessionManager {
    /// Directory where sessions are stored
    sessions_dir: PathBuf,
    /// In-memory cache of session info
    sessions: HashMap<SessionId, SessionInfo>,
    /// Currently active session (only one can be active)
    active_session_id: Option<SessionId>,
}

impl SessionManager {
    /// Create a new SessionManager.
    ///
    /// Sessions are stored in `~/.cas/sessions/` by default,
    /// or a custom path can be provided.
    pub fn new(sessions_dir: Option<PathBuf>) -> Result<Self> {
        let sessions_dir = sessions_dir.unwrap_or_else(default_sessions_dir);

        // Ensure directory exists
        fs::create_dir_all(&sessions_dir)?;

        let mut manager = Self {
            sessions_dir,
            sessions: HashMap::new(),
            active_session_id: None,
        };

        // Load existing sessions
        manager.load_sessions()?;

        Ok(manager)
    }

    /// Create a new factory session.
    ///
    /// # Arguments
    /// * `name` - Human-readable name for the session
    /// * `project_dir` - Project directory for this session
    ///
    /// # Returns
    /// The session ID on success.
    pub fn create_session(
        &mut self,
        name: impl Into<String>,
        project_dir: impl AsRef<Path>,
    ) -> Result<SessionId> {
        let session = SessionInfo::new(name, project_dir.as_ref().to_path_buf());
        let session_id = session.id.clone();

        // Pause any currently active session
        if let Some(active_id) = self.active_session_id.take()
            && let Some(active) = self.sessions.get_mut(&active_id)
        {
            active.state = SessionState::Paused;
            active.paused_at = Some(Utc::now());
            save_session_to_dir(&self.sessions_dir, active)?;
        }

        // Save and track new session
        save_session_to_dir(&self.sessions_dir, &session)?;
        self.sessions.insert(session_id.clone(), session);
        self.active_session_id = Some(session_id.clone());

        Ok(session_id)
    }

    /// Pause an active session.
    ///
    /// Paused sessions retain their state and can be resumed later.
    pub fn pause_session(&mut self, id: &str) -> Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;

        if session.state != SessionState::Active {
            return Err(SessionError::InvalidTransition(
                "pause".to_string(),
                session.state,
            ));
        }

        session.touch(); // Accumulate active time
        session.state = SessionState::Paused;
        session.paused_at = Some(Utc::now());
        save_session_to_dir(&self.sessions_dir, session)?;

        if self.active_session_id.as_deref() == Some(id) {
            self.active_session_id = None;
        }

        Ok(())
    }

    /// Resume a paused session.
    ///
    /// This makes the session active again. Any currently active session
    /// will be paused automatically.
    pub fn resume_session(&mut self, id: &str) -> Result<()> {
        // Verify the session exists and is paused
        {
            let session = self
                .sessions
                .get(id)
                .ok_or_else(|| SessionError::NotFound(id.to_string()))?;

            if session.state != SessionState::Paused {
                return Err(SessionError::InvalidTransition(
                    "resume".to_string(),
                    session.state,
                ));
            }
        }

        // Pause any currently active session first
        if let Some(active_id) = self.active_session_id.take()
            && active_id != id
            && let Some(active) = self.sessions.get_mut(&active_id)
        {
            active.touch();
            active.state = SessionState::Paused;
            active.paused_at = Some(Utc::now());
            save_session_to_dir(&self.sessions_dir, active)?;
        }

        // Now resume the target session
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;
        session.state = SessionState::Active;
        session.paused_at = None;
        session.last_active_at = Utc::now();
        save_session_to_dir(&self.sessions_dir, session)?;

        self.active_session_id = Some(id.to_string());

        Ok(())
    }

    /// Archive a session.
    ///
    /// Archived sessions are read-only and cannot be resumed.
    /// They are kept for historical reference.
    pub fn archive_session(&mut self, id: &str) -> Result<()> {
        let session = self
            .sessions
            .get_mut(id)
            .ok_or_else(|| SessionError::NotFound(id.to_string()))?;

        if session.state == SessionState::Archived {
            return Err(SessionError::InvalidTransition(
                "archive".to_string(),
                session.state,
            ));
        }

        session.touch(); // Accumulate any remaining active time
        session.state = SessionState::Archived;
        session.archived_at = Some(Utc::now());
        save_session_to_dir(&self.sessions_dir, session)?;

        if self.active_session_id.as_deref() == Some(id) {
            self.active_session_id = None;
        }

        Ok(())
    }

    /// List all sessions, optionally filtered by state.
    ///
    /// Sessions are returned sorted by last_active_at (most recent first).
    pub fn list_sessions(&self, state: Option<SessionState>) -> Vec<SessionInfo> {
        let mut sessions: Vec<_> = self
            .sessions
            .values()
            .filter(|s| state.is_none() || Some(s.state) == state)
            .cloned()
            .collect();

        sessions.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
        sessions
    }

    /// Get a session by ID.
    pub fn get_session(&self, id: &str) -> Option<&SessionInfo> {
        self.sessions.get(id)
    }

    /// Get a mutable reference to a session.
    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut SessionInfo> {
        self.sessions.get_mut(id)
    }

    /// Get the currently active session.
    pub fn active_session(&self) -> Option<&SessionInfo> {
        self.active_session_id
            .as_ref()
            .and_then(|id| self.sessions.get(id))
    }

    /// Get the currently active session ID.
    pub fn active_session_id(&self) -> Option<&str> {
        self.active_session_id.as_deref()
    }

    /// Update and save a session.
    ///
    /// Call this after modifying session metadata to persist changes.
    pub fn update_session(&mut self, session: &SessionInfo) -> Result<()> {
        save_session_to_dir(&self.sessions_dir, session)?;
        self.sessions.insert(session.id.clone(), session.clone());
        Ok(())
    }

    /// Delete a session permanently.
    ///
    /// This removes both the in-memory entry and the persisted file.
    pub fn delete_session(&mut self, id: &str) -> Result<()> {
        if !self.sessions.contains_key(id) {
            return Err(SessionError::NotFound(id.to_string()));
        }

        // Remove file
        let path = session_path(&self.sessions_dir, id);
        if path.exists() {
            fs::remove_file(&path)?;
        }

        // Remove from memory
        self.sessions.remove(id);
        if self.active_session_id.as_deref() == Some(id) {
            self.active_session_id = None;
        }

        Ok(())
    }

    /// Touch the active session to update last_active_at.
    pub fn touch_active(&mut self) {
        if let Some(id) = self.active_session_id.clone()
            && let Some(session) = self.sessions.get_mut(&id)
        {
            session.touch();
        }
    }

    /// Save the active session to disk.
    ///
    /// Call this periodically to persist accumulated active time.
    pub fn save_active(&mut self) -> Result<()> {
        if let Some(id) = self.active_session_id.clone()
            && let Some(session) = self.sessions.get(&id)
        {
            save_session_to_dir(&self.sessions_dir, session)?;
        }
        Ok(())
    }

    /// Get the sessions directory path.
    pub fn sessions_dir(&self) -> &Path {
        &self.sessions_dir
    }

    // === Private methods ===

    /// Load all sessions from disk
    fn load_sessions(&mut self) -> Result<()> {
        let entries = fs::read_dir(&self.sessions_dir)?;

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                match load_session_file(&path) {
                    Ok(session) => {
                        if session.state == SessionState::Active {
                            self.active_session_id = Some(session.id.clone());
                        }
                        self.sessions.insert(session.id.clone(), session);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load session from {:?}: {}", path, e);
                    }
                }
            }
        }

        Ok(())
    }
}

fn load_session_file(path: &Path) -> Result<SessionInfo> {
    let content = fs::read_to_string(path)?;
    let session: SessionInfo = serde_json::from_str(&content)?;
    Ok(session)
}

/// Save a session to a directory
fn save_session_to_dir(sessions_dir: &Path, session: &SessionInfo) -> Result<()> {
    let path = session_path(sessions_dir, &session.id);
    let content = serde_json::to_string_pretty(session)?;
    fs::write(&path, content)?;
    Ok(())
}

/// Get the file path for a session
fn session_path(sessions_dir: &Path, id: &str) -> PathBuf {
    sessions_dir.join(format!("{id}.json"))
}

fn default_sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cas")
        .join("sessions")
}
