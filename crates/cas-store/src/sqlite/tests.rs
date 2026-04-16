use crate::Store;
use crate::sqlite::SqliteStore;
use cas_types::{Entry, ShareScope};
use tempfile::TempDir;

// Imports used only by the per-entity round-trip tests below.
use crate::{RuleStore, SkillStore, TaskStore};
use crate::{SqliteRuleStore, SqliteSkillStore, SqliteTaskStore};
use cas_types::{Rule, Skill, Task};

#[test]
fn test_sqlite_store_crud() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open(temp.path()).unwrap();
    store.init().unwrap();

    // Generate ID and add entry
    let id = store.generate_id().unwrap();
    let entry = Entry {
        id: id.clone(),
        content: "Test content".to_string(),
        ..Default::default()
    };
    store.add(&entry).unwrap();

    // Get entry
    let retrieved = store.get(&id).unwrap();
    assert_eq!(retrieved.content, "Test content");

    // Update entry
    let mut updated = retrieved;
    updated.content = "Updated content".to_string();
    updated.helpful_count = 5;
    store.update(&updated).unwrap();

    let retrieved = store.get(&id).unwrap();
    assert_eq!(retrieved.content, "Updated content");
    assert_eq!(retrieved.helpful_count, 5);

    // List entries
    let entries = store.list().unwrap();
    assert_eq!(entries.len(), 1);

    // Archive entry
    store.archive(&id).unwrap();
    assert!(store.get(&id).is_err());

    let archived = store.list_archived().unwrap();
    assert_eq!(archived.len(), 1);

    // Unarchive entry
    store.unarchive(&id).unwrap();
    assert!(store.get(&id).is_ok());

    // Delete entry
    store.delete(&id).unwrap();
    assert!(store.get(&id).is_err());
}

/// T5 cas-07d7: Entry.share must round-trip through SQLite.
/// Covers the three states — None, Private, Team — across add→get
/// and update→get, plus archive→unarchive (the P1 residual where
/// reload was losing share on the rule path).
#[test]
fn test_entry_share_roundtrip() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open(temp.path()).unwrap();
    store.init().unwrap();

    // None (default) round-trips
    let id_none = store.generate_id().unwrap();
    let e_none = Entry {
        id: id_none.clone(),
        content: "no share".to_string(),
        ..Default::default()
    };
    store.add(&e_none).unwrap();
    assert_eq!(store.get(&id_none).unwrap().share, None);

    // Some(Team) round-trips through add
    let id_team = store.generate_id().unwrap();
    let e_team = Entry {
        id: id_team.clone(),
        content: "team share".to_string(),
        share: Some(ShareScope::Team),
        ..Default::default()
    };
    store.add(&e_team).unwrap();
    assert_eq!(store.get(&id_team).unwrap().share, Some(ShareScope::Team));

    // Some(Private) round-trips through update
    let mut e_priv = store.get(&id_none).unwrap();
    e_priv.share = Some(ShareScope::Private);
    store.update(&e_priv).unwrap();
    assert_eq!(
        store.get(&id_none).unwrap().share,
        Some(ShareScope::Private)
    );

    // Archive/unarchive preserves share (P1 residual from T3 review)
    store.archive(&id_team).unwrap();
    store.unarchive(&id_team).unwrap();
    assert_eq!(store.get(&id_team).unwrap().share, Some(ShareScope::Team));
}

/// T5 cas-07d7: Rule.share must round-trip. The rule store's INSERT/
/// UPDATE/SELECT paths were all extended to cover `share` — this test
/// guards against future column-list drift.
#[test]
fn test_rule_share_roundtrip() {
    let temp = TempDir::new().unwrap();
    let store = SqliteRuleStore::open(temp.path()).unwrap();
    store.init().unwrap();

    let mut r = Rule::new("rule-001".to_string(), "test".to_string());
    r.share = Some(ShareScope::Team);
    store.add(&r).unwrap();
    assert_eq!(store.get("rule-001").unwrap().share, Some(ShareScope::Team));

    // Update round-trip
    let mut loaded = store.get("rule-001").unwrap();
    loaded.share = Some(ShareScope::Private);
    store.update(&loaded).unwrap();
    assert_eq!(
        store.get("rule-001").unwrap().share,
        Some(ShareScope::Private)
    );

    // None round-trip
    let mut loaded = store.get("rule-001").unwrap();
    loaded.share = None;
    store.update(&loaded).unwrap();
    assert_eq!(store.get("rule-001").unwrap().share, None);
}

/// T5 cas-07d7: Skill.share must round-trip. First code-review round
/// found the skill SELECT/INSERT/UPDATE had silently skipped the
/// `share` column; this test exists to catch that regression class.
#[test]
fn test_skill_share_roundtrip() {
    let temp = TempDir::new().unwrap();
    let store = SqliteSkillStore::open(temp.path()).unwrap();
    store.init().unwrap();

    let mut s = Skill::new("sk-share-01".to_string(), "test".to_string());
    s.share = Some(ShareScope::Team);
    store.add(&s).unwrap();
    assert_eq!(
        store.get("sk-share-01").unwrap().share,
        Some(ShareScope::Team)
    );

    let mut loaded = store.get("sk-share-01").unwrap();
    loaded.share = Some(ShareScope::Private);
    store.update(&loaded).unwrap();
    assert_eq!(
        store.get("sk-share-01").unwrap().share,
        Some(ShareScope::Private)
    );
}

/// T5 cas-07d7: Task.share must round-trip through the many SELECT
/// sites and the INSERT/UPDATE paths.
#[test]
fn test_task_share_roundtrip() {
    let temp = TempDir::new().unwrap();
    let store = SqliteTaskStore::open(temp.path()).unwrap();
    store.init().unwrap();

    let mut t = Task::new("cas-share".to_string(), "test task".to_string());
    t.share = Some(ShareScope::Team);
    store.add(&t).unwrap();
    assert_eq!(store.get("cas-share").unwrap().share, Some(ShareScope::Team));

    let mut loaded = store.get("cas-share").unwrap();
    loaded.share = Some(ShareScope::Private);
    store.update(&loaded).unwrap();
    assert_eq!(
        store.get("cas-share").unwrap().share,
        Some(ShareScope::Private)
    );
}

/// Helper to add session signal columns for testing
/// These columns are normally added by migrations m042-m044
fn add_session_signal_columns(store: &SqliteStore) {
    let conn = store.conn.lock().unwrap();
    // Ignore errors if columns already exist
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN outcome TEXT", []);
    let _ = conn.execute("ALTER TABLE sessions ADD COLUMN friction_score REAL", []);
    let _ = conn.execute(
        "ALTER TABLE sessions ADD COLUMN delight_count INTEGER DEFAULT 0",
        [],
    );
}

#[test]
fn test_signals_outcome_summary() {
    let temp = TempDir::new().unwrap();
    let store = SqliteStore::open(temp.path()).unwrap();
    store.init().unwrap();
    add_session_signal_columns(&store);

    // Create sessions with different outcomes
    let session1 = cas_types::Session::new(
        "session-1".to_string(),
        "/test".to_string(),
        Some("default".to_string()),
    );
    store.start_session(&session1).unwrap();
    store.end_session("session-1").unwrap();
    store
        .update_session_outcome("session-1", cas_types::SessionOutcome::TasksCompleted)
        .unwrap();

    let session2 = cas_types::Session::new(
        "session-2".to_string(),
        "/test".to_string(),
        Some("default".to_string()),
    );
    store.start_session(&session2).unwrap();
    store.end_session("session-2").unwrap();
    store
        .update_session_outcome("session-2", cas_types::SessionOutcome::TasksCompleted)
        .unwrap();

    let session3 = cas_types::Session::new(
        "session-3".to_string(),
        "/test".to_string(),
        Some("default".to_string()),
    );
    store.start_session(&session3).unwrap();
    store.end_session("session-3").unwrap();
    store
        .update_session_outcome("session-3", cas_types::SessionOutcome::Abandoned)
        .unwrap();

    // Query outcome summary
    let results = store.outcome_summary(30).unwrap();
    assert_eq!(results.len(), 2);

    // Find tasks_completed
    let completed = results
        .iter()
        .find(|(o, _, _)| o == "tasks_completed")
        .unwrap();
    assert_eq!(completed.1, 2); // count
    assert!((completed.2 - 66.67).abs() < 0.1); // ~66.67%

    // Find abandoned
    let abandoned = results.iter().find(|(o, _, _)| o == "abandoned").unwrap();
    assert_eq!(abandoned.1, 1); // count
    assert!((abandoned.2 - 33.33).abs() < 0.1); // ~33.33%
}
