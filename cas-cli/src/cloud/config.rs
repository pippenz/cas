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
use std::process::Command;
use std::sync::OnceLock;

use crate::error::CasError;
use crate::store::find_cas_root;

/// Cached project canonical ID. The git remote doesn't change during a process lifetime.
static CACHED_PROJECT_ID: OnceLock<Option<String>> = OnceLock::new();

/// Get the canonical project ID from the current git repository.
///
/// This normalizes git remote URLs to a canonical format:
/// - `git@github.com:owner/repo.git` → `github.com/owner/repo`
/// - `https://github.com/owner/repo.git` → `github.com/owner/repo`
/// - `ssh://git@gitlab.com/team/project.git` → `gitlab.com/team/project`
///
/// Returns None if not in a git repository or no remote is configured.
/// The result is cached for the lifetime of the process.
pub fn get_project_canonical_id() -> Option<String> {
    CACHED_PROJECT_ID
        .get_or_init(|| {
            let output = Command::new("git")
                .args(["remote", "get-url", "origin"])
                .output()
                .ok()?;

            if !output.status.success() {
                return None;
            }

            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if url.is_empty() {
                return None;
            }

            Some(normalize_git_remote(&url))
        })
        .clone()
}

/// Normalize a git remote URL to a canonical format.
///
/// Examples:
/// - `git@github.com:company/repo.git` → `github.com/company/repo`
/// - `https://github.com/company/repo.git` → `github.com/company/repo`
/// - `ssh://git@gitlab.com/team/project.git` → `gitlab.com/team/project`
pub fn normalize_git_remote(url: &str) -> String {
    let mut result = url.trim().to_string();

    // Remove protocol (https://, ssh://, git://)
    if let Some(pos) = result.find("://") {
        result = result[pos + 3..].to_string();
    }

    // Remove git@ prefix
    if result.starts_with("git@") {
        result = result[4..].to_string();
    }

    // Convert : to / for SSH URLs (git@github.com:owner/repo)
    // But don't convert port numbers (github.com:443/owner/repo)
    if let Some(pos) = result.find(':') {
        let after_colon = &result[pos + 1..];
        // If what follows the colon starts with a digit, it's a port
        if !after_colon
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit())
        {
            result = format!("{}/{}", &result[..pos], after_colon);
        }
    }

    // Remove .git suffix
    if result.ends_with(".git") {
        result = result[..result.len() - 4].to_string();
    }

    // Remove trailing slashes
    while result.ends_with('/') {
        result.pop();
    }

    // Remove any userinfo (user:pass@)
    if let Some(at_pos) = result.find('@') {
        if let Some(slash_pos) = result.find('/') {
            if at_pos < slash_pos {
                result = result[at_pos + 1..].to_string();
            }
        }
    }

    // Lowercase for consistency
    result.to_lowercase()
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
