use crate::worktree::git::GitError;
use crate::worktree::manager::{WorktreeError, WorktreeManager, WorktreeResult, slugify_title};

impl WorktreeManager {
    /// Create an epic branch from current HEAD
    pub fn create_epic_branch(&self, epic_title: &str) -> WorktreeResult<String> {
        let slug = slugify_title(epic_title);
        let branch_name = format!("epic/{slug}");

        let newly_created = match self.git.create_branch_if_not_exists(&branch_name) {
            Ok(true) => {
                tracing::info!("Created epic branch: {}", branch_name);
                true
            }
            Ok(false) => {
                tracing::info!("Using existing epic branch: {}", branch_name);
                false
            }
            Err(e) => {
                return Err(WorktreeError::Git(e));
            }
        };

        if newly_created {
            if let Err(e) = self.git.push_branch(&branch_name) {
                tracing::warn!("Failed to push epic branch to remote: {}", e);
            } else {
                tracing::info!("Pushed epic branch to remote: {}", branch_name);
            }
        }

        Ok(branch_name)
    }

    /// Merge all worker branches into the epic branch
    pub fn merge_workers_to_epic(
        &self,
        epic_branch: &str,
    ) -> WorktreeResult<Vec<(String, bool, Option<String>)>> {
        let mut results = Vec::new();

        self.git.checkout(epic_branch)?;

        for (name, worktree) in &self.workers {
            let worker_branch = &worktree.branch;

            if !self.git.branch_exists(worker_branch)? {
                results.push((name.clone(), false, Some("Branch not found".to_string())));
                continue;
            }

            match self.git.merge_branch(worker_branch, true) {
                Ok(_commit) => {
                    tracing::info!("Merged {} into {}", worker_branch, epic_branch);
                    results.push((name.clone(), true, None));
                }
                Err(GitError::MergeConflict) => {
                    let _ = std::process::Command::new("git")
                        .args(["merge", "--abort"])
                        .current_dir(&self.repo_root)
                        .output();
                    results.push((
                        name.clone(),
                        false,
                        Some("Merge conflict - manual resolution required".to_string()),
                    ));
                }
                Err(e) => {
                    results.push((name.clone(), false, Some(e.to_string())));
                }
            }
        }

        Ok(results)
    }

    /// Cleanup worker branches after epic completion
    pub fn cleanup_worker_branches(
        &self,
        epic_branch: &str,
        force: bool,
    ) -> WorktreeResult<Vec<String>> {
        let mut deleted = Vec::new();

        for (name, worktree) in &self.workers {
            let worker_branch = &worktree.branch;

            if !self.git.branch_exists(worker_branch)? {
                continue;
            }

            let is_merged = self.is_branch_merged(worker_branch, epic_branch)?;

            if is_merged || force {
                match self.git.delete_branch(worker_branch, force) {
                    Ok(()) => {
                        tracing::info!(
                            "Deleted worker branch: {} (worker: {})",
                            worker_branch,
                            name
                        );
                        deleted.push(worker_branch.clone());
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to delete branch {} (may still be checked out in worktree): {}",
                            worker_branch,
                            e
                        );
                    }
                }
            } else {
                tracing::warn!(
                    "Branch {} not merged into {} - skipping cleanup",
                    worker_branch,
                    epic_branch
                );
            }
        }

        Ok(deleted)
    }

    /// Check if a branch is merged into another branch
    pub(crate) fn is_branch_merged(&self, branch: &str, into: &str) -> WorktreeResult<bool> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["branch", "--merged", into])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Ok(false);
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout.lines().any(|line| {
            let trimmed = line
                .trim()
                .trim_start_matches('*')
                .trim_start_matches('+')
                .trim_start_matches('-')
                .trim();
            trimmed == branch
        }))
    }

    /// Get a list of orphaned epic branches (epic branches with no active workers)
    pub fn list_orphaned_epic_branches(&self) -> WorktreeResult<Vec<String>> {
        let worktrees = self.git.list_worktrees()?;

        let mut epic_branches: Vec<String> = worktrees
            .iter()
            .filter_map(|wt| wt.branch.as_ref())
            .filter(|b| b.starts_with("epic/"))
            .cloned()
            .collect();

        epic_branches.sort();
        epic_branches.dedup();

        Ok(epic_branches)
    }
}
