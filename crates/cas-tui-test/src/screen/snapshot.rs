//! Snapshot serialization and comparison
//!
//! Provides YAML-based snapshot capture and diffing for deterministic testing.

use crate::screen::buffer::{CursorPos, ScreenBuffer, TermSize};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::path::Path;

/// A captured snapshot of the screen state
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Snapshot {
    /// Snapshot name/identifier
    pub name: String,
    /// Terminal size at capture
    pub size: TermSize,
    /// Text content (rows of strings, trailing whitespace trimmed)
    pub content: Vec<String>,
    /// Cursor position at capture
    pub cursor: CursorPos,
    /// Whether cursor was visible
    pub cursor_visible: bool,
    /// Metadata about the snapshot
    #[serde(default)]
    pub metadata: SnapshotMetadata,
}

/// Metadata about the snapshot
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotMetadata {
    /// Creation timestamp (ISO8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    /// Framework version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub framework_version: Option<String>,
}

impl Snapshot {
    /// Create a snapshot from a screen buffer
    pub fn from_buffer(name: impl Into<String>, buffer: &ScreenBuffer) -> Self {
        Self {
            name: name.into(),
            size: buffer.size(),
            content: buffer.text_lines(),
            cursor: buffer.cursor(),
            cursor_visible: buffer.cursor_visible(),
            metadata: SnapshotMetadata::default(),
        }
    }

    /// Create a snapshot with metadata
    pub fn from_buffer_with_metadata(
        name: impl Into<String>,
        buffer: &ScreenBuffer,
        metadata: SnapshotMetadata,
    ) -> Self {
        Self {
            name: name.into(),
            size: buffer.size(),
            content: buffer.text_lines(),
            cursor: buffer.cursor(),
            cursor_visible: buffer.cursor_visible(),
            metadata,
        }
    }

    /// Serialize to YAML string
    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    /// Deserialize from YAML string
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    /// Save snapshot to file
    pub fn save(&self, path: impl AsRef<Path>) -> std::io::Result<()> {
        let yaml = self
            .to_yaml()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        std::fs::write(path, yaml)
    }

    /// Load snapshot from file
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let yaml = std::fs::read_to_string(path)?;
        Self::from_yaml(&yaml)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
    }

    /// Compare with another snapshot and return diff if different
    pub fn diff(&self, other: &Snapshot) -> Option<SnapshotDiff> {
        if self == other {
            return None;
        }

        let mut differences = Vec::new();

        if self.size != other.size {
            differences.push(DiffItem::SizeMismatch {
                expected: self.size,
                actual: other.size,
            });
        }

        if self.cursor != other.cursor {
            differences.push(DiffItem::CursorMismatch {
                expected: self.cursor,
                actual: other.cursor,
            });
        }

        // Compare content line by line
        let max_lines = self.content.len().max(other.content.len());
        for i in 0..max_lines {
            let expected = self.content.get(i).map(|s| s.as_str()).unwrap_or("");
            let actual = other.content.get(i).map(|s| s.as_str()).unwrap_or("");
            if expected != actual {
                differences.push(DiffItem::LineMismatch {
                    line: i,
                    expected: expected.to_string(),
                    actual: actual.to_string(),
                });
            }
        }

        Some(SnapshotDiff {
            name: self.name.clone(),
            differences,
        })
    }

    /// Get the text content as a single string
    pub fn text(&self) -> String {
        self.content.join("\n")
    }

    /// Check if content contains text
    pub fn contains_text(&self, needle: &str) -> bool {
        self.text().contains(needle)
    }
}

/// A diff between two snapshots
#[derive(Clone, Debug)]
pub struct SnapshotDiff {
    /// Name of the snapshot
    pub name: String,
    /// List of differences
    pub differences: Vec<DiffItem>,
}

impl SnapshotDiff {
    /// Check if there are any differences
    pub fn is_empty(&self) -> bool {
        self.differences.is_empty()
    }

    /// Get the number of differences
    pub fn len(&self) -> usize {
        self.differences.len()
    }
}

impl fmt::Display for SnapshotDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Snapshot '{}' differs:", self.name)?;
        for diff in &self.differences {
            writeln!(f, "  {diff}")?;
        }
        Ok(())
    }
}

/// A single difference between snapshots
#[derive(Clone, Debug)]
pub enum DiffItem {
    SizeMismatch {
        expected: TermSize,
        actual: TermSize,
    },
    CursorMismatch {
        expected: CursorPos,
        actual: CursorPos,
    },
    LineMismatch {
        line: usize,
        expected: String,
        actual: String,
    },
}

impl fmt::Display for DiffItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffItem::SizeMismatch { expected, actual } => {
                write!(
                    f,
                    "Size: expected {}x{}, got {}x{}",
                    expected.cols, expected.rows, actual.cols, actual.rows
                )
            }
            DiffItem::CursorMismatch { expected, actual } => {
                write!(
                    f,
                    "Cursor: expected ({}, {}), got ({}, {})",
                    expected.row, expected.col, actual.row, actual.col
                )
            }
            DiffItem::LineMismatch {
                line,
                expected,
                actual,
            } => {
                write!(f, "Line {line}: expected {expected:?}, got {actual:?}")
            }
        }
    }
}

/// Frame captured for debugging/artifacts
#[derive(Clone, Debug)]
pub struct Frame {
    /// The captured screen state
    pub buffer: ScreenBuffer,
    /// Raw PTY output that produced this frame
    pub raw_output: Vec<u8>,
}

impl Frame {
    /// Create a frame from a buffer and raw output
    pub fn new(buffer: ScreenBuffer, raw_output: Vec<u8>) -> Self {
        Self { buffer, raw_output }
    }

    /// Create a snapshot from this frame
    pub fn to_snapshot(&self, name: impl Into<String>) -> Snapshot {
        Snapshot::from_buffer(name, &self.buffer)
    }
}

/// Rolling buffer of recent frames for debugging
#[derive(Debug)]
pub struct FrameHistory {
    frames: VecDeque<Frame>,
    max_frames: usize,
}

impl FrameHistory {
    /// Create a new frame history with max capacity
    pub fn new(max_frames: usize) -> Self {
        Self {
            frames: VecDeque::with_capacity(max_frames),
            max_frames,
        }
    }

    /// Add a frame to the history
    pub fn push(&mut self, frame: Frame) {
        if self.frames.len() >= self.max_frames {
            self.frames.pop_front();
        }
        self.frames.push_back(frame);
    }

    /// Get all frames in order
    pub fn frames(&self) -> Vec<&Frame> {
        self.frames.iter().collect()
    }

    /// Iterate frames in order without allocation
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, Frame> {
        self.frames.iter()
    }

    /// Get all frames in order (alias for frames())
    pub fn frames_vec(&self) -> Vec<&Frame> {
        self.frames.iter().collect()
    }

    /// Get the latest frame
    pub fn latest(&self) -> Option<&Frame> {
        self.frames.back()
    }

    /// Get the number of frames
    pub fn len(&self) -> usize {
        self.frames.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Clear all frames
    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

impl Default for FrameHistory {
    fn default() -> Self {
        Self::new(10)
    }
}

#[cfg(test)]
mod tests {
    use crate::screen::snapshot::*;

    fn make_buffer_with_text(text: &str) -> ScreenBuffer {
        let mut buffer = ScreenBuffer::new(80, 24);
        buffer.put_str(text);
        buffer
    }

    #[test]
    fn test_snapshot_creation() {
        let buffer = make_buffer_with_text("Hello World");
        let snapshot = Snapshot::from_buffer("test", &buffer);

        assert_eq!(snapshot.name, "test");
        assert_eq!(snapshot.size, TermSize::new(80, 24));
        assert_eq!(snapshot.content[0], "Hello World");
    }

    #[test]
    fn test_snapshot_yaml_roundtrip() {
        let buffer = make_buffer_with_text("Test content");
        let snapshot = Snapshot::from_buffer("yaml_test", &buffer);

        let yaml = snapshot.to_yaml().unwrap();
        let loaded = Snapshot::from_yaml(&yaml).unwrap();

        assert_eq!(snapshot, loaded);
    }

    #[test]
    fn test_snapshot_diff_equal() {
        let buffer = make_buffer_with_text("Same content");
        let snap1 = Snapshot::from_buffer("test", &buffer);
        let snap2 = Snapshot::from_buffer("test", &buffer);

        assert!(snap1.diff(&snap2).is_none());
    }

    #[test]
    fn test_snapshot_diff_content() {
        let buffer1 = make_buffer_with_text("Hello");
        let buffer2 = make_buffer_with_text("World");
        let snap1 = Snapshot::from_buffer("test", &buffer1);
        let snap2 = Snapshot::from_buffer("test", &buffer2);

        let diff = snap1.diff(&snap2).unwrap();
        assert!(!diff.is_empty());
        assert!(
            diff.differences
                .iter()
                .any(|d| matches!(d, DiffItem::LineMismatch { .. }))
        );
    }

    #[test]
    fn test_snapshot_diff_cursor() {
        let mut buffer1 = ScreenBuffer::new(80, 24);
        buffer1.move_cursor_to(0, 0);
        let mut buffer2 = ScreenBuffer::new(80, 24);
        buffer2.move_cursor_to(5, 10);

        let snap1 = Snapshot::from_buffer("test", &buffer1);
        let snap2 = Snapshot::from_buffer("test", &buffer2);

        let diff = snap1.diff(&snap2).unwrap();
        assert!(
            diff.differences
                .iter()
                .any(|d| matches!(d, DiffItem::CursorMismatch { .. }))
        );
    }

    #[test]
    fn test_frame_history() {
        let mut history = FrameHistory::new(3);

        for i in 0..5 {
            let buffer = make_buffer_with_text(&format!("Frame {i}"));
            history.push(Frame::new(buffer, vec![]));
        }

        // Should only keep last 3
        assert_eq!(history.len(), 3);
        let latest = history.latest().unwrap();
        assert!(latest.buffer.contains_text("Frame 4"));
    }

    #[test]
    fn test_snapshot_contains_text() {
        let buffer = make_buffer_with_text("Welcome to the application");
        let snapshot = Snapshot::from_buffer("test", &buffer);

        assert!(snapshot.contains_text("Welcome"));
        assert!(snapshot.contains_text("application"));
        assert!(!snapshot.contains_text("Goodbye"));
    }
}
