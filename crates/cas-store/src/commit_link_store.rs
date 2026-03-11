//! SQLite-based commit link storage for tracking git commits from AI sessions
//!
//! This module provides storage for commit links that enable code attribution:
//! tracing git commits back to the session, agent, and prompts that created them.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::Result;
use crate::error::StoreError;
use cas_types::{CommitLink, Scope};

/// Helper to convert mutex poison error to StoreError
fn lock_error<T>(_: std::sync::PoisonError<T>) -> StoreError {
    StoreError::Other("lock poisoned".to_string())
}

/// Schema for commit_links table (also defined in migration m143)
pub const COMMIT_LINK_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS commit_links (
    commit_hash TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    branch TEXT NOT NULL,
    message TEXT NOT NULL,
    files_changed TEXT NOT NULL,
    prompt_ids TEXT NOT NULL,
    committed_at TEXT NOT NULL,
    author TEXT NOT NULL,
    scope TEXT NOT NULL DEFAULT 'project'
);

CREATE INDEX IF NOT EXISTS idx_commit_links_session ON commit_links(session_id);
CREATE INDEX IF NOT EXISTS idx_commit_links_branch ON commit_links(branch);
CREATE INDEX IF NOT EXISTS idx_commit_links_committed ON commit_links(committed_at DESC);
"#;

/// Trait for commit link storage operations
pub trait CommitLinkStore: Send + Sync {
    /// Initialize the store (create tables)
    fn init(&self) -> Result<()>;

    /// Add a new commit link
    fn add(&self, link: &CommitLink) -> Result<()>;

    /// Get a commit link by hash
    fn get(&self, commit_hash: &str) -> Result<Option<CommitLink>>;

    /// Get commit links for a session
    fn list_by_session(&self, session_id: &str, limit: usize) -> Result<Vec<CommitLink>>;

    /// Get commit links for a branch
    fn list_by_branch(&self, branch: &str, limit: usize) -> Result<Vec<CommitLink>>;

    /// Get recent commits (most recent first)
    fn list_recent(&self, limit: usize) -> Result<Vec<CommitLink>>;

    /// Find commits that changed a specific file
    fn find_by_file(&self, file_path: &str, limit: usize) -> Result<Vec<CommitLink>>;

    /// Delete old commit links (keep last N days)
    fn prune(&self, days: i64) -> Result<usize>;

    /// Close the store
    fn close(&self) -> Result<()>;
}

/// SQLite implementation of CommitLinkStore
pub struct SqliteCommitLinkStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteCommitLinkStore {
    /// Open or create a commit link store
    pub fn open(cas_dir: &Path) -> Result<Self> {
        let db_path = cas_dir.join("cas.db");
        let conn = crate::shared_db::shared_connection(&db_path)?;

        let store = Self { conn };
        store.init()?;
        Ok(store)
    }

    /// Create from an existing connection (for use within other stores)
    pub fn from_connection(conn: Connection) -> Result<Self> {
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        store.init()?;
        Ok(store)
    }

    /// Parse a row into a CommitLink
    fn row_to_commit_link(row: &rusqlite::Row) -> rusqlite::Result<CommitLink> {
        let committed_at_str: String = row.get("committed_at")?;
        let scope_str: String = row.get("scope")?;
        let files_changed_json: String = row.get("files_changed")?;
        let prompt_ids_json: String = row.get("prompt_ids")?;

        // Parse JSON arrays
        let files_changed: Vec<String> =
            serde_json::from_str(&files_changed_json).unwrap_or_default();
        let prompt_ids: Vec<String> = serde_json::from_str(&prompt_ids_json).unwrap_or_default();

        Ok(CommitLink {
            commit_hash: row.get("commit_hash")?,
            session_id: row.get("session_id")?,
            agent_id: row.get("agent_id")?,
            branch: row.get("branch")?,
            message: row.get("message")?,
            files_changed,
            prompt_ids,
            committed_at: parse_datetime(&committed_at_str),
            author: row.get("author")?,
            scope: scope_str.parse().unwrap_or(Scope::Project),
        })
    }
}

impl CommitLinkStore for SqliteCommitLinkStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().map_err(lock_error)?;
        conn.execute_batch(COMMIT_LINK_SCHEMA)?;
        Ok(())
    }

    fn add(&self, link: &CommitLink) -> Result<()> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let files_changed_json = serde_json::to_string(&link.files_changed)
            .map_err(|e| StoreError::Other(format!("Failed to serialize files_changed: {e}")))?;
        let prompt_ids_json = serde_json::to_string(&link.prompt_ids)
            .map_err(|e| StoreError::Other(format!("Failed to serialize prompt_ids: {e}")))?;

        conn.execute(
            "INSERT OR REPLACE INTO commit_links
             (commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                link.commit_hash,
                link.session_id,
                link.agent_id,
                link.branch,
                link.message,
                files_changed_json,
                prompt_ids_json,
                link.committed_at.to_rfc3339(),
                link.author,
                link.scope.to_string(),
            ],
        )?;

        Ok(())
    }

    fn get(&self, commit_hash: &str) -> Result<Option<CommitLink>> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let mut stmt = conn.prepare(
            "SELECT commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope
             FROM commit_links
             WHERE commit_hash = ?1",
        )?;

        let link = stmt
            .query_row(params![commit_hash], Self::row_to_commit_link)
            .optional()?;

        Ok(link)
    }

    fn list_by_session(&self, session_id: &str, limit: usize) -> Result<Vec<CommitLink>> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let mut stmt = conn.prepare(
            "SELECT commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope
             FROM commit_links
             WHERE session_id = ?1
             ORDER BY committed_at DESC
             LIMIT ?2",
        )?;

        let links = stmt
            .query_map(params![session_id, limit as i64], Self::row_to_commit_link)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    fn list_by_branch(&self, branch: &str, limit: usize) -> Result<Vec<CommitLink>> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let mut stmt = conn.prepare(
            "SELECT commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope
             FROM commit_links
             WHERE branch = ?1
             ORDER BY committed_at DESC
             LIMIT ?2",
        )?;

        let links = stmt
            .query_map(params![branch, limit as i64], Self::row_to_commit_link)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    fn list_recent(&self, limit: usize) -> Result<Vec<CommitLink>> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let mut stmt = conn.prepare(
            "SELECT commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope
             FROM commit_links
             ORDER BY committed_at DESC
             LIMIT ?1",
        )?;

        let links = stmt
            .query_map(params![limit as i64], Self::row_to_commit_link)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    fn find_by_file(&self, file_path: &str, limit: usize) -> Result<Vec<CommitLink>> {
        // Search for file path in the JSON array
        // Using LIKE with the JSON string is a simple approach
        let conn = self.conn.lock().map_err(lock_error)?;

        let pattern = format!("%\"{file_path}%");
        let mut stmt = conn.prepare(
            "SELECT commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope
             FROM commit_links
             WHERE files_changed LIKE ?1
             ORDER BY committed_at DESC
             LIMIT ?2",
        )?;

        let links = stmt
            .query_map(params![pattern, limit as i64], Self::row_to_commit_link)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(links)
    }

    fn prune(&self, days: i64) -> Result<usize> {
        let conn = self.conn.lock().map_err(lock_error)?;

        let cutoff = Utc::now() - chrono::Duration::days(days);

        let deleted = conn.execute(
            "DELETE FROM commit_links WHERE committed_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;

        Ok(deleted)
    }

    fn close(&self) -> Result<()> {
        // Connection will be closed when dropped
        Ok(())
    }
}

/// Parse a datetime string, with fallback to current time
fn parse_datetime(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.with_timezone(&Utc))
        .or_else(|_| {
            // Try ISO format without timezone
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
                .map(|dt| Utc.from_utc_datetime(&dt))
        })
        .unwrap_or_else(|_| Utc::now())
}

/// Helper to add a commit link using the connection from another store
pub fn add_commit_link_with_conn(
    conn: &Connection,
    link: &CommitLink,
) -> std::result::Result<(), rusqlite::Error> {
    let files_changed_json =
        serde_json::to_string(&link.files_changed).unwrap_or_else(|_| "[]".to_string());
    let prompt_ids_json =
        serde_json::to_string(&link.prompt_ids).unwrap_or_else(|_| "[]".to_string());

    conn.execute(
        "INSERT OR REPLACE INTO commit_links
         (commit_hash, session_id, agent_id, branch, message, files_changed, prompt_ids, committed_at, author, scope)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            link.commit_hash,
            link.session_id,
            link.agent_id,
            link.branch,
            link.message,
            files_changed_json,
            prompt_ids_json,
            link.committed_at.to_rfc3339(),
            link.author,
            link.scope.to_string(),
        ],
    )?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::commit_link_store::*;
    use tempfile::TempDir;

    fn setup_store() -> (SqliteCommitLinkStore, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let store = SqliteCommitLinkStore::open(temp_dir.path()).unwrap();
        (store, temp_dir)
    }

    fn create_test_link(commit_hash: &str, session_id: &str, branch: &str) -> CommitLink {
        CommitLink::new(
            commit_hash.to_string(),
            session_id.to_string(),
            "agent-1".to_string(),
            branch.to_string(),
            "Test commit message".to_string(),
            vec!["src/main.rs".to_string(), "src/lib.rs".to_string()],
            vec!["prompt-1".to_string()],
            "Test Author <test@example.com>".to_string(),
        )
    }

    #[test]
    fn test_add_and_get() {
        let (store, _dir) = setup_store();

        let link = create_test_link("abc123def456", "session-1", "main");
        store.add(&link).unwrap();

        let retrieved = store.get("abc123def456").unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.commit_hash, "abc123def456");
        assert_eq!(retrieved.branch, "main");
        assert_eq!(retrieved.files_changed.len(), 2);
        assert_eq!(retrieved.prompt_ids.len(), 1);
    }

    #[test]
    fn test_list_by_session() {
        let (store, _dir) = setup_store();

        store
            .add(&create_test_link("commit-1", "session-A", "main"))
            .unwrap();
        store
            .add(&create_test_link("commit-2", "session-A", "main"))
            .unwrap();
        store
            .add(&create_test_link("commit-3", "session-B", "main"))
            .unwrap();

        let session_a = store.list_by_session("session-A", 10).unwrap();
        assert_eq!(session_a.len(), 2);

        let session_b = store.list_by_session("session-B", 10).unwrap();
        assert_eq!(session_b.len(), 1);
    }

    #[test]
    fn test_list_by_branch() {
        let (store, _dir) = setup_store();

        store
            .add(&create_test_link("commit-1", "session-1", "main"))
            .unwrap();
        store
            .add(&create_test_link("commit-2", "session-1", "feature"))
            .unwrap();
        store
            .add(&create_test_link("commit-3", "session-1", "main"))
            .unwrap();

        let main_commits = store.list_by_branch("main", 10).unwrap();
        assert_eq!(main_commits.len(), 2);

        let feature_commits = store.list_by_branch("feature", 10).unwrap();
        assert_eq!(feature_commits.len(), 1);
    }

    #[test]
    fn test_list_recent() {
        let (store, _dir) = setup_store();

        for i in 0..5 {
            store
                .add(&create_test_link(
                    &format!("commit-{i}"),
                    "session-1",
                    "main",
                ))
                .unwrap();
        }

        let recent = store.list_recent(3).unwrap();
        assert_eq!(recent.len(), 3);
    }

    #[test]
    fn test_find_by_file() {
        let (store, _dir) = setup_store();

        let mut link1 = create_test_link("commit-1", "session-1", "main");
        link1.files_changed = vec!["src/main.rs".to_string()];

        let mut link2 = create_test_link("commit-2", "session-1", "main");
        link2.files_changed = vec!["src/lib.rs".to_string()];

        let mut link3 = create_test_link("commit-3", "session-1", "main");
        link3.files_changed = vec!["src/main.rs".to_string(), "tests/test.rs".to_string()];

        store.add(&link1).unwrap();
        store.add(&link2).unwrap();
        store.add(&link3).unwrap();

        let main_commits = store.find_by_file("src/main.rs", 10).unwrap();
        assert_eq!(main_commits.len(), 2);

        let lib_commits = store.find_by_file("src/lib.rs", 10).unwrap();
        assert_eq!(lib_commits.len(), 1);
    }

    #[test]
    fn test_prune() {
        let (store, _dir) = setup_store();

        store
            .add(&create_test_link("commit-1", "session-1", "main"))
            .unwrap();

        // Prune commits older than 30 days (should delete nothing)
        let deleted = store.prune(30).unwrap();
        assert_eq!(deleted, 0);

        let links = store.list_recent(10).unwrap();
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_json_arrays_serialization() {
        let (store, _dir) = setup_store();

        let link = CommitLink::new(
            "abc123".to_string(),
            "session-1".to_string(),
            "agent-1".to_string(),
            "main".to_string(),
            "Test".to_string(),
            vec![
                "file1.rs".to_string(),
                "file2.rs".to_string(),
                "file3.rs".to_string(),
            ],
            vec!["prompt-1".to_string(), "prompt-2".to_string()],
            "Test <test@test.com>".to_string(),
        );

        store.add(&link).unwrap();

        let retrieved = store.get("abc123").unwrap().unwrap();
        assert_eq!(retrieved.files_changed.len(), 3);
        assert_eq!(retrieved.prompt_ids.len(), 2);
        assert!(retrieved.files_changed.contains(&"file2.rs".to_string()));
        assert!(retrieved.prompt_ids.contains(&"prompt-2".to_string()));
    }
}
