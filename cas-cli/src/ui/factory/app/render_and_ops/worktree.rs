use crate::ui::factory::app::imports::*;

impl FactoryApp {
    /// Get the project path
    pub fn project_path(&self) -> &std::path::Path {
        &self.project_dir
    }

    /// Create an epic branch based on the configured trunk (not supervisor HEAD)
    ///
    /// Base resolution order (cas-b082): `.cas/config.toml`
    /// `[factory] epic_base_branch` if set, else the repo's detected
    /// default branch. Either way, the base is fetched and resolved
    /// against its remote tip before branching — a stale local base can
    /// never silently seed a new epic branch (BUG-epic-branch-stale-local-base).
    pub fn create_epic_branch(&self, epic_title: &str) -> anyhow::Result<String> {
        use crate::config::Config;
        use crate::worktree::GitOperations;

        let branch_name = epic_branch_name(epic_title);
        let git_ops = GitOperations::new(self.project_dir.clone());
        let trunk = Config::configured_epic_base_branch(&self.project_dir)
            .unwrap_or_else(|| git_ops.detect_default_branch());
        let resolved = git_ops.resolve_fresh_base(&trunk)?;

        if git_ops.create_branch_from(&branch_name, &resolved.branch_ref)? {
            tracing::info!(
                "Created epic branch {} from base '{}' (sha={}, behind={})",
                branch_name,
                resolved.branch_ref,
                &resolved.sha[..resolved.sha.len().min(7)],
                resolved.behind_count,
            );
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
