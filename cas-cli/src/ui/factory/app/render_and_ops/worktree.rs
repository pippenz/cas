use crate::ui::factory::app::imports::*;

impl FactoryApp {
    /// Get the project path
    pub fn project_path(&self) -> &std::path::Path {
        &self.project_dir
    }

    /// Create an epic branch from the current branch
    pub fn create_epic_branch(&self, epic_title: &str) -> anyhow::Result<String> {
        use crate::worktree::GitOperations;

        let branch_name = epic_branch_name(epic_title);
        let git_ops = GitOperations::new(self.project_dir.clone());

        if git_ops.create_branch_if_not_exists(&branch_name)? {
            tracing::info!("Created epic branch: {}", branch_name);
        } else {
            tracing::info!("Epic branch already exists: {}", branch_name);
        }

        Ok(branch_name)
    }

    /// Merge all worker branches to the epic branch
    pub fn merge_workers_to_epic(&self) -> anyhow::Result<Vec<(String, bool, Option<String>)>> {
        let epic_branch = self
            .epic_branch
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No epic branch active"))?;

        if let Some(manager) = &self.worktree_manager {
            let results = manager.merge_workers_to_epic(epic_branch)?;
            Ok(results)
        } else {
            // No worktrees - nothing to merge
            Ok(Vec::new())
        }
    }

    /// Cleanup worker branches after epic completion
    ///
    /// Deletes all worker branches that have been merged into the epic branch.
    pub fn cleanup_worker_branches(&self, force: bool) -> anyhow::Result<Vec<String>> {
        let epic_branch = self
            .epic_branch
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No epic branch active"))?;

        if let Some(manager) = &self.worktree_manager {
            let deleted = manager.cleanup_worker_branches(epic_branch, force)?;
            Ok(deleted)
        } else {
            // No worktrees - nothing to cleanup
            Ok(Vec::new())
        }
    }
}
