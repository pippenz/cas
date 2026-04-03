//! Cloud configuration management
//!
//! Stores cloud authentication and sync state in `.cas/cloud.json`.
//!
//! # Integration Status
//! Methods ready for cloud sync feature when enabled.

// #![allow(dead_code)] // Check unused

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::error::CasError;
use crate::store::find_cas_root;

/// Cached project canonical ID. The folder name doesn't change during a process lifetime.
static CACHED_PROJECT_ID: OnceLock<Option<String>> = OnceLock::new();

/// Get the canonical project ID for the current CAS project.
///
/// The canonical ID is the folder name of the project root directory (the directory
/// containing `.cas/`). This is:
/// - Stable across git remote changes (fork, transfer, rename)
/// - Works for non-git projects
/// - Human-readable in logs, UI, and team project lists
///
/// Examples:
/// - `/home/user/projects/petra-stella-cloud/.cas/` → `petra-stella-cloud`
/// - `/home/user/cas-src/.cas/` → `cas-src`
/// - `/home/user/gabber-studio/.cas/` → `gabber-studio`
///
/// Returns `None` only if not inside a CAS project directory.
/// The result is cached for the lifetime of the process.
pub fn get_project_canonical_id() -> Option<String> {
    CACHED_PROJECT_ID
        .get_or_init(|| {
            let cas_root = find_cas_root().ok()?;
            canonical_id_from_cas_root(&cas_root)
        })
        .clone()
}

/// Derive the canonical project ID from a `.cas` directory path.
///
/// The canonical ID is the folder name of the parent directory (the project root).
/// Returns `None` if the path has no parent or no file name (e.g. filesystem root).
pub fn canonical_id_from_cas_root(cas_root: &Path) -> Option<String> {
    let project_dir = cas_root.parent().unwrap_or(cas_root);
    project_dir
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
}

/// Cloud configuration stored in .cas/cloud.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudConfig {
    /// Cloud API endpoint
    #[serde(default = "default_endpoint")]
    pub endpoint: String,

    /// API token for authentication
    pub token: Option<String>,

    /// User email
    pub email: Option<String>,

    /// User plan
    pub plan: Option<String>,

    /// Organization ID (for enterprise users)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,

    /// Organization slug (for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub org_slug: Option<String>,

    /// Team ID (for enterprise users)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,

    /// Team slug (for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub team_slug: Option<String>,

    /// Per-team sync timestamps (team_id -> last sync time)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub team_sync_timestamps: HashMap<String, DateTime<Utc>>,

    /// Per-project team memory sync timestamps (canonical_id -> last pull time)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub team_memory_sync_timestamps: HashMap<String, String>,

    /// Last sync timestamp for entries
    pub last_entry_sync: Option<String>,

    /// Last sync timestamp for tasks
    pub last_task_sync: Option<String>,

    /// Last sync timestamp for rules
    pub last_rule_sync: Option<String>,

    /// Last sync timestamp for skills
    pub last_skill_sync: Option<String>,
}

fn default_endpoint() -> String {
    "https://cas.dev".to_string()
}

impl Default for CloudConfig {
    fn default() -> Self {
        Self {
            endpoint: default_endpoint(),
            token: None,
            email: None,
            plan: None,
            org_id: None,
            org_slug: None,
            team_id: None,
            team_slug: None,
            team_sync_timestamps: HashMap::new(),
            team_memory_sync_timestamps: HashMap::new(),
            last_entry_sync: None,
            last_task_sync: None,
            last_rule_sync: None,
            last_skill_sync: None,
        }
    }
}

impl CloudConfig {
    /// Load cloud config from .cas/cloud.json
    pub fn load() -> Result<Self, CasError> {
        let path = Self::config_path()?;
        Self::load_from(&path)
    }

    /// Load cloud config from a specific path
    pub fn load_from(path: &Path) -> Result<Self, CasError> {
        if path.exists() {
            let content = fs::read_to_string(path)?;
            let config: Self = serde_json::from_str(&content)
                .map_err(|e| CasError::Other(format!("Failed to parse cloud config: {e}")))?;
            Ok(config)
        } else {
            Ok(Self::default())
        }
    }

    /// Load cloud config from a specific cas directory
    pub fn load_from_cas_dir(cas_dir: &Path) -> Result<Self, CasError> {
        let path = cas_dir.join("cloud.json");
        Self::load_from(&path)
    }

    /// Save cloud config to .cas/cloud.json
    pub fn save(&self) -> Result<(), CasError> {
        let path = Self::config_path()?;
        self.save_to(&path)
    }

    /// Save cloud config to a specific path
    pub fn save_to(&self, path: &Path) -> Result<(), CasError> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| CasError::Other(format!("Failed to serialize cloud config: {e}")))?;
        fs::write(path, content)?;
        Ok(())
    }

    /// Save cloud config to a specific cas directory
    pub fn save_to_cas_dir(&self, cas_dir: &Path) -> Result<(), CasError> {
        let path = cas_dir.join("cloud.json");
        self.save_to(&path)
    }

    /// Get the path to cloud.json
    pub fn config_path() -> Result<PathBuf, CasError> {
        let cas_root = find_cas_root()?;
        Ok(cas_root.join("cloud.json"))
    }

    /// Check if user is logged in (has a valid token)
    pub fn is_logged_in(&self) -> bool {
        self.token.as_ref().is_some_and(|t| !t.is_empty())
    }

    /// Clear authentication (logout)
    pub fn logout(&mut self) {
        self.token = None;
        self.email = None;
        self.plan = None;
        self.org_id = None;
        self.org_slug = None;
        self.team_id = None;
        self.team_slug = None;
    }

    /// Check if user belongs to an organization
    pub fn has_org(&self) -> bool {
        self.org_id.is_some()
    }

    /// Check if user belongs to a team
    pub fn has_team(&self) -> bool {
        self.team_id.is_some()
    }

    /// Set the current team context
    pub fn set_team(&mut self, team_id: &str, team_slug: &str) {
        self.team_id = Some(team_id.to_string());
        self.team_slug = Some(team_slug.to_string());
    }

    /// Clear the current team context
    pub fn clear_team(&mut self) {
        self.team_id = None;
        self.team_slug = None;
    }

    /// Get the last sync timestamp for a specific team
    pub fn get_team_sync_timestamp(&self, team_id: &str) -> Option<DateTime<Utc>> {
        self.team_sync_timestamps.get(team_id).copied()
    }

    /// Set the last sync timestamp for a specific team
    pub fn set_team_sync_timestamp(&mut self, team_id: &str, ts: DateTime<Utc>) {
        self.team_sync_timestamps.insert(team_id.to_string(), ts);
    }

    /// Clear the sync timestamp for a specific team
    pub fn clear_team_sync_timestamp(&mut self, team_id: &str) {
        self.team_sync_timestamps.remove(team_id);
    }

    /// Get the last team memory sync timestamp for a project
    pub fn get_team_memory_sync(&self, canonical_id: &str) -> Option<&str> {
        self.team_memory_sync_timestamps
            .get(canonical_id)
            .map(|s| s.as_str())
    }

    /// Set the last team memory sync timestamp for a project
    pub fn set_team_memory_sync(&mut self, canonical_id: &str, timestamp: &str) {
        self.team_memory_sync_timestamps
            .insert(canonical_id.to_string(), timestamp.to_string());
    }
}

#[cfg(test)]
mod tests {
    use crate::cloud::config::*;
    use tempfile::TempDir;

    #[test]
    fn test_default_config() {
        let config = CloudConfig::default();
        assert_eq!(config.endpoint, "https://cas.dev");
        assert!(config.token.is_none());
        assert!(!config.is_logged_in());
    }

    #[test]
    fn test_save_and_load() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("cloud.json");

        let config = CloudConfig {
            token: Some("test_token".to_string()),
            email: Some("test@example.com".to_string()),
            ..Default::default()
        };

        config.save_to(&path).unwrap();

        let loaded = CloudConfig::load_from(&path).unwrap();
        assert_eq!(loaded.token, Some("test_token".to_string()));
        assert_eq!(loaded.email, Some("test@example.com".to_string()));
        assert!(loaded.is_logged_in());
    }

    #[test]
    fn test_logout() {
        let mut config = CloudConfig {
            token: Some("test_token".to_string()),
            email: Some("test@example.com".to_string()),
            ..Default::default()
        };

        assert!(config.is_logged_in());

        config.logout();

        assert!(!config.is_logged_in());
        assert!(config.token.is_none());
        assert!(config.email.is_none());
    }

    #[test]
    fn test_set_and_clear_team() {
        let mut config = CloudConfig::default();
        assert!(!config.has_team());
        assert!(config.team_id.is_none());
        assert!(config.team_slug.is_none());

        config.set_team("team-123", "my-team");
        assert!(config.has_team());
        assert_eq!(config.team_id, Some("team-123".to_string()));
        assert_eq!(config.team_slug, Some("my-team".to_string()));

        config.clear_team();
        assert!(!config.has_team());
        assert!(config.team_id.is_none());
        assert!(config.team_slug.is_none());
    }

    #[test]
    fn test_team_sync_timestamps() {
        let mut config = CloudConfig::default();

        // Initially no timestamps
        assert!(config.get_team_sync_timestamp("team-a").is_none());

        // Set timestamp for team-a
        let ts1 = Utc::now();
        config.set_team_sync_timestamp("team-a", ts1);
        assert_eq!(config.get_team_sync_timestamp("team-a"), Some(ts1));

        // Set timestamp for team-b
        let ts2 = Utc::now();
        config.set_team_sync_timestamp("team-b", ts2);
        assert_eq!(config.get_team_sync_timestamp("team-b"), Some(ts2));

        // team-a still has its timestamp
        assert_eq!(config.get_team_sync_timestamp("team-a"), Some(ts1));

        // Clear team-a timestamp
        config.clear_team_sync_timestamp("team-a");
        assert!(config.get_team_sync_timestamp("team-a").is_none());
        assert_eq!(config.get_team_sync_timestamp("team-b"), Some(ts2));
    }

    #[test]
    fn test_team_memory_sync_timestamps() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("cloud.json");

        let mut config = CloudConfig {
            token: Some("t".to_string()),
            ..Default::default()
        };

        // Initially no timestamp
        assert!(config.get_team_memory_sync("github.com/foo/bar").is_none());

        // Set and get
        config.set_team_memory_sync("github.com/foo/bar", "2026-04-02T10:00:00Z");
        assert_eq!(
            config.get_team_memory_sync("github.com/foo/bar"),
            Some("2026-04-02T10:00:00Z")
        );

        // Persists through save/load
        config.save_to(&path).unwrap();
        let loaded = CloudConfig::load_from(&path).unwrap();
        assert_eq!(
            loaded.get_team_memory_sync("github.com/foo/bar"),
            Some("2026-04-02T10:00:00Z")
        );
    }

    #[test]
    fn test_canonical_id_from_cas_root() {
        // Create real temp directories simulating different project layouts
        let temp = TempDir::new().unwrap();

        // Simulate /tmp/.../petra-stella-cloud/.cas
        let project_a = temp.path().join("petra-stella-cloud");
        let cas_a = project_a.join(".cas");
        std::fs::create_dir_all(&cas_a).unwrap();
        assert_eq!(
            canonical_id_from_cas_root(&cas_a),
            Some("petra-stella-cloud".to_string())
        );

        // Simulate /tmp/.../gabber-studio/.cas
        let project_b = temp.path().join("gabber-studio");
        let cas_b = project_b.join(".cas");
        std::fs::create_dir_all(&cas_b).unwrap();
        assert_eq!(
            canonical_id_from_cas_root(&cas_b),
            Some("gabber-studio".to_string())
        );

        // Non-git project works the same way
        let project_c = temp.path().join("local-only-project");
        let cas_c = project_c.join(".cas");
        std::fs::create_dir_all(&cas_c).unwrap();
        assert_eq!(
            canonical_id_from_cas_root(&cas_c),
            Some("local-only-project".to_string())
        );

        // Folder with spaces
        let project_d = temp.path().join("Richards LLC");
        let cas_d = project_d.join(".cas");
        std::fs::create_dir_all(&cas_d).unwrap();
        assert_eq!(
            canonical_id_from_cas_root(&cas_d),
            Some("Richards LLC".to_string())
        );
    }

    #[test]
    fn test_canonical_id_from_filesystem_root() {
        // Edge case: .cas at filesystem root — parent is "/" which has no file_name
        use std::path::Path;
        let root_cas = Path::new("/.cas");
        assert_eq!(canonical_id_from_cas_root(root_cas), None);
    }

    #[test]
    fn test_team_sync_timestamps_persist() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("cloud.json");

        let mut config = CloudConfig {
            token: Some("test_token".to_string()),
            ..Default::default()
        };
        config.set_team("team-123", "my-team");
        let ts = Utc::now();
        config.set_team_sync_timestamp("team-123", ts);

        config.save_to(&path).unwrap();

        let loaded = CloudConfig::load_from(&path).unwrap();
        assert_eq!(loaded.team_id, Some("team-123".to_string()));
        assert_eq!(loaded.team_slug, Some("my-team".to_string()));
        // Timestamps are stored with second precision in JSON
        let loaded_ts = loaded.get_team_sync_timestamp("team-123").unwrap();
        assert!((loaded_ts - ts).num_seconds().abs() < 1);
    }
}
