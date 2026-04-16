//! Syncing skill store wrapper
//!
//! Automatically queues skill changes for cloud sync on add/update/delete.
//! When a team is configured and the skill passes the T1 filter policy,
//! the write is dual-enqueued to both the personal queue and the team queue.

use std::sync::Arc;

use crate::cloud::{CloudConfig, EntityType, SyncOperation, SyncQueue};
use crate::store::share_policy::eligible_for_team_skill;
use crate::store::{Result, SkillStore};
use crate::types::{Skill, SkillStatus};

/// A skill store wrapper that queues changes for cloud sync
pub struct SyncingSkillStore {
    inner: Arc<dyn SkillStore>,
    queue: Arc<SyncQueue>,
    cloud_config: Option<Arc<CloudConfig>>,
}

impl SyncingSkillStore {
    /// Create a new syncing skill store (personal queue only).
    pub fn new(inner: Arc<dyn SkillStore>, queue: Arc<SyncQueue>) -> Self {
        Self {
            inner,
            queue,
            cloud_config: None,
        }
    }

    /// Attach a cloud config for team auto-promotion.
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

    fn queue_upsert(&self, skill: &Skill) {
        let payload = match serde_json::to_string(skill) {
            Ok(p) => p,
            Err(_) => return,
        };

        let _ = self.queue.enqueue(
            EntityType::Skill,
            &skill.id,
            SyncOperation::Upsert,
            Some(&payload),
        );

        if let Some(team_id) = self.active_team_id()
            && eligible_for_team_skill(skill)
        {
            if let Err(e) = self.queue.enqueue_for_team(
                EntityType::Skill,
                &skill.id,
                SyncOperation::Upsert,
                Some(&payload),
                &team_id,
            ) {
                tracing::warn!(
                    target: "cas::sync",
                    entity_id = skill.id,
                    team_id = team_id,
                    error = %e,
                    "team enqueue failed for skill"
                );
            }
        }
    }

    fn queue_delete(&self, id: &str) {
        let _ = self
            .queue
            .enqueue(EntityType::Skill, id, SyncOperation::Delete, None);

        if let Some(team_id) = self.active_team_id() {
            if let Err(e) = self.queue.enqueue_for_team(
                EntityType::Skill,
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
                    "team enqueue failed for skill delete"
                );
            }
        }
    }
}

impl SkillStore for SyncingSkillStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn generate_id(&self) -> Result<String> {
        self.inner.generate_id()
    }

    fn add(&self, skill: &Skill) -> Result<()> {
        self.inner.add(skill)?;
        self.queue_upsert(skill);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Skill> {
        self.inner.get(id)
    }

    fn update(&self, skill: &Skill) -> Result<()> {
        self.inner.update(skill)?;
        self.queue_upsert(skill);
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.inner.delete(id)?;
        self.queue_delete(id);
        Ok(())
    }

    fn list(&self, status: Option<SkillStatus>) -> Result<Vec<Skill>> {
        self.inner.list(status)
    }

    fn list_enabled(&self) -> Result<Vec<Skill>> {
        self.inner.list_enabled()
    }

    fn search(&self, query: &str) -> Result<Vec<Skill>> {
        self.inner.search(query)
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }
}

#[cfg(test)]
mod tests {
    use crate::store::SqliteSkillStore;
    use crate::store::syncing_skill::*;
    use crate::types::SkillType;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SyncingSkillStore) {
        let temp = TempDir::new().unwrap();
        let cas_dir = temp.path();

        let inner = SqliteSkillStore::open(cas_dir).unwrap();
        inner.init().unwrap();

        let queue = SyncQueue::open(cas_dir).unwrap();
        queue.init().unwrap();

        let store = SyncingSkillStore::new(Arc::new(inner), Arc::new(queue));
        (temp, store)
    }

    #[test]
    fn test_add_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut skill = Skill::new("test-skill".to_string(), "Test Skill".to_string());
        skill.description = "A test skill".to_string();
        skill.invocation = "test".to_string();
        skill.skill_type = SkillType::Command;
        store.add(&skill).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entity_type, EntityType::Skill);
        assert_eq!(pending[0].entity_id, skill.id);
        assert_eq!(pending[0].operation, SyncOperation::Upsert);
    }

    #[test]
    fn test_delete_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut skill = Skill::new("test-skill-2".to_string(), "Test Skill".to_string());
        skill.description = "A test skill".to_string();
        skill.invocation = "test".to_string();
        skill.skill_type = SkillType::Command;
        store.add(&skill).unwrap();

        // Clear queue
        queue.clear().unwrap();

        store.delete(&skill.id).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation, SyncOperation::Delete);
    }
}
