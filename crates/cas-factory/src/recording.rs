//! Recording integration for factory sessions.
//!
//! This module provides a synchronous interface to the async cas-recording crate,
//! allowing FactoryCore to record PTY output without requiring async methods.

use std::collections::HashMap;
use std::path::PathBuf;

use cas_recording::{RecordingWriter, WriterConfig, WriterStats};
use tokio::runtime::Handle;
use tracing::{debug, error, warn};

/// Manages recording writers for all panes in a factory session.
///
/// Provides a synchronous API that internally uses tokio for async I/O.
/// Each pane (supervisor and workers) gets its own recording file.
pub struct RecordingManager {
    /// Recording writers keyed by pane/agent name
    writers: HashMap<String, RecordingWriter>,
    /// Session ID for this recording session
    session_id: String,
    /// Tokio runtime handle for async operations
    handle: Handle,
    /// Base configuration for writers
    config: WriterConfig,
    /// Terminal dimensions
    cols: u16,
    rows: u16,
}

impl RecordingManager {
    /// Create a new RecordingManager for a factory session.
    ///
    /// # Arguments
    /// * `session_id` - Unique identifier for this recording session
    /// * `cols` - Terminal width in columns
    /// * `rows` - Terminal height in rows
    /// * `recordings_dir` - Optional custom recordings directory
    ///
    /// # Panics
    /// Panics if called outside a tokio runtime context.
    pub fn new(
        session_id: impl Into<String>,
        cols: u16,
        rows: u16,
        recordings_dir: Option<PathBuf>,
    ) -> Self {
        let handle = Handle::current();
        let session_id = session_id.into();

        let mut config = WriterConfig::default();
        if let Some(dir) = recordings_dir {
            config.recordings_dir = dir;
        }

        debug!(
            "Created RecordingManager for session {} at {:?}",
            session_id, config.recordings_dir
        );

        Self {
            writers: HashMap::new(),
            session_id,
            handle,
            config,
            cols,
            rows,
        }
    }

    /// Start recording for a pane/agent.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent (e.g., "supervisor", "swift-fox")
    /// * `agent_role` - Role: "supervisor", "worker", or "primary"
    ///
    /// # Returns
    /// `true` if recording started successfully, `false` on error.
    pub fn start_recording(&mut self, agent_name: &str, agent_role: &str) -> bool {
        if self.writers.contains_key(agent_name) {
            warn!("Recording already active for agent: {}", agent_name);
            return false;
        }

        let session_id = self.session_id.clone();
        let agent_name_owned = agent_name.to_string();
        let agent_role_owned = agent_role.to_string();
        let cols = self.cols;
        let rows = self.rows;
        let config = self.config.clone();

        // Use block_in_place to allow blocking within async context
        let result = tokio::task::block_in_place(|| {
            self.handle.block_on(async {
                RecordingWriter::new(
                    cols,
                    rows,
                    &agent_name_owned,
                    &session_id,
                    &agent_role_owned,
                    config,
                )
                .await
            })
        });

        match result {
            Ok(writer) => {
                debug!(
                    "Started recording for {} ({}) in session {}",
                    agent_name, agent_role, self.session_id
                );
                self.writers.insert(agent_name.to_string(), writer);
                true
            }
            Err(e) => {
                error!("Failed to start recording for {}: {}", agent_name, e);
                false
            }
        }
    }

    /// Write PTY output to a pane's recording.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent
    /// * `data` - Raw PTY output bytes
    pub fn write_output(&self, agent_name: &str, data: &[u8]) {
        if let Some(writer) = self.writers.get(agent_name) {
            let result = tokio::task::block_in_place(|| {
                self.handle
                    .block_on(async { writer.write_output(data).await })
            });

            if let Err(e) = result {
                error!("Failed to write recording output for {}: {}", agent_name, e);
            }
        }
    }

    /// Write a resize event to a pane's recording.
    ///
    /// # Arguments
    /// * `agent_name` - Name of the agent
    /// * `cols` - New terminal width
    /// * `rows` - New terminal height
    pub fn write_resize(&mut self, agent_name: &str, cols: u16, rows: u16) {
        // Update internal dimensions
        self.cols = cols;
        self.rows = rows;

        if let Some(writer) = self.writers.get_mut(agent_name) {
            let result = tokio::task::block_in_place(|| {
                self.handle
                    .block_on(async { writer.write_resize(cols, rows).await })
            });

            if let Err(e) = result {
                error!("Failed to write recording resize for {}: {}", agent_name, e);
            }
        }
    }

    /// Stop recording for a specific pane/agent.
    ///
    /// # Returns
    /// Recording statistics if the agent was being recorded.
    pub fn stop_recording(&mut self, agent_name: &str) -> Option<WriterStats> {
        if let Some(writer) = self.writers.remove(agent_name) {
            let result = tokio::task::block_in_place(|| {
                self.handle.block_on(async { writer.close().await })
            });

            match result {
                Ok(stats) => {
                    debug!(
                        "Stopped recording for {}: {} events, {}ms duration",
                        agent_name, stats.total_events, stats.total_duration_ms
                    );
                    Some(stats)
                }
                Err(e) => {
                    error!("Failed to close recording for {}: {}", agent_name, e);
                    None
                }
            }
        } else {
            None
        }
    }

    /// Stop all recordings and return statistics.
    ///
    /// # Returns
    /// Map of agent names to their recording statistics.
    pub fn stop_all(&mut self) -> HashMap<String, WriterStats> {
        let mut stats = HashMap::new();
        let agent_names: Vec<_> = self.writers.keys().cloned().collect();

        for agent_name in agent_names {
            if let Some(s) = self.stop_recording(&agent_name) {
                stats.insert(agent_name, s);
            }
        }

        stats
    }

    /// Check if recording is active for an agent.
    pub fn is_recording(&self, agent_name: &str) -> bool {
        self.writers.contains_key(agent_name)
    }

    /// Get the session ID.
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Get the recordings directory.
    pub fn recordings_dir(&self) -> &PathBuf {
        &self.config.recordings_dir
    }

    /// Get the path to a specific agent's recording file.
    pub fn recording_path(&self, agent_name: &str) -> PathBuf {
        self.config
            .recordings_dir
            .join(&self.session_id)
            .join(format!("{agent_name}.rec"))
    }

    /// Get the number of active recordings.
    pub fn active_count(&self) -> usize {
        self.writers.len()
    }
}

impl Drop for RecordingManager {
    fn drop(&mut self) {
        if !self.writers.is_empty() {
            debug!(
                "RecordingManager dropped with {} active recordings, closing...",
                self.writers.len()
            );
            let _ = self.stop_all();
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::recording::*;

    // Note: These tests require a multi-threaded tokio runtime for block_in_place

    #[tokio::test(flavor = "multi_thread")]
    async fn test_recording_manager_new() {
        let dir = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        assert_eq!(manager.session_id(), "test-session");
        assert_eq!(manager.active_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_start_recording() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        let result = manager.start_recording("agent-1", "worker");
        assert!(result);
        assert!(manager.is_recording("agent-1"));
        assert_eq!(manager.active_count(), 1);

        // Cleanup
        manager.stop_all();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_write_output() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        manager.start_recording("agent-1", "worker");
        manager.write_output("agent-1", b"Hello, World!\n");

        let stats = manager.stop_recording("agent-1").unwrap();
        assert_eq!(stats.total_events, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_write_resize() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        manager.start_recording("agent-1", "worker");
        manager.write_resize("agent-1", 120, 40);

        let stats = manager.stop_recording("agent-1").unwrap();
        assert_eq!(stats.total_events, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_all() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        manager.start_recording("supervisor", "supervisor");
        manager.start_recording("worker-1", "worker");
        manager.start_recording("worker-2", "worker");

        assert_eq!(manager.active_count(), 3);

        let stats = manager.stop_all();
        assert_eq!(stats.len(), 3);
        assert_eq!(manager.active_count(), 0);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_recording_path() {
        let dir = tempfile::tempdir().unwrap();
        let manager = RecordingManager::new("sess-123", 80, 24, Some(dir.path().to_path_buf()));

        let path = manager.recording_path("swift-fox");
        assert!(path.ends_with("sess-123/swift-fox.rec"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_duplicate_recording_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut manager =
            RecordingManager::new("test-session", 80, 24, Some(dir.path().to_path_buf()));

        assert!(manager.start_recording("agent-1", "worker"));
        assert!(!manager.start_recording("agent-1", "worker")); // Should return false

        manager.stop_all();
    }
}
