use chrono::Utc;
use std::sync::Arc;

use crate::daemon::decay::apply_memory_decay;
use crate::daemon::{CodeIndexResult, DaemonConfig, DaemonStatus, WatchEvent};
use crate::store::Store;
use crate::store::mock::MockStore;
use crate::types::{Entry, EntryType, MemoryTier};

#[test]
fn test_daemon_config_default() {
    let config = DaemonConfig::default();
    assert_eq!(config.interval_minutes, 30);
    assert_eq!(config.model, "haiku");
    assert!(!config.auto_prune);
}

#[test]
fn test_daemon_status_default() {
    let status = DaemonStatus::default();
    assert!(!status.running);
    assert!(status.last_run.is_none());
}

fn make_entry(id: &str, entry_type: EntryType, tier: MemoryTier) -> Entry {
    Entry {
        id: id.to_string(),
        content: format!("Content for {id}"),
        entry_type,
        memory_tier: tier,
        created: Utc::now(),
        importance: 0.5,
        stability: 0.5,
        ..Default::default()
    }
}

fn make_store(entries: Vec<Entry>) -> Arc<dyn Store> {
    Arc::new(MockStore::with_entries(entries)) as Arc<dyn Store>
}

#[test]
fn test_observation_without_feedback_moves_to_cold() {
    let mut observation = make_entry("obs-001", EntryType::Observation, MemoryTier::Working);
    observation.helpful_count = 0;
    observation.harmful_count = 0;

    let store = make_store(vec![observation]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 1);

    let updated = store.get("obs-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Cold);
}

#[test]
fn test_observation_with_positive_feedback_stays_working() {
    let mut observation = make_entry("obs-001", EntryType::Observation, MemoryTier::Working);
    observation.helpful_count = 1;
    observation.harmful_count = 0;

    let store = make_store(vec![observation]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("obs-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Working);
}

#[test]
fn test_low_importance_moves_to_cold() {
    let mut low_imp = make_entry("low-001", EntryType::Learning, MemoryTier::Working);
    low_imp.importance = 0.2;
    low_imp.helpful_count = 0;
    low_imp.harmful_count = 0;

    let store = make_store(vec![low_imp]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 1);

    let updated = store.get("low-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Cold);
}

#[test]
fn test_low_importance_with_feedback_stays_working() {
    let mut low_imp = make_entry("low-001", EntryType::Learning, MemoryTier::Working);
    low_imp.importance = 0.2;
    low_imp.helpful_count = 1;

    let store = make_store(vec![low_imp]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("low-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Working);
}

#[test]
fn test_negative_feedback_moves_to_archive() {
    let mut negative = make_entry("neg-001", EntryType::Learning, MemoryTier::Working);
    negative.helpful_count = 0;
    negative.harmful_count = 2;

    let store = make_store(vec![negative]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 1);

    let updated = store.get("neg-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Archive);
}

#[test]
fn test_negative_feedback_from_cold_to_archive() {
    let mut negative = make_entry("neg-001", EntryType::Learning, MemoryTier::Cold);
    negative.helpful_count = 1;
    negative.harmful_count = 3;

    let store = make_store(vec![negative]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 1);

    let updated = store.get("neg-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Archive);
}

#[test]
fn test_in_context_entries_are_skipped() {
    let mut pinned = make_entry("pin-001", EntryType::Observation, MemoryTier::InContext);
    pinned.helpful_count = 0;
    pinned.harmful_count = 5;

    let store = make_store(vec![pinned]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 0);

    let updated = store.get("pin-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::InContext);
}

#[test]
fn test_low_stability_demotes_to_cold() {
    let mut low_stab = make_entry("stab-001", EntryType::Learning, MemoryTier::Working);
    low_stab.stability = 0.2;
    low_stab.importance = 0.5;

    let store = make_store(vec![low_stab]);
    let count = apply_memory_decay(&store).unwrap();

    assert!(count >= 1);

    let updated = store.get("stab-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Cold);
}

#[test]
fn test_very_low_stability_demotes_cold_to_archive() {
    let mut very_low_stab = make_entry("stab-001", EntryType::Learning, MemoryTier::Cold);
    very_low_stab.stability = 0.1;

    let store = make_store(vec![very_low_stab]);
    let count = apply_memory_decay(&store).unwrap();

    assert!(count >= 1);

    let updated = store.get("stab-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Archive);
}

#[test]
fn test_normal_entry_no_immediate_tier_change() {
    let normal = make_entry("norm-001", EntryType::Learning, MemoryTier::Working);

    let store = make_store(vec![normal]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("norm-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Working);
}

#[test]
fn test_multiple_entries_tiering() {
    let mut obs = make_entry("obs-001", EntryType::Observation, MemoryTier::Working);
    obs.helpful_count = 0;

    let mut negative = make_entry("neg-001", EntryType::Learning, MemoryTier::Working);
    negative.harmful_count = 1;

    let normal = make_entry("norm-001", EntryType::Learning, MemoryTier::Working);

    let store = make_store(vec![obs, negative, normal]);
    let count = apply_memory_decay(&store).unwrap();

    assert!(count >= 2);

    assert_eq!(store.get("obs-001").unwrap().memory_tier, MemoryTier::Cold);
    assert_eq!(
        store.get("neg-001").unwrap().memory_tier,
        MemoryTier::Archive
    );
    assert_eq!(
        store.get("norm-001").unwrap().memory_tier,
        MemoryTier::Working
    );
}

#[test]
fn test_already_archived_not_double_processed() {
    let mut archived = make_entry("arch-001", EntryType::Learning, MemoryTier::Archive);
    archived.harmful_count = 5;

    let store = make_store(vec![archived]);
    let count = apply_memory_decay(&store).unwrap();

    assert_eq!(count, 0);

    let updated = store.get("arch-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Archive);
}

#[test]
fn test_boundary_importance_value() {
    let mut boundary = make_entry("bound-001", EntryType::Learning, MemoryTier::Working);
    boundary.importance = 0.3;
    boundary.helpful_count = 0;

    let store = make_store(vec![boundary]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("bound-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Working);
}

#[test]
fn test_boundary_stability_value() {
    let mut boundary = make_entry("bound-001", EntryType::Learning, MemoryTier::Working);
    boundary.stability = 0.3;

    let store = make_store(vec![boundary]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("bound-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Working);
}

#[test]
fn test_observation_in_cold_stays_cold() {
    let mut obs_cold = make_entry("obs-001", EntryType::Observation, MemoryTier::Cold);
    obs_cold.helpful_count = 0;
    obs_cold.stability = 0.5;

    let store = make_store(vec![obs_cold]);
    let _count = apply_memory_decay(&store).unwrap();

    let updated = store.get("obs-001").unwrap();
    assert_eq!(updated.memory_tier, MemoryTier::Cold);
}

#[test]
fn test_code_index_result_default() {
    let result = CodeIndexResult::default();
    assert_eq!(result.files_indexed, 0);
    assert_eq!(result.files_deleted, 0);
    assert_eq!(result.symbols_indexed, 0);
    assert!(result.errors.is_empty());
}

#[test]
fn test_code_index_result_tracks_deletions() {
    let result = CodeIndexResult {
        files_deleted: 5,
        files_indexed: 10,
        ..Default::default()
    };
    assert_eq!(result.files_deleted, 5);
    assert_eq!(result.files_indexed, 10);
}

#[test]
fn test_watch_event_variants() {
    use std::path::PathBuf;

    let modified = WatchEvent::Modified(PathBuf::from("test.rs"));
    let deleted = WatchEvent::Deleted(PathBuf::from("deleted.rs"));
    let error = WatchEvent::Error("test error".to_string());

    match modified {
        WatchEvent::Modified(path) => assert_eq!(path, PathBuf::from("test.rs")),
        _ => panic!("Expected Modified variant"),
    }

    match deleted {
        WatchEvent::Deleted(path) => assert_eq!(path, PathBuf::from("deleted.rs")),
        _ => panic!("Expected Deleted variant"),
    }

    match error {
        WatchEvent::Error(message) => assert_eq!(message, "test error"),
        _ => panic!("Expected Error variant"),
    }
}
