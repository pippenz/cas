//! Snapshot storage and management
//!
//! Manages snapshot files with stable directory layout and update mode support.
//!
//! # Directory Layout
//!
//! ```text
//! tests/snapshots/           # Default snapshot directory
//!   my_test/
//!     initial.snap.yaml      # Named snapshots
//!     after_login.snap.yaml
//!   other_test/
//!     main.snap.yaml
//! ```
//!
//! # Environment Variables
//!
//! - `TUI_TEST_SNAPSHOT_DIR` - Override default snapshot directory
//! - `TUI_TEST_UPDATE_SNAPSHOTS` - Set to "1" to update snapshots instead of comparing

use crate::screen::{ScreenBuffer, Snapshot, SnapshotDiff, SnapshotMetadata};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Snapshot store configuration
#[derive(Clone, Debug)]
pub struct SnapshotStoreConfig {
    /// Base directory for snapshots
    pub snapshot_dir: PathBuf,
    /// Whether to update snapshots instead of comparing
    pub update_mode: bool,
}

impl Default for SnapshotStoreConfig {
    fn default() -> Self {
        Self {
            snapshot_dir: PathBuf::from("tests/snapshots"),
            update_mode: false,
        }
    }
}

impl SnapshotStoreConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Self {
        let snapshot_dir = std::env::var("TUI_TEST_SNAPSHOT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("tests/snapshots"));

        let update_mode = std::env::var("TUI_TEST_UPDATE_SNAPSHOTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            snapshot_dir,
            update_mode,
        }
    }

    /// Set snapshot directory
    pub fn with_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.snapshot_dir = dir.into();
        self
    }

    /// Enable update mode
    pub fn with_update_mode(mut self, update: bool) -> Self {
        self.update_mode = update;
        self
    }
}

/// Manages snapshot storage and comparison
#[derive(Debug)]
pub struct SnapshotStore {
    config: SnapshotStoreConfig,
    /// Test name for directory organization
    test_name: String,
}

impl SnapshotStore {
    /// Create a new snapshot store for a test
    pub fn new(test_name: impl Into<String>) -> Self {
        Self::with_config(test_name, SnapshotStoreConfig::from_env())
    }

    /// Create with custom configuration
    pub fn with_config(test_name: impl Into<String>, config: SnapshotStoreConfig) -> Self {
        Self {
            config,
            test_name: test_name.into(),
        }
    }

    /// Get the directory for this test's snapshots
    pub fn test_dir(&self) -> PathBuf {
        self.config.snapshot_dir.join(&self.test_name)
    }

    /// Get the path for a named snapshot
    pub fn snapshot_path(&self, name: &str) -> PathBuf {
        self.test_dir().join(format!("{name}.snap.yaml"))
    }

    /// Check if update mode is enabled
    pub fn is_update_mode(&self) -> bool {
        self.config.update_mode
    }

    /// Assert that a buffer matches a stored snapshot
    ///
    /// If the snapshot doesn't exist or update mode is enabled, saves the current state.
    /// Otherwise, compares and returns an error with diff on mismatch.
    pub fn assert_snapshot(
        &self,
        name: &str,
        buffer: &ScreenBuffer,
    ) -> Result<SnapshotResult, SnapshotError> {
        let path = self.snapshot_path(name);
        let current = Snapshot::from_buffer_with_metadata(
            name,
            buffer,
            SnapshotMetadata {
                created_at: None,
                framework_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            },
        );

        if self.config.update_mode || !path.exists() {
            // Create or update snapshot
            self.save_snapshot(&path, &current)?;
            return Ok(if path.exists() && !self.config.update_mode {
                SnapshotResult::Created
            } else {
                SnapshotResult::Updated
            });
        }

        // Load and compare
        let expected = self.load_snapshot(&path)?;

        // Compare ignoring metadata (created_at, version)
        if snapshots_match(&expected, &current) {
            Ok(SnapshotResult::Matched)
        } else {
            let diff = expected.diff(&current);
            Err(SnapshotError::Mismatch {
                name: name.to_string(),
                path,
                diff,
            })
        }
    }

    /// Save a snapshot to the store
    fn save_snapshot(&self, path: &Path, snapshot: &Snapshot) -> Result<(), SnapshotError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(SnapshotError::Io)?;
        }
        snapshot.save(path).map_err(SnapshotError::Io)
    }

    /// Load a snapshot from the store
    fn load_snapshot(&self, path: &Path) -> Result<Snapshot, SnapshotError> {
        Snapshot::load(path).map_err(SnapshotError::Io)
    }

    /// List all snapshots for this test
    pub fn list_snapshots(&self) -> io::Result<Vec<String>> {
        let dir = self.test_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut snapshots = Vec::new();
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("yaml") {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    if let Some(name) = stem.strip_suffix(".snap") {
                        snapshots.push(name.to_string());
                    }
                }
            }
        }
        snapshots.sort();
        Ok(snapshots)
    }

    /// Delete a snapshot
    pub fn delete_snapshot(&self, name: &str) -> io::Result<bool> {
        let path = self.snapshot_path(name);
        if path.exists() {
            fs::remove_file(path)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Clean all snapshots for this test
    pub fn clean(&self) -> io::Result<usize> {
        let dir = self.test_dir();
        if !dir.exists() {
            return Ok(0);
        }

        let snapshots = self.list_snapshots()?;
        let count = snapshots.len();
        fs::remove_dir_all(dir)?;
        Ok(count)
    }
}

/// Compare snapshots ignoring metadata differences
fn snapshots_match(a: &Snapshot, b: &Snapshot) -> bool {
    a.size == b.size
        && a.content == b.content
        && a.cursor == b.cursor
        && a.cursor_visible == b.cursor_visible
}

/// Result of a snapshot assertion
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotResult {
    /// Snapshot matched expected
    Matched,
    /// New snapshot was created
    Created,
    /// Existing snapshot was updated
    Updated,
}

/// Errors from snapshot operations
#[derive(Debug)]
pub enum SnapshotError {
    /// IO error
    Io(io::Error),
    /// Snapshot mismatch
    Mismatch {
        name: String,
        path: PathBuf,
        diff: Option<SnapshotDiff>,
    },
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SnapshotError::Io(e) => write!(f, "Snapshot IO error: {e}"),
            SnapshotError::Mismatch { name, path, diff } => {
                writeln!(f, "Snapshot '{name}' mismatch")?;
                writeln!(f, "  File: {}", path.display())?;
                if let Some(d) = diff {
                    write!(f, "{d}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SnapshotError::Io(e) => Some(e),
            _ => None,
        }
    }
}

/// Macro for inline snapshot assertions
///
/// # Example
///
/// ```ignore
/// assert_snapshot!(store, "login_screen", buffer);
/// ```
#[macro_export]
macro_rules! assert_snapshot {
    ($store:expr, $name:expr, $buffer:expr) => {
        match $store.assert_snapshot($name, $buffer) {
            Ok(_) => {}
            Err(e) => panic!("{}", e),
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::artifact::store::*;
    use tempfile::TempDir;

    fn make_buffer_with_text(text: &str) -> ScreenBuffer {
        let mut buffer = ScreenBuffer::new(40, 10);
        buffer.put_str(text);
        buffer
    }

    #[test]
    fn test_snapshot_path() {
        let store = SnapshotStore::with_config(
            "my_test",
            SnapshotStoreConfig::default().with_dir("snapshots"),
        );
        let path = store.snapshot_path("initial");
        assert_eq!(path, PathBuf::from("snapshots/my_test/initial.snap.yaml"));
    }

    #[test]
    fn test_create_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_create", config);

        let buffer = make_buffer_with_text("Hello World");
        let result = store.assert_snapshot("main", &buffer).unwrap();

        assert_eq!(result, SnapshotResult::Created);
        assert!(store.snapshot_path("main").exists());
    }

    #[test]
    fn test_match_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_match", config);

        let buffer = make_buffer_with_text("Test content");

        // Create snapshot
        store.assert_snapshot("main", &buffer).unwrap();

        // Match snapshot
        let result = store.assert_snapshot("main", &buffer).unwrap();
        assert_eq!(result, SnapshotResult::Matched);
    }

    #[test]
    fn test_mismatch_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_mismatch", config);

        let buffer1 = make_buffer_with_text("Original");
        let buffer2 = make_buffer_with_text("Changed");

        // Create with original
        store.assert_snapshot("main", &buffer1).unwrap();

        // Should fail with changed
        let err = store.assert_snapshot("main", &buffer2).unwrap_err();
        assert!(matches!(err, SnapshotError::Mismatch { .. }));
    }

    #[test]
    fn test_update_mode() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default()
            .with_dir(temp_dir.path())
            .with_update_mode(true);
        let store = SnapshotStore::with_config("test_update", config);

        let buffer1 = make_buffer_with_text("Original");
        let buffer2 = make_buffer_with_text("Updated");

        // Create snapshot
        store.assert_snapshot("main", &buffer1).unwrap();

        // Update should succeed even with different content
        let result = store.assert_snapshot("main", &buffer2).unwrap();
        assert_eq!(result, SnapshotResult::Updated);

        // Verify the snapshot was updated
        let snapshot = Snapshot::load(store.snapshot_path("main")).unwrap();
        assert!(snapshot.contains_text("Updated"));
    }

    #[test]
    fn test_list_snapshots() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_list", config);

        let buffer = make_buffer_with_text("Content");
        store.assert_snapshot("first", &buffer).unwrap();
        store.assert_snapshot("second", &buffer).unwrap();
        store.assert_snapshot("third", &buffer).unwrap();

        let list = store.list_snapshots().unwrap();
        assert_eq!(list.len(), 3);
        assert!(list.contains(&"first".to_string()));
        assert!(list.contains(&"second".to_string()));
        assert!(list.contains(&"third".to_string()));
    }

    #[test]
    fn test_delete_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_delete", config);

        let buffer = make_buffer_with_text("Content");
        store.assert_snapshot("to_delete", &buffer).unwrap();

        assert!(store.delete_snapshot("to_delete").unwrap());
        assert!(!store.snapshot_path("to_delete").exists());
        assert!(!store.delete_snapshot("to_delete").unwrap()); // Already deleted
    }

    #[test]
    fn test_clean() {
        let temp_dir = TempDir::new().unwrap();
        let config = SnapshotStoreConfig::default().with_dir(temp_dir.path());
        let store = SnapshotStore::with_config("test_clean", config);

        let buffer = make_buffer_with_text("Content");
        store.assert_snapshot("one", &buffer).unwrap();
        store.assert_snapshot("two", &buffer).unwrap();

        let count = store.clean().unwrap();
        assert_eq!(count, 2);
        assert!(!store.test_dir().exists());
    }
}
