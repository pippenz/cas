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
    /// Pre-resolved team UUID for dual-enqueue; see
    /// `SyncingEntryStore::team_id` for the protocol. `None` preserves
    /// personal-only behaviour.
    team_id: Option<Arc<str>>,
}

impl SyncingSkillStore {
    /// Create a new syncing skill store (personal queue only).
    pub fn new(inner: Arc<dyn SkillStore>, queue: Arc<SyncQueue>) -> Self {
        Self {
            inner,
            queue,
            team_id: None,
        }
    }

    /// Attach a cloud config for team auto-promotion.
    #[must_use]
    pub fn with_cloud_config(mut self, cloud_config: Arc<CloudConfig>) -> Self {
        self.team_id = cloud_config.active_team_id().map(Arc::from);
        self
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

        if let Some(team_id) = self.team_id.as_deref()
            && eligible_for_team_skill(skill)
        {
            let _ = self.queue.enqueue_for_team(
                EntityType::Skill,
                &skill.id,
                SyncOperation::Upsert,
                Some(&payload),
                team_id,
            );
        }
    }

    fn queue_delete(&self, id: &str) {
        let _ = self
            .queue
            .enqueue(EntityType::Skill, id, SyncOperation::Delete, None);

        // Mirror the upsert path's dual-enqueue. We can't consult the
        // predicate here because we don't have the entity — but deletes
        // are cheap to over-push (the server has no row to touch), and
        // under-pushing would leave stale team rows forever. Trade
        // over-push for correctness. Best-effort matches personal path.
        if let Some(team_id) = self.team_id.as_deref() {
            let _ = self.queue.enqueue_for_team(
                EntityType::Skill,
                id,
                SyncOperation::Delete,
                None,
                team_id,
            );
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

    // ── Dual-enqueue behaviour (cas-82a1) ────────────────────────────────

    use cas_types::Scope;

    const TEST_TEAM: &str = "550e8400-e29b-41d4-a716-446655440000";

    fn create_team_store(team_auto_promote: Option<bool>) -> (TempDir, SyncingSkillStore) {
        let temp = TempDir::new().unwrap();
        let cas_dir = temp.path();
        let inner = SqliteSkillStore::open(cas_dir).unwrap();
        inner.init().unwrap();
        let queue = SyncQueue::open(cas_dir).unwrap();
        queue.init().unwrap();
        let mut cfg = CloudConfig::default();
        cfg.set_team(TEST_TEAM, "test-team");
        cfg.team_auto_promote = team_auto_promote;
        let store = SyncingSkillStore::new(Arc::new(inner), Arc::new(queue))
            .with_cloud_config(Arc::new(cfg));
        (temp, store)
    }

    fn make_skill(id: &str) -> Skill {
        let mut skill = Skill::new(id.to_string(), "Test Skill".to_string());
        skill.description = "A test skill".to_string();
        skill.invocation = "test".to_string();
        skill.skill_type = SkillType::Command;
        skill
    }

    fn queue_counts(queue: &SyncQueue) -> (usize, usize) {
        let personal = queue.pending(100, 5).unwrap().len();
        let team = queue.pending_for_team(TEST_TEAM, 100, 5).unwrap().len();
        (personal, team)
    }

    #[test]
    fn skill_dual_enqueue_when_team_configured_and_project_scope() {
        let (temp, store) = create_team_store(None);
        let queue = SyncQueue::open(temp.path()).unwrap();

        // Skill::new defaults to Global scope; explicitly set Project for
        // the auto-promote path (Skill type has no Preference analogue,
        // so scope is the only predicate input).
        let mut skill = make_skill("p-skill-001");
        skill.scope = Scope::Project;
        store.add(&skill).unwrap();

        let (personal, team) = queue_counts(&queue);
        assert_eq!(personal, 1);
        assert_eq!(team, 1);
    }

    #[test]
    fn skill_personal_only_when_global_scope() {
        let (temp, store) = create_team_store(None);
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut skill = make_skill("g-skill-001");
        skill.scope = Scope::Global;
        store.add(&skill).unwrap();

        let (personal, team) = queue_counts(&queue);
        assert_eq!(personal, 1);
        assert_eq!(team, 0);
    }

    #[test]
    fn skill_personal_only_when_kill_switch_engaged() {
        let (temp, store) = create_team_store(Some(false));
        let queue = SyncQueue::open(temp.path()).unwrap();

        // Project scope so we'd otherwise dual-enqueue; kill-switch must
        // still suppress it.
        let mut skill = make_skill("p-skill-002");
        skill.scope = Scope::Project;
        store.add(&skill).unwrap();

        let (personal, team) = queue_counts(&queue);
        assert_eq!(personal, 1);
        assert_eq!(team, 0);
    }

    #[test]
    fn skill_delete_dual_enqueues_when_team_configured() {
        let (temp, store) = create_team_store(None);
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut skill = make_skill("p-skill-003");
        skill.scope = Scope::Project;
        store.add(&skill).unwrap();
        queue.clear().unwrap();

        store.delete(&skill.id).unwrap();

        let (personal, team) = queue_counts(&queue);
        assert_eq!(personal, 1);
        assert_eq!(team, 1);
    }
}
