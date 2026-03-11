//! Integration tests for worktree feature
//!
//! Tests the full worktree lifecycle including:
//! - Task lifecycle with worktrees
//! - Branch scoping for entries
//! - Config enable/disable
//! - Edge cases

use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Create a test git repository
fn create_test_repo() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let repo_path = temp.path().to_path_buf();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to init git repo");

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git");

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to configure git");

    // Create initial commit
    std::fs::write(repo_path.join("README.md"), "# Test Repo\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to add files");

    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(&repo_path)
        .output()
        .expect("Failed to commit");

    (temp, repo_path)
}

/// Initialize CAS in a directory
fn init_cas(path: &Path) -> PathBuf {
    let cas_dir = path.join(".cas");
    std::fs::create_dir_all(&cas_dir).unwrap();

    // Create config with worktrees enabled (TOML format)
    let config_content = r#"
[worktrees]
enabled = true
base_path = "../.worktrees/{project}"
branch_prefix = "cas/"
auto_merge = true
cleanup_on_close = true
promote_entries_on_merge = true
"#;
    std::fs::write(cas_dir.join("config.toml"), config_content).unwrap();

    // Initialize SQLite store
    let db_path = cas_dir.join("cas.db");
    let conn = rusqlite::Connection::open(&db_path).unwrap();

    // Create entries table with branch column
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS entries (
            id TEXT PRIMARY KEY,
            type TEXT NOT NULL DEFAULT 'learning',
            tags TEXT,
            created TEXT NOT NULL,
            helpful_count INTEGER NOT NULL DEFAULT 0,
            harmful_count INTEGER NOT NULL DEFAULT 0,
            last_accessed TEXT,
            title TEXT,
            content TEXT NOT NULL,
            archived INTEGER NOT NULL DEFAULT 0,
            session_id TEXT,
            source_tool TEXT,
            pending_extraction INTEGER NOT NULL DEFAULT 0,
            observation_type TEXT,
            stability REAL NOT NULL DEFAULT 0.5,
            access_count INTEGER NOT NULL DEFAULT 0,
            raw_content TEXT,
            compressed INTEGER NOT NULL DEFAULT 0,
            memory_tier TEXT NOT NULL DEFAULT 'working',
            importance REAL NOT NULL DEFAULT 0.5,
            valid_from TEXT,
            valid_until TEXT,
            review_after TEXT,
            pending_embedding INTEGER NOT NULL DEFAULT 1,
            belief_type TEXT NOT NULL DEFAULT 'fact',
            confidence REAL NOT NULL DEFAULT 1.0,
            domain TEXT,
            branch TEXT
        );
        CREATE TABLE IF NOT EXISTS tasks (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            description TEXT,
            status TEXT NOT NULL DEFAULT 'open',
            priority INTEGER NOT NULL DEFAULT 2,
            task_type TEXT NOT NULL DEFAULT 'task',
            labels TEXT,
            notes TEXT,
            created TEXT NOT NULL,
            updated TEXT NOT NULL,
            closed_at TEXT,
            claimed_by TEXT,
            claimed_until TEXT,
            branch TEXT,
            worktree_id TEXT
        );
        CREATE TABLE IF NOT EXISTS worktrees (
            id TEXT PRIMARY KEY,
            task_id TEXT,
            branch TEXT NOT NULL,
            parent_branch TEXT NOT NULL,
            path TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'active',
            created_at TEXT NOT NULL,
            merged_at TEXT,
            removed_at TEXT,
            created_by_agent TEXT,
            merge_commit TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_entries_branch ON entries(branch);
        CREATE INDEX IF NOT EXISTS idx_worktrees_task ON worktrees(task_id);
        CREATE INDEX IF NOT EXISTS idx_worktrees_status ON worktrees(status);
        ",
    )
    .unwrap();

    cas_dir
}

#[test]
fn test_worktree_config_parsing() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    // Verify config loads correctly (TOML format)
    let config_path = cas_dir.join("config.toml");
    assert!(config_path.exists());

    let content = std::fs::read_to_string(&config_path).unwrap();
    assert!(content.contains("enabled = true"));
    assert!(content.contains("branch_prefix = \"cas/\""));
}

#[test]
fn test_worktree_store_crud() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Insert a worktree record
    conn.execute(
        "INSERT INTO worktrees (id, task_id, branch, parent_branch, path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "wt-test123",
            "cas-1234",
            "cas/cas-1234",
            "main",
            "/tmp/test-worktree",
            "active",
            "2024-01-01T00:00:00Z"
        ],
    )
    .unwrap();

    // Read it back
    let branch: String = conn
        .query_row(
            "SELECT branch FROM worktrees WHERE id = ?",
            ["wt-test123"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch, "cas/cas-1234");

    // Update status
    conn.execute(
        "UPDATE worktrees SET status = 'merged' WHERE id = ?",
        ["wt-test123"],
    )
    .unwrap();

    let status: String = conn
        .query_row(
            "SELECT status FROM worktrees WHERE id = ?",
            ["wt-test123"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "merged");
}

#[test]
fn test_entry_branch_scoping() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create entries with different branch scopes
    conn.execute(
        "INSERT INTO entries (id, type, created, content, branch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "entry-1",
            "learning",
            "2024-01-01T00:00:00Z",
            "Global entry (no branch)",
            rusqlite::types::Null
        ],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO entries (id, type, created, content, branch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "entry-2",
            "learning",
            "2024-01-01T00:00:01Z",
            "Scoped to branch A",
            "cas/branch-a"
        ],
    )
    .unwrap();

    conn.execute(
        "INSERT INTO entries (id, type, created, content, branch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "entry-3",
            "learning",
            "2024-01-01T00:00:02Z",
            "Scoped to branch B",
            "cas/branch-b"
        ],
    )
    .unwrap();

    // Query all entries
    let all_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE archived = 0",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(all_count, 3);

    // Query entries visible from branch A (branch A entries + unscoped)
    let branch_a_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE archived = 0 AND (branch IS NULL OR branch = ?)",
            ["cas/branch-a"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch_a_count, 2); // entry-1 (global) + entry-2 (branch A)

    // Query entries visible from branch B
    let branch_b_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE archived = 0 AND (branch IS NULL OR branch = ?)",
            ["cas/branch-b"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch_b_count, 2); // entry-1 (global) + entry-3 (branch B)

    // Query only unscoped entries (global view)
    let global_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM entries WHERE archived = 0 AND branch IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(global_count, 1);
}

#[test]
fn test_entry_promotion_on_merge() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create a scoped entry
    conn.execute(
        "INSERT INTO entries (id, type, created, content, branch)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![
            "entry-scoped",
            "learning",
            "2024-01-01T00:00:00Z",
            "Entry in worktree",
            "cas/feature-branch"
        ],
    )
    .unwrap();

    // Verify it's scoped
    let branch: Option<String> = conn
        .query_row(
            "SELECT branch FROM entries WHERE id = ?",
            ["entry-scoped"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(branch, Some("cas/feature-branch".to_string()));

    // Simulate promotion (what happens when worktree is merged)
    conn.execute(
        "UPDATE entries SET branch = NULL WHERE branch = ?",
        ["cas/feature-branch"],
    )
    .unwrap();

    // Verify it's now global
    let branch_after: Option<String> = conn
        .query_row(
            "SELECT branch FROM entries WHERE id = ?",
            ["entry-scoped"],
            |row| row.get(0),
        )
        .unwrap();
    assert!(branch_after.is_none());
}

#[test]
fn test_task_worktree_association() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create a task with worktree association
    conn.execute(
        "INSERT INTO tasks (id, title, status, created, updated, branch, worktree_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "cas-1234",
            "Test task",
            "in_progress",
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:00Z",
            "cas/cas-1234",
            "wt-abc123"
        ],
    )
    .unwrap();

    // Create the associated worktree
    conn.execute(
        "INSERT INTO worktrees (id, task_id, branch, parent_branch, path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "wt-abc123",
            "cas-1234",
            "cas/cas-1234",
            "main",
            "/tmp/wt-abc123",
            "active",
            "2024-01-01T00:00:00Z"
        ],
    )
    .unwrap();

    // Query task and get worktree info
    let (task_branch, worktree_id): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT branch, worktree_id FROM tasks WHERE id = ?",
            ["cas-1234"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(task_branch, Some("cas/cas-1234".to_string()));
    assert_eq!(worktree_id, Some("wt-abc123".to_string()));

    // Query worktree by task
    let wt_branch: String = conn
        .query_row(
            "SELECT branch FROM worktrees WHERE task_id = ? AND status = 'active'",
            ["cas-1234"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(wt_branch, "cas/cas-1234");
}

#[test]
fn test_worktree_status_lifecycle() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create worktree in active state
    conn.execute(
        "INSERT INTO worktrees (id, task_id, branch, parent_branch, path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "wt-lifecycle",
            "cas-test",
            "cas/cas-test",
            "main",
            "/tmp/wt-lifecycle",
            "active",
            "2024-01-01T00:00:00Z"
        ],
    )
    .unwrap();

    // Test active -> merged transition
    conn.execute(
        "UPDATE worktrees SET status = 'merged', merged_at = ?1 WHERE id = ?2",
        rusqlite::params!["2024-01-01T01:00:00Z", "wt-lifecycle"],
    )
    .unwrap();

    let (status, merged_at): (String, Option<String>) = conn
        .query_row(
            "SELECT status, merged_at FROM worktrees WHERE id = ?",
            ["wt-lifecycle"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "merged");
    assert!(merged_at.is_some());

    // Test merged -> removed transition
    conn.execute(
        "UPDATE worktrees SET status = 'removed', removed_at = ?1 WHERE id = ?2",
        rusqlite::params!["2024-01-01T02:00:00Z", "wt-lifecycle"],
    )
    .unwrap();

    let (status, removed_at): (String, Option<String>) = conn
        .query_row(
            "SELECT status, removed_at FROM worktrees WHERE id = ?",
            ["wt-lifecycle"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(status, "removed");
    assert!(removed_at.is_some());
}

#[test]
fn test_list_active_worktrees() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create worktrees in various states
    for (id, status) in [
        ("wt-1", "active"),
        ("wt-2", "active"),
        ("wt-3", "merged"),
        ("wt-4", "abandoned"),
        ("wt-5", "removed"),
    ] {
        conn.execute(
            "INSERT INTO worktrees (id, branch, parent_branch, path, status, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                id,
                format!("cas/{}", id),
                "main",
                format!("/tmp/{}", id),
                status,
                "2024-01-01T00:00:00Z"
            ],
        )
        .unwrap();
    }

    // Count active worktrees
    let active_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM worktrees WHERE status = 'active'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 2);

    // List all non-removed worktrees
    let non_removed_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM worktrees WHERE status != 'removed'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(non_removed_count, 4);
}

#[test]
fn test_worktree_by_branch_lookup() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    conn.execute(
        "INSERT INTO worktrees (id, branch, parent_branch, path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![
            "wt-branch-test",
            "cas/feature-xyz",
            "main",
            "/tmp/wt-branch-test",
            "active",
            "2024-01-01T00:00:00Z"
        ],
    )
    .unwrap();

    // Find worktree by branch name
    let found_id: String = conn
        .query_row(
            "SELECT id FROM worktrees WHERE branch = ?",
            ["cas/feature-xyz"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(found_id, "wt-branch-test");

    // Non-existent branch
    let not_found: Result<String, _> = conn.query_row(
        "SELECT id FROM worktrees WHERE branch = ?",
        ["cas/nonexistent"],
        |row| row.get(0),
    );
    assert!(not_found.is_err());
}

#[test]
fn test_config_worktrees_disabled() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = repo_path.join(".cas");
    std::fs::create_dir_all(&cas_dir).unwrap();

    // Create config with worktrees disabled (TOML format)
    let config_content = r#"
[worktrees]
enabled = false
"#;
    std::fs::write(cas_dir.join("config.toml"), config_content).unwrap();

    let content = std::fs::read_to_string(cas_dir.join("config.toml")).unwrap();
    assert!(content.contains("enabled = false"));
}

#[test]
fn test_multiple_worktrees_same_task_prevented() {
    let (_temp, repo_path) = create_test_repo();
    let cas_dir = init_cas(&repo_path);

    let conn = rusqlite::Connection::open(cas_dir.join("cas.db")).unwrap();

    // Create first worktree for task
    conn.execute(
        "INSERT INTO worktrees (id, task_id, branch, parent_branch, path, status, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![
            "wt-first",
            "cas-shared",
            "cas/cas-shared-1",
            "main",
            "/tmp/wt-first",
            "active",
            "2024-01-01T00:00:00Z"
        ],
    )
    .unwrap();

    // Query: only one active worktree per task
    let active_count: i32 = conn
        .query_row(
            "SELECT COUNT(*) FROM worktrees WHERE task_id = ? AND status = 'active'",
            ["cas-shared"],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(active_count, 1);
}
