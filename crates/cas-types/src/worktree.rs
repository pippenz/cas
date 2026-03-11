//! Git worktree types for CAS
//!
//! Provides types for tracking git worktrees associated with epics.
//! Worktrees are scoped to epics (not individual tasks), allowing multiple
//! related tasks within an epic to share a single development environment.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::str::FromStr;

/// Status of a worktree in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeStatus {
    /// Worktree is active and usable
    #[default]
    Active,
    /// Branch was merged back to parent
    Merged,
    /// Task closed without merge (work discarded)
    Abandoned,
    /// Worktree directory has been removed
    Removed,
    /// Merge conflict detected, needs manual resolution
    Conflict,
}

impl std::fmt::Display for WorktreeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorktreeStatus::Active => write!(f, "active"),
            WorktreeStatus::Merged => write!(f, "merged"),
            WorktreeStatus::Abandoned => write!(f, "abandoned"),
            WorktreeStatus::Removed => write!(f, "removed"),
            WorktreeStatus::Conflict => write!(f, "conflict"),
        }
    }
}

impl FromStr for WorktreeStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "active" => Ok(WorktreeStatus::Active),
            "merged" => Ok(WorktreeStatus::Merged),
            "abandoned" => Ok(WorktreeStatus::Abandoned),
            "removed" => Ok(WorktreeStatus::Removed),
            "conflict" => Ok(WorktreeStatus::Conflict),
            _ => Err(format!("Unknown worktree status: {s}")),
        }
    }
}

/// A git worktree managed by CAS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Worktree {
    /// Unique identifier (wt-{short_hash})
    pub id: String,

    /// Epic that owns this worktree
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub epic_id: Option<String>,

    /// Git branch name (e.g., "cas/cas-1234")
    pub branch: String,

    /// Branch the worktree was created from (e.g., "main")
    pub parent_branch: String,

    /// Absolute path to worktree directory
    pub path: PathBuf,

    /// Current status in lifecycle
    pub status: WorktreeStatus,

    /// When the worktree was created
    pub created_at: DateTime<Utc>,

    /// When the branch was merged back (if merged)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_at: Option<DateTime<Utc>>,

    /// When the worktree directory was removed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub removed_at: Option<DateTime<Utc>>,

    /// Agent that created this worktree
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_by_agent: Option<String>,

    /// Commit hash after merge (for audit)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_commit: Option<String>,
}

impl Worktree {
    /// Create a new worktree
    pub fn new(id: String, branch: String, parent_branch: String, path: PathBuf) -> Self {
        Self {
            id,
            epic_id: None,
            branch,
            parent_branch,
            path,
            status: WorktreeStatus::Active,
            created_at: Utc::now(),
            merged_at: None,
            removed_at: None,
            created_by_agent: None,
            merge_commit: None,
        }
    }

    /// Create a worktree for a specific epic
    ///
    /// Multiple tasks within the same epic share this worktree.
    pub fn for_epic(
        id: String,
        epic_id: String,
        branch: String,
        parent_branch: String,
        path: PathBuf,
        agent_id: Option<String>,
    ) -> Self {
        Self {
            id,
            epic_id: Some(epic_id),
            branch,
            parent_branch,
            path,
            status: WorktreeStatus::Active,
            created_at: Utc::now(),
            merged_at: None,
            removed_at: None,
            created_by_agent: agent_id,
            merge_commit: None,
        }
    }

    /// Check if the worktree is still active
    pub fn is_active(&self) -> bool {
        self.status == WorktreeStatus::Active
    }

    /// Check if the worktree can be worked on
    pub fn is_usable(&self) -> bool {
        matches!(
            self.status,
            WorktreeStatus::Active | WorktreeStatus::Conflict
        )
    }

    /// Mark as merged
    pub fn mark_merged(&mut self, commit: Option<String>) {
        self.status = WorktreeStatus::Merged;
        self.merged_at = Some(Utc::now());
        self.merge_commit = commit;
    }

    /// Mark as abandoned
    pub fn mark_abandoned(&mut self) {
        self.status = WorktreeStatus::Abandoned;
    }

    /// Mark as removed
    pub fn mark_removed(&mut self) {
        self.status = WorktreeStatus::Removed;
        self.removed_at = Some(Utc::now());
    }

    /// Mark as having a merge conflict
    pub fn mark_conflict(&mut self) {
        self.status = WorktreeStatus::Conflict;
    }

    /// Generate a worktree ID
    pub fn generate_id() -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        use std::sync::atomic::{AtomicU64, Ordering};

        static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

        let mut hasher = DefaultHasher::new();
        Utc::now().timestamp_nanos_opt().hash(&mut hasher);
        std::process::id().hash(&mut hasher);
        ID_COUNTER.fetch_add(1, Ordering::Relaxed).hash(&mut hasher);

        let hash = hasher.finish();
        format!("wt-{:08x}", hash as u32)
    }
}

/// Information about the current git context
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GitContext {
    /// Current git branch name
    pub branch: Option<String>,

    /// Whether we're in a git worktree (vs main checkout)
    pub is_worktree: bool,

    /// Path to the worktree root (if in a worktree)
    pub worktree_path: Option<PathBuf>,

    /// Path to the main git directory
    pub git_dir: Option<PathBuf>,

    /// CAS worktree record ID (if this worktree is managed by CAS)
    pub cas_worktree_id: Option<String>,
}

impl GitContext {
    /// Check if we have valid git context
    pub fn is_valid(&self) -> bool {
        self.branch.is_some()
    }
}

#[cfg(test)]
mod tests {
    use crate::worktree::*;

    #[test]
    fn test_worktree_status_roundtrip() {
        for status in [
            WorktreeStatus::Active,
            WorktreeStatus::Merged,
            WorktreeStatus::Abandoned,
            WorktreeStatus::Removed,
            WorktreeStatus::Conflict,
        ] {
            let s = status.to_string();
            let parsed: WorktreeStatus = s.parse().unwrap();
            assert_eq!(status, parsed);
        }
    }

    #[test]
    fn test_worktree_new() {
        let wt = Worktree::new(
            "wt-12345678".to_string(),
            "cas/cas-1234".to_string(),
            "main".to_string(),
            PathBuf::from("/tmp/worktree"),
        );

        assert_eq!(wt.status, WorktreeStatus::Active);
        assert!(wt.epic_id.is_none());
        assert!(wt.is_active());
    }

    #[test]
    fn test_worktree_for_epic() {
        let wt = Worktree::for_epic(
            "wt-12345678".to_string(),
            "cas-epic-1234".to_string(),
            "cas/cas-epic-1234".to_string(),
            "main".to_string(),
            PathBuf::from("/tmp/worktree"),
            Some("agent-123".to_string()),
        );

        assert_eq!(wt.epic_id, Some("cas-epic-1234".to_string()));
        assert_eq!(wt.created_by_agent, Some("agent-123".to_string()));
    }

    #[test]
    fn test_worktree_lifecycle() {
        let mut wt = Worktree::new(
            "wt-12345678".to_string(),
            "cas/cas-1234".to_string(),
            "main".to_string(),
            PathBuf::from("/tmp/worktree"),
        );

        assert!(wt.is_active());
        assert!(wt.is_usable());

        wt.mark_merged(Some("abc123".to_string()));
        assert!(!wt.is_active());
        assert!(!wt.is_usable());
        assert!(wt.merged_at.is_some());
        assert_eq!(wt.merge_commit, Some("abc123".to_string()));

        wt.mark_removed();
        assert_eq!(wt.status, WorktreeStatus::Removed);
        assert!(wt.removed_at.is_some());
    }

    #[test]
    fn test_generate_id() {
        let id1 = Worktree::generate_id();
        let id2 = Worktree::generate_id();

        assert!(id1.starts_with("wt-"));
        assert!(id2.starts_with("wt-"));
        // IDs should be unique (with very high probability)
        assert_ne!(id1, id2);
    }
}
