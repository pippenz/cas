//! Syncing rule store wrapper
//!
//! Automatically syncs rules to Claude Code on add/update/delete,
//! and optionally queues changes for cloud sync.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cloud::{EntityType, SyncOperation, SyncQueue};
use crate::store::{Result, RuleStore};
use crate::types::Rule;
use cas_core::Syncer;

/// A rule store wrapper that syncs rules to Claude Code and cloud
pub struct SyncingRuleStore {
    inner: Arc<dyn RuleStore>,
    syncer: Syncer,
    /// Optional cloud sync queue
    cloud_queue: Option<Arc<SyncQueue>>,
}

impl SyncingRuleStore {
    /// Create a new syncing rule store (local sync only)
    pub fn new(inner: Arc<dyn RuleStore>, target_dir: PathBuf, min_helpful: i32) -> Self {
        Self {
            inner,
            syncer: Syncer::new(target_dir, min_helpful),
            cloud_queue: None,
        }
    }

    /// Create a new syncing rule store with cloud sync
    pub fn with_cloud_queue(
        inner: Arc<dyn RuleStore>,
        target_dir: PathBuf,
        min_helpful: i32,
        cloud_queue: Arc<SyncQueue>,
    ) -> Self {
        Self {
            inner,
            syncer: Syncer::new(target_dir, min_helpful),
            cloud_queue: Some(cloud_queue),
        }
    }

    fn try_sync(&self, rule: &Rule) {
        // Ignore sync errors - syncing is best-effort
        let _ = self.syncer.sync_rule(rule);
    }

    fn try_remove(&self, rule_id: &str) {
        let _ = self.syncer.remove_rule(rule_id);
    }

    fn queue_upsert(&self, rule: &Rule) {
        if let Some(queue) = &self.cloud_queue {
            if let Ok(payload) = serde_json::to_string(rule) {
                let _ = queue.enqueue(
                    EntityType::Rule,
                    &rule.id,
                    SyncOperation::Upsert,
                    Some(&payload),
                );
            }
        }
    }

    fn queue_delete(&self, id: &str) {
        if let Some(queue) = &self.cloud_queue {
            let _ = queue.enqueue(EntityType::Rule, id, SyncOperation::Delete, None);
        }
    }
}

impl RuleStore for SyncingRuleStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn generate_id(&self) -> Result<String> {
        self.inner.generate_id()
    }

    fn add(&self, rule: &Rule) -> Result<()> {
        self.inner.add(rule)?;
        self.try_sync(rule);
        self.queue_upsert(rule);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Rule> {
        self.inner.get(id)
    }

    fn update(&self, rule: &Rule) -> Result<()> {
        self.inner.update(rule)?;
        self.try_sync(rule);
        self.queue_upsert(rule);
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.inner.delete(id)?;
        self.try_remove(id);
        self.queue_delete(id);
        Ok(())
    }

    fn list(&self) -> Result<Vec<Rule>> {
        self.inner.list()
    }

    fn list_proven(&self) -> Result<Vec<Rule>> {
        self.inner.list_proven()
    }

    fn list_critical(&self) -> Result<Vec<Rule>> {
        self.inner.list_critical()
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }
}
