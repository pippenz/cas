use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::Worktree;
use crate::worktree::git::GitOperations;
use crate::worktree::manager::{WorktreeError, WorktreeManager, WorktreeResult, symlink_project_config};

impl WorktreeManager {
    /// Calculate the worktree path for a factory worker
    pub fn worktree_path_for_worker(&self, worker_name: &str) -> PathBuf {
        self.worktree_root().join(worker_name)
    }

    /// Calculate the branch name for a factory worker
    pub fn branch_name_for_worker(&self, worker_name: &str) -> String {
        format!("factory/{worker_name}")
    }

    /// Check if a worktree exists for a worker
    pub fn worktree_exists_for_worker(&self, worker_name: &str) -> bool {
        let path = self.worktree_path_for_worker(worker_name);
        path.exists()
    }

    /// Create a worktree for a factory worker
    pub fn create_for_worker(&mut self, worker_name: &str) -> WorktreeResult<Worktree> {
        if self.context.is_worktree {
            return Err(WorktreeError::AlreadyInWorktree);
        }

        let worktree_path = self.worktree_path_for_worker(worker_name);
        let branch_name = self.branch_name_for_worker(worker_name);
        let parent_branch = self.git.current_branch()?;

        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        self.git
            .create_worktree(&worktree_path, &branch_name, Some(&parent_branch))?;

        let _ = self.git.mark_config_skip_worktree(&worktree_path);
        symlink_project_config(&self.repo_root, &worktree_path);

        let worktree = Worktree::new(
            Worktree::generate_id(),
            branch_name,
            parent_branch,
            worktree_path,
        );

        self.workers
            .insert(worker_name.to_string(), worktree.clone());

        Ok(worktree)
    }

    /// Create a worktree for a factory worker from a specific parent branch
    pub fn create_for_worker_from(
        &mut self,
        worker_name: &str,
        parent_branch: &str,
    ) -> WorktreeResult<Worktree> {
        if self.context.is_worktree {
            return Err(WorktreeError::AlreadyInWorktree);
        }

        let worktree_path = self.worktree_path_for_worker(worker_name);
        let branch_name = self.branch_name_for_worker(worker_name);

        if let Some(parent) = worktree_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        self.git
            .create_worktree(&worktree_path, &branch_name, Some(parent_branch))?;

        let _ = self.git.mark_config_skip_worktree(&worktree_path);
        symlink_project_config(&self.repo_root, &worktree_path);

        let worktree = Worktree::new(
            Worktree::generate_id(),
            branch_name,
            parent_branch.to_string(),
            worktree_path,
        );

        self.workers
            .insert(worker_name.to_string(), worktree.clone());

        Ok(worktree)
    }

    /// Ensure a worktree exists for a worker (idempotent)
    pub fn ensure_worker_worktree(&mut self, worker_name: &str) -> WorktreeResult<&Worktree> {
        if self.workers.contains_key(worker_name) {
            return self.worker_ref(worker_name);
        }

        let worktree_path = self.worktree_path_for_worker(worker_name);
        if worktree_path.exists() {
            let _ = self.git.mark_config_skip_worktree(&worktree_path);
            let _ = self.git.init_submodules(&worktree_path);
            symlink_project_config(&self.repo_root, &worktree_path);

            let branch_name = self.branch_name_for_worker(worker_name);
            let parent_branch = self
                .context
                .branch
                .clone()
                .unwrap_or_else(|| self.git.detect_default_branch());

            let worktree = Worktree::new(
                Worktree::generate_id(),
                branch_name,
                parent_branch,
                worktree_path,
            );
            self.workers.insert(worker_name.to_string(), worktree);
            return self.worker_ref(worker_name);
        }

        self.create_for_worker(worker_name)?;
        self.worker_ref(worker_name)
    }

    /// Ensure a worktree exists for a worker from a specific parent branch (idempotent)
    pub fn ensure_worker_worktree_from(
        &mut self,
        worker_name: &str,
        parent_branch: &str,
    ) -> WorktreeResult<&Worktree> {
        if self.workers.contains_key(worker_name) {
            return self.worker_ref(worker_name);
        }

        let worktree_path = self.worktree_path_for_worker(worker_name);
        if worktree_path.exists() {
            let _ = self.git.mark_config_skip_worktree(&worktree_path);
            let _ = self.git.init_submodules(&worktree_path);
            symlink_project_config(&self.repo_root, &worktree_path);

            let branch_name = self.branch_name_for_worker(worker_name);

            let worktree = Worktree::new(
                Worktree::generate_id(),
                branch_name,
                parent_branch.to_string(),
                worktree_path,
            );
            self.workers.insert(worker_name.to_string(), worktree);
            return self.worker_ref(worker_name);
        }

        self.create_for_worker_from(worker_name, parent_branch)?;
        self.worker_ref(worker_name)
    }

    /// Get worker working directories for MuxConfig
    pub fn worker_cwds(&self) -> HashMap<String, PathBuf> {
        self.workers
            .iter()
            .filter(|(_, wt)| wt.path.exists())
            .map(|(name, wt)| (name.clone(), wt.path.clone()))
            .collect()
    }

    /// Get a worker's worktree if it exists
    pub fn get_worker(&self, worker_name: &str) -> Option<&Worktree> {
        self.workers.get(worker_name)
    }

    /// Register a worktree that was created externally.
    pub fn register_worktree(&mut self, worker_name: &str, worktree: Worktree) {
        self.workers.insert(worker_name.to_string(), worktree);
    }

    /// Get a reference to the git operations wrapper
    pub fn git(&self) -> &GitOperations {
        &self.git
    }

    /// Cleanup worker worktrees
    pub fn cleanup_workers(&mut self, force: bool) -> WorktreeResult<Vec<String>> {
        let mut cleaned = Vec::new();

        let worker_names: Vec<String> = self.workers.keys().cloned().collect();

        for name in worker_names {
            if let Some(mut worktree) = self.workers.remove(&name) {
                if !force && worktree.path.exists() {
                    if let Ok(true) = self.git.has_uncommitted_changes(&worktree.path) {
                        self.workers.insert(name, worktree);
                        continue;
                    }
                }

                if worktree.path.exists() {
                    let _ = self.git.remove_worktree(&worktree.path, force);
                }

                let _ = self.git.delete_branch(&worktree.branch, true);

                worktree.mark_abandoned();
                worktree.mark_removed();

                cleaned.push(name);
            }
        }

        Ok(cleaned)
    }

    /// Remove a single worker's worktree
    pub fn remove_worker(&mut self, worker_name: &str, force: bool) -> WorktreeResult<()> {
        if let Some(mut worktree) = self.workers.remove(worker_name) {
            if !force
                && worktree.path.exists()
                && self.git.has_uncommitted_changes(&worktree.path)?
            {
                self.workers.insert(worker_name.to_string(), worktree);
                return Err(WorktreeError::UncommittedChanges);
            }

            if worktree.path.exists() {
                self.git.remove_worktree(&worktree.path, force)?;
            }

            let _ = self.git.delete_branch(&worktree.branch, true);

            worktree.mark_abandoned();
            worktree.mark_removed();
        }

        Ok(())
    }
}
