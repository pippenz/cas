use std::path::Path;
use std::process::Command;

use crate::worktree::git::{GitError, GitOperations, ResolvedBase, Result};

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

    /// Create a branch from a specific base ref if it doesn't exist.
    ///
    /// Unlike `create_branch_if_not_exists`, this uses an explicit start point
    /// rather than the current HEAD. Pass the configured trunk (e.g. "main") so
    /// epic and worker branches are always anchored to the correct base.
    ///
    /// Returns true if the branch was created, false if it already existed.
    pub fn create_branch_from(&self, branch: &str, base: &str) -> Result<bool> {
        if self.branch_exists(branch)? {
            return Ok(false);
        }

        let output = Command::new("git")
            .args(["branch", branch, base])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(true)
    }

    /// Fetch a single branch from `origin`.
    ///
    /// Best-effort by design: callers should treat an `Err` as "could not
    /// verify freshness" (offline, no remote configured, remote branch
    /// doesn't exist yet) rather than a hard failure — local-only repos
    /// must keep working.
    pub fn fetch_branch(&self, branch: &str) -> Result<()> {
        let output = Command::new("git")
            .args(["fetch", "origin", branch])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(())
    }

    /// Count commits reachable from `to` but not from `from`
    /// (`git rev-list --count from..to`) — i.e. how far `from` is behind `to`.
    pub fn commits_behind(&self, from: &str, to: &str) -> Result<u32> {
        let output = Command::new("git")
            .args(["rev-list", "--count", &format!("{from}..{to}")])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u32>()
            .map_err(|e| GitError::CommandFailed(format!("Failed to parse rev-list count: {e}")))
    }

    /// Resolve a branch-creation base against its remote tip (cas-b082 —
    /// BUG-epic-branch-stale-local-base).
    ///
    /// Fetches `origin/<base>` and, when reachable, always branches from
    /// the fetched remote tip rather than the local `<base>` ref — so a
    /// stale local base (the observed failure: local 30 commits behind
    /// origin) can never silently seed a new epic/worker branch. When the
    /// local base was behind, logs a loud warning with the exact
    /// behind-count before returning. Falls back to the local `<base>`
    /// ref when there is no remote, the fetch fails (offline), or
    /// `origin/<base>` doesn't exist — local-only repos keep working
    /// unchanged.
    pub fn resolve_fresh_base(&self, base: &str) -> Result<ResolvedBase> {
        let remote_ref = format!("origin/{base}");
        let fetch_ok = self.fetch_branch(base).is_ok();

        if fetch_ok && self.branch_exists(&remote_ref).unwrap_or(false) {
            let behind_count = if self.branch_exists(base).unwrap_or(false) {
                self.commits_behind(base, &remote_ref).unwrap_or(0)
            } else {
                0
            };

            if behind_count > 0 {
                tracing::warn!(
                    "Local '{}' is {} commit(s) behind 'origin/{}' — basing the new branch \
                     on the fetched remote tip instead of the stale local ref",
                    base,
                    behind_count,
                    base
                );
            }

            let sha = self.ref_sha(&remote_ref).unwrap_or_default();
            return Ok(ResolvedBase {
                branch_ref: remote_ref,
                sha,
                behind_count,
                used_remote: true,
            });
        }

        let sha = self.ref_sha(base).unwrap_or_default();
        Ok(ResolvedBase {
            branch_ref: base.to_string(),
            sha,
            behind_count: 0,
            used_remote: false,
        })
    }

    /// Resolve the full SHA of a ref (branch name, "HEAD", etc.).
    ///
    /// Returns a 40-character hex SHA, or a GitError if the ref doesn't exist.
    pub fn ref_sha(&self, ref_name: &str) -> Result<String> {
        let output = Command::new("git")
            .args(["rev-parse", ref_name])
            .current_dir(&self.repo_root)
            .output()?;

        if !output.status.success() {
            return Err(GitError::CommandFailed(
                String::from_utf8_lossy(&output.stderr).to_string(),
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
