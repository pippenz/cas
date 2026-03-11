use crate::PROMPT_SCHEMA;
use crate::file_change_store::*;
use tempfile::TempDir;

fn setup_store() -> (SqliteFileChangeStore, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("cas.db");
    let conn = Connection::open(&db_path).unwrap();

    // Set busy timeout for concurrent access
    conn.busy_timeout(crate::SQLITE_BUSY_TIMEOUT).unwrap();

    // Enable WAL mode and foreign keys
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA foreign_keys=ON;",
    )
    .unwrap();

    // Create prompts table first (required for foreign key)
    conn.execute_batch(PROMPT_SCHEMA).unwrap();

    // Create file_changes table
    conn.execute_batch(FILE_CHANGE_SCHEMA).unwrap();

    drop(conn);

    let store = SqliteFileChangeStore::open(temp_dir.path()).unwrap();
    (store, temp_dir)
}

/// Helper to create a test prompt in the database
fn create_test_prompt(dir: &TempDir, prompt_id: &str) {
    let db_path = dir.path().join("cas.db");
    let conn = Connection::open(&db_path).unwrap();
    // Use prompt_id as content_hash to ensure uniqueness
    conn.execute(
            "INSERT INTO prompts (id, session_id, agent_id, content, content_hash, timestamp, scope)
             VALUES (?1, 'test-session', 'test-agent', 'test prompt', ?2, datetime('now'), 'project')",
            params![prompt_id, format!("hash-{}", prompt_id)],
        ).unwrap();
}

fn create_test_change(id: &str, session_id: &str, file_path: &str) -> FileChange {
    FileChange::new(
        id.to_string(),
        session_id.to_string(),
        "agent-1".to_string(),
        "test-repo".to_string(),
        file_path.to_string(),
        ChangeType::Modified,
        "Edit".to_string(),
        "hash123".to_string(),
    )
}

#[test]
fn test_add_and_get() {
    let (store, _dir) = setup_store();

    let change = create_test_change("fc-001", "session-1", "src/main.rs");
    store.add(&change).unwrap();

    let retrieved = store.get("fc-001").unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.id, "fc-001");
    assert_eq!(retrieved.file_path, "src/main.rs");
    assert_eq!(retrieved.change_type, ChangeType::Modified);
}

#[test]
fn test_list_by_session() {
    let (store, _dir) = setup_store();

    store
        .add(&create_test_change("fc-001", "session-A", "file1.rs"))
        .unwrap();
    store
        .add(&create_test_change("fc-002", "session-A", "file2.rs"))
        .unwrap();
    store
        .add(&create_test_change("fc-003", "session-B", "file3.rs"))
        .unwrap();

    let session_a = store.list_by_session("session-A", 10).unwrap();
    assert_eq!(session_a.len(), 2);

    let session_b = store.list_by_session("session-B", 10).unwrap();
    assert_eq!(session_b.len(), 1);
}

#[test]
fn test_list_by_prompt() {
    let (store, dir) = setup_store();

    // Create the prompt first (FK constraint)
    create_test_prompt(&dir, "prompt-abc");

    let mut change1 = create_test_change("fc-001", "session-1", "file1.rs");
    change1.prompt_id = Some("prompt-abc".to_string());

    let mut change2 = create_test_change("fc-002", "session-1", "file2.rs");
    change2.prompt_id = Some("prompt-abc".to_string());

    let change3 = create_test_change("fc-003", "session-1", "file3.rs");
    // No prompt_id

    store.add(&change1).unwrap();
    store.add(&change2).unwrap();
    store.add(&change3).unwrap();

    let prompt_changes = store.list_by_prompt("prompt-abc", 10).unwrap();
    assert_eq!(prompt_changes.len(), 2);
}

#[test]
fn test_list_by_file() {
    let (store, _dir) = setup_store();

    store
        .add(&create_test_change("fc-001", "session-1", "src/main.rs"))
        .unwrap();
    store
        .add(&create_test_change("fc-002", "session-2", "src/main.rs"))
        .unwrap();
    store
        .add(&create_test_change("fc-003", "session-1", "src/lib.rs"))
        .unwrap();

    let main_changes = store.list_by_file("test-repo", "src/main.rs", 10).unwrap();
    assert_eq!(main_changes.len(), 2);

    let lib_changes = store.list_by_file("test-repo", "src/lib.rs", 10).unwrap();
    assert_eq!(lib_changes.len(), 1);
}

#[test]
fn test_uncommitted_and_link() {
    let (store, _dir) = setup_store();

    store
        .add(&create_test_change("fc-001", "session-1", "file1.rs"))
        .unwrap();
    store
        .add(&create_test_change("fc-002", "session-1", "file2.rs"))
        .unwrap();

    // Both should be uncommitted
    let uncommitted = store.list_uncommitted("session-1").unwrap();
    assert_eq!(uncommitted.len(), 2);

    // Link to commit
    let ids: Vec<String> = uncommitted.iter().map(|c| c.id.clone()).collect();
    let updated = store.link_to_commit(&ids, "abc123def").unwrap();
    assert_eq!(updated, 2);

    // Should be none uncommitted now
    let uncommitted = store.list_uncommitted("session-1").unwrap();
    assert_eq!(uncommitted.len(), 0);

    // Should be findable by commit
    let committed = store.list_by_commit("abc123def").unwrap();
    assert_eq!(committed.len(), 2);
    assert!(committed[0].commit_hash.is_some());
    assert!(committed[0].committed_at.is_some());
}

#[test]
fn test_list_recent() {
    let (store, _dir) = setup_store();

    for i in 0..5 {
        store
            .add(&create_test_change(
                &format!("fc-{i}"),
                "session-1",
                &format!("file{i}.rs"),
            ))
            .unwrap();
    }

    let recent = store.list_recent(3).unwrap();
    assert_eq!(recent.len(), 3);
}

#[test]
fn test_prune() {
    let (store, _dir) = setup_store();

    store
        .add(&create_test_change("fc-001", "session-1", "file.rs"))
        .unwrap();

    // Prune changes older than 30 days (should delete nothing)
    let deleted = store.prune(30).unwrap();
    assert_eq!(deleted, 0);

    let changes = store.list_recent(10).unwrap();
    assert_eq!(changes.len(), 1);
}

#[test]
fn test_change_with_prompt() {
    let (store, dir) = setup_store();

    // Create the prompt first (FK constraint)
    create_test_prompt(&dir, "prompt-xyz");

    let change = FileChange::with_prompt(
        "fc-001".to_string(),
        "session-1".to_string(),
        "agent-1".to_string(),
        Some("prompt-xyz".to_string()),
        "repo".to_string(),
        "file.rs".to_string(),
        ChangeType::Created,
        "Write".to_string(),
        None,
        "newhash".to_string(),
    );

    store.add(&change).unwrap();

    let retrieved = store.get("fc-001").unwrap().unwrap();
    assert_eq!(retrieved.prompt_id, Some("prompt-xyz".to_string()));
    assert_eq!(retrieved.change_type, ChangeType::Created);
    assert!(retrieved.old_content_hash.is_none());
}
