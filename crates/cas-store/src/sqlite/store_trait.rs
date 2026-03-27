use crate::sqlite::SqliteStore;
use crate::{Result, Store};
use cas_types::Entry;
use std::path::Path;

impl Store for SqliteStore {
    fn init(&self) -> Result<()> {
        self.store_init()
    }

    fn generate_id(&self) -> Result<String> {
        self.store_generate_id()
    }

    fn add(&self, entry: &Entry) -> Result<()> {
        self.store_add(entry)
    }

    fn get(&self, id: &str) -> Result<Entry> {
        self.store_get(id)
    }

    fn get_archived(&self, id: &str) -> Result<Entry> {
        self.store_get_archived(id)
    }

    fn update(&self, entry: &Entry) -> Result<()> {
        self.store_update(entry)
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.store_delete(id)
    }

    fn list(&self) -> Result<Vec<Entry>> {
        self.store_list()
    }

    fn recent(&self, n: usize) -> Result<Vec<Entry>> {
        self.store_recent(n)
    }

    fn archive(&self, id: &str) -> Result<()> {
        self.store_archive(id)
    }

    fn unarchive(&self, id: &str) -> Result<()> {
        self.store_unarchive(id)
    }

    fn list_archived(&self) -> Result<Vec<Entry>> {
        self.store_list_archived()
    }

    fn list_by_branch(&self, branch: &str) -> Result<Vec<Entry>> {
        self.store_list_by_branch(branch)
    }

    fn list_pending(&self, limit: usize) -> Result<Vec<Entry>> {
        self.store_list_pending(limit)
    }

    fn mark_extracted(&self, id: &str) -> Result<()> {
        self.store_mark_extracted(id)
    }

    fn list_pinned(&self) -> Result<Vec<Entry>> {
        self.store_list_pinned()
    }

    fn list_helpful(&self, limit: usize) -> Result<Vec<Entry>> {
        self.store_list_helpful(limit)
    }

    fn list_by_session(&self, session_id: &str) -> Result<Vec<Entry>> {
        self.store_list_by_session(session_id)
    }

    fn list_unreviewed_learnings(&self, limit: usize) -> Result<Vec<Entry>> {
        self.store_list_unreviewed_learnings(limit)
    }

    fn mark_reviewed(&self, id: &str) -> Result<()> {
        self.store_mark_reviewed(id)
    }

    fn list_pending_index(&self, limit: usize) -> Result<Vec<Entry>> {
        self.store_list_pending_index(limit)
    }

    fn mark_indexed(&self, id: &str) -> Result<()> {
        self.store_mark_indexed(id)
    }

    fn mark_indexed_batch(&self, ids: &[&str]) -> Result<()> {
        self.store_mark_indexed_batch(ids)
    }

    fn cas_dir(&self) -> &Path {
        self.store_cas_dir()
    }

    fn close(&self) -> Result<()> {
        self.store_close()
    }
}
