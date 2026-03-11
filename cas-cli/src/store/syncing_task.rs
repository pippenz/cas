//! Syncing task store wrapper
//!
//! Automatically queues task changes for cloud sync on add/update/delete.

use std::sync::Arc;

use crate::cloud::{EntityType, SyncOperation, SyncQueue};
use crate::store::{Result, TaskStore};
use crate::types::{Dependency, DependencyType, Task, TaskStatus};

/// A task store wrapper that queues changes for cloud sync
pub struct SyncingTaskStore {
    inner: Arc<dyn TaskStore>,
    queue: Arc<SyncQueue>,
}

impl SyncingTaskStore {
    /// Create a new syncing task store
    pub fn new(inner: Arc<dyn TaskStore>, queue: Arc<SyncQueue>) -> Self {
        Self { inner, queue }
    }

    fn queue_upsert(&self, task: &Task) {
        // Best-effort queuing - don't fail the operation if queue fails
        if let Ok(payload) = serde_json::to_string(task) {
            let _ = self.queue.enqueue(
                EntityType::Task,
                &task.id,
                SyncOperation::Upsert,
                Some(&payload),
            );
        }
    }

    fn queue_delete(&self, id: &str) {
        let _ = self
            .queue
            .enqueue(EntityType::Task, id, SyncOperation::Delete, None);
    }
}

impl TaskStore for SyncingTaskStore {
    fn init(&self) -> Result<()> {
        self.inner.init()
    }

    fn generate_id(&self) -> Result<String> {
        self.inner.generate_id()
    }

    fn add(&self, task: &Task) -> Result<()> {
        self.inner.add(task)?;
        self.queue_upsert(task);
        Ok(())
    }

    fn create_atomic(
        &self,
        task: &Task,
        blocked_by: &[String],
        epic_id: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<()> {
        self.inner
            .create_atomic(task, blocked_by, epic_id, created_by)?;
        self.queue_upsert(task);
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Task> {
        self.inner.get(id)
    }

    fn update(&self, task: &Task) -> Result<()> {
        self.inner.update(task)?;
        self.queue_upsert(task);
        Ok(())
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.inner.delete(id)?;
        self.queue_delete(id);
        Ok(())
    }

    fn list(&self, status: Option<TaskStatus>) -> Result<Vec<Task>> {
        self.inner.list(status)
    }

    fn list_ready(&self) -> Result<Vec<Task>> {
        self.inner.list_ready()
    }

    fn list_blocked(&self) -> Result<Vec<(Task, Vec<Task>)>> {
        self.inner.list_blocked()
    }

    fn close(&self) -> Result<()> {
        self.inner.close()
    }

    // Dependency operations - don't sync these as they're derived from task relationships
    fn add_dependency(&self, dep: &Dependency) -> Result<()> {
        self.inner.add_dependency(dep)
    }

    fn remove_dependency(&self, from_id: &str, to_id: &str) -> Result<()> {
        self.inner.remove_dependency(from_id, to_id)
    }

    fn get_dependencies(&self, task_id: &str) -> Result<Vec<Dependency>> {
        self.inner.get_dependencies(task_id)
    }

    fn get_dependents(&self, task_id: &str) -> Result<Vec<Dependency>> {
        self.inner.get_dependents(task_id)
    }

    fn get_blockers(&self, task_id: &str) -> Result<Vec<Task>> {
        self.inner.get_blockers(task_id)
    }

    fn would_create_cycle(&self, from_id: &str, to_id: &str) -> Result<bool> {
        self.inner.would_create_cycle(from_id, to_id)
    }

    fn list_dependencies(&self, dep_type: Option<DependencyType>) -> Result<Vec<Dependency>> {
        self.inner.list_dependencies(dep_type)
    }

    fn get_subtasks(&self, parent_id: &str) -> Result<Vec<Task>> {
        self.inner.get_subtasks(parent_id)
    }

    fn get_sibling_notes(
        &self,
        epic_id: &str,
        exclude_task_id: &str,
    ) -> Result<Vec<(String, String, String)>> {
        self.inner.get_sibling_notes(epic_id, exclude_task_id)
    }

    fn get_parent_epic(&self, task_id: &str) -> Result<Option<Task>> {
        self.inner.get_parent_epic(task_id)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::SqliteTaskStore;
    use crate::store::syncing_task::*;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SyncingTaskStore) {
        let temp = TempDir::new().unwrap();
        let cas_dir = temp.path();

        let inner = SqliteTaskStore::open(cas_dir).unwrap();
        inner.init().unwrap();

        let queue = SyncQueue::open(cas_dir).unwrap();
        queue.init().unwrap();

        let store = SyncingTaskStore::new(Arc::new(inner), Arc::new(queue));
        (temp, store)
    }

    #[test]
    fn test_add_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let task = Task::new("task-001".to_string(), "Test task".to_string());
        store.add(&task).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].entity_type, EntityType::Task);
        assert_eq!(pending[0].entity_id, task.id);
        assert_eq!(pending[0].operation, SyncOperation::Upsert);
    }

    #[test]
    fn test_update_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let mut task = Task::new("task-002".to_string(), "Test task".to_string());
        store.add(&task).unwrap();

        // Clear queue
        queue.clear().unwrap();

        task.title = "Updated title".to_string();
        store.update(&task).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert!(
            pending[0]
                .payload
                .as_ref()
                .unwrap()
                .contains("Updated title")
        );
    }

    #[test]
    fn test_delete_queues_sync() {
        let (temp, store) = create_test_store();
        let queue = SyncQueue::open(temp.path()).unwrap();

        let task = Task::new("task-003".to_string(), "Test task".to_string());
        store.add(&task).unwrap();

        // Clear queue
        queue.clear().unwrap();

        store.delete(&task.id).unwrap();

        let pending = queue.pending(10, 5).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].operation, SyncOperation::Delete);
    }
}
