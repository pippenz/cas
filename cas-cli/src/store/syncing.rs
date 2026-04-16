//! Syncing rule store wrapper
//!
//! Automatically syncs rules to Claude Code on add/update/delete,
//! and optionally queues changes for cloud sync. When a team is configured
//! and the rule passes the T1 filter policy, the write is dual-enqueued
//! to both the personal queue and the team queue.

use std::path::PathBuf;
use std::sync::Arc;

use crate::cloud::{CloudConfig, EntityType, SyncOperation, SyncQueue};
use crate::store::share_policy::eligible_for_team_rule;
use crate::store::{Result, RuleStore};
use crate::types::Rule;
use cas_core::Syncer;

/// A rule store wrapper that syncs rules to Claude Code and cloud
pub struct SyncingRuleStore {
    inner: Arc<dyn RuleStore>,
    syncer: Syncer,
    /// Optional cloud sync queue
    cloud_queue: Option<Arc<SyncQueue>>,
    /// Optional cloud config for team auto-promotion. Only used when
    /// `cloud_queue` is also `Some`.
    cloud_config: Option<Arc<CloudConfig>>,
}

impl SyncingRuleStore {
    /// Create a new syncing rule store (local sync only)
    pub fn new(inner: Arc<dyn RuleStore>, target_dir: PathBuf, min_helpful: i32) -> Self {
        Self {
            inner,
            syncer: Syncer::new(target_dir, min_helpful),
            cloud_queue: None,
            cloud_config: None,
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
            cloud_config: None,
        }
    }

    /// Attach a cloud config for team auto-promotion. Meaningful only
    /// when `with_cloud_queue` also provided a queue — without a queue
    /// there is nowhere to enqueue.
    #[must_use]
    pub fn with_cloud_config(mut self, cloud_config: Arc<CloudConfig>) -> Self {
        self.cloud_config = Some(cloud_config);
        self
    }

    fn active_team_id(&self) -> Option<String> {
        self.cloud_config
            .as_ref()
            .and_then(|c| c.active_team_id().map(|s| s.to_string()))
    }

    fn try_sync(&self, rule: &Rule) {
        // Ignore sync errors - syncing is best-effort
        let _ = self.syncer.sync_rule(rule);
    }

    fn try_remove(&self, rule_id: &str) {
        let _ = self.syncer.remove_rule(rule_id);
    }

    fn queue_upsert(&self, rule: &Rule) {
        let Some(queue) = &self.cloud_queue else {
            return;
        };
        let payload = match serde_json::to_string(rule) {
            Ok(p) => p,
            Err(_) => return,
        };

        let _ = queue.enqueue(
            EntityType::Rule,
            &rule.id,
            SyncOperation::Upsert,
            Some(&payload),
        );

        if let Some(team_id) = self.active_team_id()
            && eligible_for_team_rule(rule)
        {
            if let Err(e) = queue.enqueue_for_team(
                EntityType::Rule,
                &rule.id,
                SyncOperation::Upsert,
                Some(&payload),
                &team_id,
            ) {
                tracing::warn!(
                    target: "cas::sync",
                    entity_id = rule.id,
                    team_id = team_id,
                    error = %e,
                    "team enqueue failed for rule"
                );
            }
        }
    }

    fn queue_delete(&self, id: &str) {
        let Some(queue) = &self.cloud_queue else {
            return;
        };
        let _ = queue.enqueue(EntityType::Rule, id, SyncOperation::Delete, None);

        if let Some(team_id) = self.active_team_id() {
            if let Err(e) = queue.enqueue_for_team(
                EntityType::Rule,
                id,
                SyncOperation::Delete,
                None,
                &team_id,
            ) {
                tracing::warn!(
                    target: "cas::sync",
                    entity_id = id,
                    team_id = team_id,
                    error = %e,
                    "team enqueue failed for rule delete"
                );
            }
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
