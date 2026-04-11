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
/// If the folder name cannot be derived (e.g. `.cas/` lives at the filesystem root
/// and its parent has no file name), falls back to a deterministic `local:<sha256>`
/// hash of the canonicalized project path. This guarantees every valid CAS project
/// has a stable, unique `project_id` for cloud sync scoping.
///
/// Returns `None` only if not inside a CAS project directory at all.
/// The result is cached for the lifetime of the process.
pub fn get_project_canonical_id() -> Option<String> {
    CACHED_PROJECT_ID
        .get_or_init(|| {
            let cas_root = find_cas_root().ok()?;
            resolve_canonical_id(&cas_root)
        })
        .clone()
}

/// Pure composition of the folder-name derivation and the path-hash fallback.
/// Extracted from `get_project_canonical_id` so the `.or_else` chain is testable
/// without the `OnceLock` static — callers should prefer the cached public API.
pub fn resolve_canonical_id(cas_root: &Path) -> Option<String> {
    canonical_id_from_cas_root(cas_root).or_else(|| fallback_project_id_from_path(cas_root))
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

/// Fallback project ID derived from a deterministic sha256 hash of the canonical
/// project path. Used when `canonical_id_from_cas_root` cannot produce a folder
/// name (e.g. `.cas/` at the filesystem root).
///
/// Format: `local:<first 16 hex chars of sha256(canonical_path)>` — 8 bytes of
/// entropy, more than enough to avoid collisions on a single machine while staying
/// compact in URLs and logs.
///
/// The input is the parent of `cas_root` (the project directory), canonicalized
/// via `std::fs::canonicalize` when possible so symlinked and renamed paths
/// produce the same ID. Falls back to the lexical path if canonicalization fails
/// (e.g. the directory no longer exists on disk — should not happen in practice
/// since we just resolved it via `find_cas_root`, but we stay defensive).
///
/// Returns `None` only if both the canonical and lexical paths fail to produce
/// any bytes to hash — practically unreachable.
pub fn fallback_project_id_from_path(cas_root: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};

    let project_dir = cas_root.parent().unwrap_or(cas_root);
    let canonical = std::fs::canonicalize(project_dir).unwrap_or_else(|_| project_dir.to_path_buf());
    let path_bytes = canonical.as_os_str().as_encoded_bytes();
    if path_bytes.is_empty() {
        return None;
    }

    let mut hasher = Sha256::new();
    hasher.update(path_bytes);
    let digest = hasher.finalize();
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Some(format!("local:{hex}"))
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
    fn test_fallback_project_id_from_path_is_deterministic() {
        // Same input path produces the same hash across repeated invocations,
        // and the format is `local:` + 16 lowercase-hex chars (8 bytes of sha256).
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("some-project");
        let cas_dir = project_dir.join(".cas");
        std::fs::create_dir_all(&cas_dir).unwrap();

        let first = fallback_project_id_from_path(&cas_dir).unwrap();
        let second = fallback_project_id_from_path(&cas_dir).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("local:"));
        // local: + 16 hex chars = 22 chars total
        assert_eq!(first.len(), 22);
        // Every char after the `local:` prefix must be a lowercase ASCII hex digit.
        let suffix = &first[6..];
        assert!(
            suffix.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "fallback suffix should be lowercase hex, got {suffix:?}"
        );
    }

    #[test]
    fn test_fallback_project_id_from_path_is_unique_per_path() {
        // Different project paths must produce different hashes — otherwise two
        // projects at different locations would still collide.
        let temp = TempDir::new().unwrap();

        let project_a = temp.path().join("project-a");
        let cas_a = project_a.join(".cas");
        std::fs::create_dir_all(&cas_a).unwrap();

        let project_b = temp.path().join("project-b");
        let cas_b = project_b.join(".cas");
        std::fs::create_dir_all(&cas_b).unwrap();

        let id_a = fallback_project_id_from_path(&cas_a).unwrap();
        let id_b = fallback_project_id_from_path(&cas_b).unwrap();
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn test_fallback_project_id_handles_filesystem_root() {
        // The whole point of the fallback: at filesystem root,
        // canonical_id_from_cas_root returns None; fallback must still produce a value.
        use std::path::Path;
        let root_cas = Path::new("/.cas");
        assert_eq!(canonical_id_from_cas_root(root_cas), None);

        let fallback = fallback_project_id_from_path(root_cas);
        assert!(fallback.is_some());
        let id = fallback.unwrap();
        assert!(id.starts_with("local:"));
        assert_eq!(id.len(), 22);
    }

    #[test]
    fn test_resolve_canonical_id_prefers_folder_name() {
        // End-to-end coverage of the .or_else chain: when the folder name is
        // available, resolve_canonical_id returns it unchanged — the fallback
        // must not fire on the happy path.
        let temp = TempDir::new().unwrap();
        let project_dir = temp.path().join("my-project");
        let cas_dir = project_dir.join(".cas");
        std::fs::create_dir_all(&cas_dir).unwrap();

        let id = resolve_canonical_id(&cas_dir).unwrap();
        assert_eq!(id, "my-project");
        assert!(!id.starts_with("local:"));
    }

    #[test]
    fn test_resolve_canonical_id_falls_back_at_filesystem_root() {
        // End-to-end: when folder name is unavailable (filesystem root),
        // resolve_canonical_id returns Some("local:...") instead of None.
        // A regression that dropped the `.or_else` would turn this back into None.
        use std::path::Path;
        let root_cas = Path::new("/.cas");
        let id = resolve_canonical_id(root_cas).expect("fallback should fire at fs root");
        assert!(id.starts_with("local:"));
        assert_eq!(id.len(), 22);
    }

    #[test]
    fn test_fallback_lexical_branch_when_canonicalize_fails() {
        // `fallback_project_id_from_path` falls back to the lexical path when
        // `std::fs::canonicalize` fails (e.g., the directory does not exist on
        // disk). Point it at a non-existent path and verify we still get a
        // stable `local:<hex>` value rather than a panic or None.
        let temp = TempDir::new().unwrap();
        let nonexistent_cas = temp.path().join("never-created").join(".cas");
        // Intentionally do NOT create the directory.

        let id = fallback_project_id_from_path(&nonexistent_cas)
            .expect("fallback must tolerate non-canonicalizable paths");
        assert!(id.starts_with("local:"));
        assert_eq!(id.len(), 22);

        // Deterministic: same non-existent path produces the same hash.
        let id2 = fallback_project_id_from_path(&nonexistent_cas).unwrap();
        assert_eq!(id, id2);
    }

    #[cfg(unix)]
    #[test]
    fn test_fallback_resolves_symlinks_to_same_id() {
        // Documented contract: "symlinked and renamed paths produce the same ID"
        // via `std::fs::canonicalize`. Create a real project, symlink to it,
        // and assert both paths produce the same fallback hash.
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        let real_project = temp.path().join("real-project");
        let real_cas = real_project.join(".cas");
        std::fs::create_dir_all(&real_cas).unwrap();

        let link_project = temp.path().join("link-to-project");
        symlink(&real_project, &link_project).unwrap();
        let link_cas = link_project.join(".cas");

        let id_real = fallback_project_id_from_path(&real_cas).unwrap();
        let id_link = fallback_project_id_from_path(&link_cas).unwrap();
        assert_eq!(
            id_real, id_link,
            "symlinked and real paths should hash to the same ID after canonicalization"
        );
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
