//! Syncing entry store wrapper
//!
//! Automatically queues entry changes for cloud sync on add/update/delete.

use std::path::Path;
use std::sync::Arc;

use crate::cloud::{EntityType, SyncOperation, SyncQueue};
use crate::store::{Result, Store};
use crate::types::Entry;

/// An entry store wrapper that queues changes for cloud sync
pub struct SyncingEntryStore {
    inner: Arc<dyn Store>,
    queue: Arc<SyncQueue>,
}

impl SyncingEntryStore {
    /// Create a new syncing entry store
    pub fn new(inner: Arc<dyn Store>, queue: Arc<SyncQueue>) -> Self {
        Self { inner, queue }
    }

    fn queue_upsert(&self, entry: &Entry) {
        // Best-effort queuing - don't fail the operation if queue fails
        if let Ok(payload) = serde_json::to_string(entry) {
            let _ = self.queue.enqueue(
                EntityType::Entry,
                &entry.id,
                SyncOperation::Upsert,
                Some(&payload),
            );
        }
    }

    fn queue_delete(&self, id: &str) {
        let _ = self
            .queue
            .enqueue(EntityType::Entry, id, SyncOperation::Delete, None);
    }
}

impl Store for SyncingEntryStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn generate_id(&self) -> Result<String> {
        self.inner.generate_id()
    }

    fn add(&self, entry: &Entry) -> Result<()> {
        self.inner.add(entry)?;
        self.queue_upsert(entry);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Entry> {
        self.inner.get(id)
    }

    fn get_archived(&self, id: &str) -> Result<Entry> {
        self.inner.get_archived(id)
    }

    fn update(&self, entry: &Entry) -> Result<()> {
        self.inner.update(entry)?;
        self.queue_upsert(entry);
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.inner.delete(id)?;
        self.queue_delete(id);
        Ok(())
    }

    fn list(&self) -> Result<Vec<Entry>> {
        self.inner.list()
    }

    fn recent(&self, n: usize) -> Result<Vec<Entry>> {
        self.inner.recent(n)
    }

    fn archive(&self, id: &str) -> Result<()> {
        self.inner.archive(id)?;
        // Queue the archived entry state
        if let Ok(entry) = self.inner.get_archived(id) {
            self.queue_upsert(&entry);
        }
        Ok(())
    }

    fn unarchive(&self, id: &str) -> Result<()> {
        self.inner.unarchive(id)?;
        // Queue the unarchived entry state
        if let Ok(entry) = self.inner.get(id) {
            self.queue_upsert(&entry);
        }
        Ok(())
    }

    fn list_archived(&self) -> Result<Vec<Entry>> {
        self.inner.list_archived()
    }

    fn list_pending(&self, limit: usize) -> Result<Vec<Entry>> {
        self.inner.list_pending(limit)
    }

    fn mark_extracted(&self, id: &str) -> Result<()> {
        self.inner.mark_extracted(id)?;
        // Queue the updated entry
        if let Ok(entry) = self.inner.get(id) {
            self.queue_upsert(&entry);
        }
        Ok(())
    }

    fn list_pinned(&self) -> Result<Vec<Entry>> {
        self.inner.list_pinned()
    }

    fn list_helpful(&self, limit: usize) -> Result<Vec<Entry>> {
        self.inner.list_helpful(limit)
    }

    fn list_by_session(&self, session_id: &str) -> Result<Vec<Entry>> {
        self.inner.list_by_session(session_id)
    }

    fn list_unreviewed_learnings(&self, limit: usize) -> Result<Vec<Entry>> {
        self.inner.list_unreviewed_learnings(limit)
    }

    fn mark_reviewed(&self, id: &str) -> Result<()> {
        self.inner.mark_reviewed(id)?;
        // Queue the updated entry for sync
        if let Ok(entry) = self.inner.get(id) {
            self.queue_upsert(&entry);
        }
        Ok(())
    }

    fn list_pending_index(&self, limit: usize) -> Result<Vec<Entry>> {
        self.inner.list_pending_index(limit)
    }

    fn mark_indexed(&self, id: &str) -> Result<()> {
        self.inner.mark_indexed(id)
        // Note: We don't queue for sync on mark_indexed as it's a local-only flag
    }

    fn mark_indexed_batch(&self, ids: &[&str]) -> Result<()> {
        self.inner.mark_indexed_batch(ids)
        // Note: We don't queue for sync on mark_indexed as it's a local-only flag
    }

    fn cas_dir(&self) -> &Path {
        self.inner.cas_dir()
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use crate::store::SqliteStore;
    use crate::store::syncing_entry::*;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SyncingEntryStore) {
        let temp = TempDir::new().unwrap();
        let cas_dir = temp.path();

        let inner = SqliteStore::open(cas_dir).unwrap();
        inner.init().unwrap();

        let queue = SyncQueue::open(cas_dir).unwrap();
        queue.init().unwrap();

        let store = SyncingEntryStore::new(Arc::new(inner), Arc::new(queue));
        (temp, store)
    }

    #[test]
    fn test_add_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let entry = Entry::new("entry-001".to_string(), "Test content".to_string());
        store.add(&entry).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entity_type, EntityType::Entry);
        assert_eq!(pending[0].entity_id, entry.id);
        assert_eq!(pending[0].operation, SyncOperation::Upsert);
    }

    #[test]
    fn test_update_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut entry = Entry::new("entry-002".to_string(), "Test content".to_string());
        store.add(&entry).unwrap();

        // Clear queue
        queue.clear().unwrap();

        entry.content = "Updated content".to_string();
        store.update(&entry).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(
            pending[0]
                .payload
                .as_ref()
                .unwrap()
                .contains("Updated content")
        );
    }

    #[test]
    fn test_delete_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let entry = Entry::new("entry-003".to_string(), "Test content".to_string());
        store.add(&entry).unwrap();

        // Clear queue
        queue.clear().unwrap();

        store.delete(&entry.id).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation, SyncOperation::Delete);
    }
}
