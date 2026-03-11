use std::path::Path;
use std::process::Command;

use crate::worktree::git::{GitError, GitOperations, Result};

impl GitOperations {
    /// Create a branch from HEAD if it doesn't exist
    ///
    /// Returns true if the branch was created, false if it already existed.
    pub fn create_branch_if_not_exists(&self, branch: &str) -> Result<bool> {
        if self.branch_exists(branch)? {
            return Ok(false);
        }

        let output = Command::new("git")
            .args(["branch", branch])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(true)
    }

    /// Push a branch to origin
    ///
    /// Pushes the specified branch to the 'origin' remote. If the branch doesn't exist
    /// on origin yet, it will be created. Uses -u to set up tracking.
    pub fn push_branch(&self, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["push", "-u", "origin", branch])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!("Failed to push branch {}: {}", branch, stderr);
            return Err(GitError::CommandFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Mark .claude/, CLAUDE.md, and .mcp.json as skip-worktree in a worktree
    ///
    /// This prevents workers from accidentally staging and committing CAS-synced
    /// changes to these tracked config files. The files remain in the worktree
    /// (Claude Code works normally) but git ignores local modifications.
    pub fn mark_config_skip_worktree(&self, worktree_path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["ls-files", ".claude/", "CLAUDE.md", ".mcp.json"])
            .current_dir(worktree_path)
            .output()?;

        if !output.status.success() || output.stdout.is_empty() {
            return Ok(());
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let files: Vec<&str> = stdout.lines().filter(|line| !line.is_empty()).collect();

        if files.is_empty() {
            return Ok(());
        }

        let mut args = vec!["update-index", "--skip-worktree"];
        args.extend(files.iter());

        let update_output = Command::new("git")
            .args(&args)
            .current_dir(worktree_path)
            .output()?;

        if !update_output.status.success() {
            tracing::warn!(
                "Failed to set skip-worktree on config files in {}: {}",
                worktree_path.display(),
                String::from_utf8_lossy(&update_output.stderr)
            );
        } else {
            tracing::info!(
                "Marked {} config files as skip-worktree in {}",
                files.len(),
                worktree_path.display()
            );
        }

        Ok(())
    }

    /// Reset a worktree to a specific branch/ref (hard reset)
    ///
    /// This is used to sync a worker's worktree to the latest epic branch.
    pub fn reset_hard_in_dir(&self, dir: &Path, target: &str) -> Result<()> {
        let fetch_output = Command::new("git")
            .args(["fetch", "--all"])
            .current_dir(dir)
            .output()?;

        if !fetch_output.status.success() {
            eprintln!(
                "[CAS] Warning: git fetch failed: {}",
                String::from_utf8_lossy(&fetch_output.stderr)
            );
        }

        let output = Command::new("git")
            .args(["reset", "--hard", target])
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(())
    }
}
