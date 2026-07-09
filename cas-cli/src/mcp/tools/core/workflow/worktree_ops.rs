use crate::mcp::tools::core::imports::*;

/// Resolve the on-disk path of a System B (`spawn_workers isolate=true`)
/// worktree for `assignee`, if one exists.
///
/// `spawn_workers isolate=true` places every worker at the fixed
/// convention `<cas_root>/worktrees/<assignee>` on branch
/// `factory/<assignee>` — never registered in the `WorktreeStore`
/// (System A), so `worktree_merge` needs this as a fallback lookup
/// (cas-1d11) when the System A store misses. Returns `None` when the
/// path doesn't exist or isn't a git worktree (no `.git` entry) — an
/// unknowable worktree is not treated as a false positive.
fn system_b_worktree_path(cas_root: &std::path::Path, assignee: &str) -> Option<std::path::PathBuf> {
    let path = cas_root.join("worktrees").join(assignee);
    if path.join(".git").exists() {
        Some(path)
    } else {
        None
    }
}

impl CasCore {
    pub async fn worktree_create(&self, epic_id: &str) -> Result<CallToolResult, McpError> {
        use crate::config::Config;
        use crate::store::{open_task_store, open_worktree_store};
        use crate::worktree::{WorktreeConfig, WorktreeManager};

        let cas_root = self.cas_root.clone();
        let config = Config::load(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to load config: {e}")),
            data: None,
        })?;
        let wt_config = config.worktrees();

        if !wt_config.enabled {
            return Ok(Self::success(
                "Worktrees are not enabled. Enable in .cas/config.toml:\n  worktrees:\n    enabled: true",
            ));
        }

        // Verify epic exists
        let task_store = open_task_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open task store: {e}")),
            data: None,
        })?;
        let epic = task_store.get(epic_id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Epic/task not found: {e}")),
            data: None,
        })?;

        let cwd = std::env::current_dir().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to get cwd: {e}")),
            data: None,
        })?;

        let manager_config = WorktreeConfig {
            enabled: wt_config.enabled,
            base_path: wt_config.base_path.clone(),
            branch_prefix: wt_config.branch_prefix.clone(),
            auto_merge: wt_config.auto_merge,
            cleanup_on_close: wt_config.cleanup_on_close,
            promote_entries_on_merge: wt_config.promote_entries_on_merge,
        };

        let manager = WorktreeManager::new(&cwd, manager_config).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to create worktree manager: {e}")),
            data: None,
        })?;

        // Get agent ID from registered agent (flatten Option<&Option<String>>)
        let agent_id = self.agent_id.get().and_then(|o| o.as_ref());

        let worktree = manager
            .create_for_epic(epic_id, agent_id.map(|s| s.as_str()))
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to create worktree: {e}")),
                data: None,
            })?;

        // Store the worktree record
        let worktree_store = open_worktree_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open worktree store: {e}")),
            data: None,
        })?;
        worktree_store.add(&worktree).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to store worktree: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Created worktree for epic {}:\n  ID: {}\n  Branch: {}\n  Path: {}\n\ncd {} to work in the isolated worktree",
            epic.title,
            worktree.id,
            worktree.branch,
            worktree.path.display(),
            worktree.path.display()
        )))
    }

    /// List worktrees
    pub async fn worktree_list(
        &self,
        all: bool,
        status_filter: Option<&str>,
        orphans_only: bool,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_task_store, open_worktree_store};
        use crate::types::{AgentStatus, TaskStatus, Worktree, WorktreeStatus};
        use crate::worktree::GitOperations;
        use std::collections::HashSet;

        let cas_root = self.cas_root.clone();
        let worktree_store = open_worktree_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open worktree store: {e}")),
            data: None,
        })?;
        let task_store = open_task_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open task store: {e}")),
            data: None,
        })?;
        let agent_store = open_agent_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open agent store: {e}")),
            data: None,
        })?;

        let mut worktrees = if let Some(status_str) = status_filter {
            let status: WorktreeStatus = status_str.parse().map_err(|_| McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Invalid status: {status_str}")),
                data: None,
            })?;
            worktree_store
                .list_by_status(status)
                .map_err(|e| McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!("Failed to list worktrees: {e}")),
                    data: None,
                })?
        } else if all {
            worktree_store.list().map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to list worktrees: {e}")),
                data: None,
            })?
        } else {
            worktree_store.list_active().map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to list worktrees: {e}")),
                data: None,
            })?
        };

        // Get branches already in SQLite for deduplication
        let tracked_branches: HashSet<String> =
            worktrees.iter().map(|wt| wt.branch.clone()).collect();

        // Also include factory (System B) worktrees from git that are not yet
        // tracked in the SQLite store.
        //
        // We scope the scan to paths under `<cas_root>/worktrees/` so that
        // the main checkout (and any unrelated user worktrees) are excluded.
        // Factory workers are always placed at `.cas/worktrees/<name>` by
        // `spawn_workers isolate=true`, so this filter is both safe and precise.
        let factory_worktrees_base = cas_root.join("worktrees");
        if let Some(repo_root) = cas_root.parent() {
            if let Ok(git_ops) = GitOperations::detect_repo_root(repo_root).map(GitOperations::new)
            {
                if let Ok(git_worktrees) = git_ops.list_worktrees() {
                    for git_wt in git_worktrees {
                        // Only include worktrees that live under .cas/worktrees/
                        // (factory / System B) and are not already in the store.
                        if !git_wt.path.starts_with(&factory_worktrees_base) {
                            continue;
                        }
                        let branch = git_wt.branch.clone().unwrap_or_default();
                        if !branch.is_empty() && !tracked_branches.contains(&branch) {
                            // Create a transient Worktree entry for display.
                            // The `git:` id prefix is used downstream to identify
                            // factory worktrees and render the `[factory]` label.
                            worktrees.push(Worktree::new(
                                format!("git:{branch}"),
                                branch,
                                "unknown".to_string(),
                                git_wt.path.clone(),
                            ));
                        }
                    }
                }
            }
        }

        // Filter orphans if requested
        let worktrees: Vec<_> = if orphans_only {
            worktrees
                .into_iter()
                .filter(|wt| {
                    if wt.status != WorktreeStatus::Active {
                        return false;
                    }
                    if !wt.path.exists() {
                        return true;
                    }
                    if let Some(ref epic_id) = wt.epic_id {
                        if let Ok(epic) = task_store.get(epic_id) {
                            if matches!(epic.status, TaskStatus::Closed) {
                                return true;
                            }
                        }
                    }
                    if let Some(ref agent_id) = wt.created_by_agent {
                        if let Ok(agent) = agent_store.get(agent_id) {
                            if matches!(agent.status, AgentStatus::Stale | AgentStatus::Shutdown) {
                                return true;
                            }
                        }
                    }
                    false
                })
                .collect()
        } else {
            worktrees
        };

        if worktrees.is_empty() {
            return Ok(Self::success("No worktrees found."));
        }

        let mut output = format!("WORKTREES ({})\n\n", worktrees.len());
        for wt in &worktrees {
            let status_icon = match wt.status {
                WorktreeStatus::Active => "🟢",
                WorktreeStatus::Merged => "✅",
                WorktreeStatus::Abandoned => "⚠️",
                WorktreeStatus::Conflict => "❌",
                WorktreeStatus::Removed => "🗑️",
            };
            let path_status = if wt.path.exists() { "" } else { " (missing)" };
            // Factory worktrees have IDs starting with "git:"
            let type_indicator = if wt.id.starts_with("git:") {
                " [factory]"
            } else {
                ""
            };
            output.push_str(&format!(
                "{} {} - {} {}{}{}\n   Epic: {}\n\n",
                status_icon,
                wt.id,
                wt.branch,
                wt.status,
                path_status,
                type_indicator,
                wt.epic_id.as_deref().unwrap_or("-")
            ));
        }

        Ok(Self::success(output))
    }

    /// Show worktree details
    pub async fn worktree_show(&self, id: &str) -> Result<CallToolResult, McpError> {
        use crate::store::open_worktree_store;

        let cas_root = self.cas_root.clone();
        let worktree_store = open_worktree_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open worktree store: {e}")),
            data: None,
        })?;

        let worktree = match worktree_store.get(id) {
            Ok(wt) => wt,
            Err(_) => worktree_store
                .get_by_branch(id)
                .map_err(|e| McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!("Failed to get worktree: {e}")),
                    data: None,
                })?
                .ok_or_else(|| McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!("Worktree not found: {id}")),
                    data: None,
                })?,
        };

        let path_exists = worktree.path.exists();
        Ok(Self::success(format!(
            "Worktree: {}\n\nBranch: {}\nParent: {}\nStatus: {}\nPath: {} {}\nEpic: {}\nCreated by: {}\nCreated: {}",
            worktree.id,
            worktree.branch,
            worktree.parent_branch,
            worktree.status,
            worktree.path.display(),
            if path_exists { "" } else { "(missing)" },
            worktree.epic_id.as_deref().unwrap_or("-"),
            worktree.created_by_agent.as_deref().unwrap_or("-"),
            worktree.created_at.format("%Y-%m-%d %H:%M UTC")
        )))
    }

    /// Cleanup orphaned worktrees
    pub async fn worktree_cleanup(
        &self,
        dry_run: bool,
        force: bool,
    ) -> Result<CallToolResult, McpError> {
        use crate::config::Config;
        use crate::store::{open_agent_store, open_task_store, open_worktree_store};
        use crate::types::{AgentStatus, TaskStatus};
        use crate::worktree::{WorktreeConfig, WorktreeManager};

        let cas_root = self.cas_root.clone();
        let config = Config::load(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to load config: {e}")),
            data: None,
        })?;
        let wt_config = config.worktrees();

        let worktree_store = open_worktree_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open worktree store: {e}")),
            data: None,
        })?;
        let task_store = open_task_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open task store: {e}")),
            data: None,
        })?;
        let agent_store = open_agent_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open agent store: {e}")),
            data: None,
        })?;

        let active_worktrees = worktree_store.list_active().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to list worktrees: {e}")),
            data: None,
        })?;

        // Find orphans
        let orphans: Vec<_> = active_worktrees
            .into_iter()
            .filter(|wt| {
                if !wt.path.exists() {
                    return true;
                }
                if let Some(ref epic_id) = wt.epic_id {
                    if let Ok(epic) = task_store.get(epic_id) {
                        if matches!(epic.status, TaskStatus::Closed) {
                            return true;
                        }
                    }
                }
                if let Some(ref agent_id) = wt.created_by_agent {
                    if let Ok(agent) = agent_store.get(agent_id) {
                        if matches!(agent.status, AgentStatus::Stale | AgentStatus::Shutdown) {
                            return true;
                        }
                    }
                }
                false
            })
            .collect();

        if orphans.is_empty() {
            return Ok(Self::success("No orphaned worktrees to clean up."));
        }

        if dry_run {
            let mut output = format!("Would clean up {} worktree(s):\n\n", orphans.len());
            for wt in &orphans {
                output.push_str(&format!("  {} - {}\n", wt.id, wt.branch));
            }
            output.push_str("\nRun with dry_run=false to actually clean up.");
            return Ok(Self::success(output));
        }

        let cwd = std::env::current_dir().unwrap_or_default();
        let manager_config = WorktreeConfig {
            enabled: wt_config.enabled,
            base_path: wt_config.base_path.clone(),
            branch_prefix: wt_config.branch_prefix.clone(),
            auto_merge: wt_config.auto_merge,
            cleanup_on_close: wt_config.cleanup_on_close,
            promote_entries_on_merge: wt_config.promote_entries_on_merge,
        };

        let mut cleaned = 0;
        let mut errors = Vec::new();

        for mut wt in orphans {
            if wt.path.exists() {
                if let Ok(manager) = WorktreeManager::new(&cwd, manager_config.clone()) {
                    if manager.abandon(&mut wt, force).is_ok() {
                        wt.mark_abandoned();
                        wt.mark_removed();
                        let _ = worktree_store.update(&wt);
                        cleaned += 1;
                        continue;
                    }
                }
            }
            // Just mark in store if physical cleanup failed
            wt.mark_abandoned();
            wt.mark_removed();
            if worktree_store.update(&wt).is_ok() {
                cleaned += 1;
            } else {
                errors.push(wt.id.clone());
            }
        }

        if errors.is_empty() {
            Ok(Self::success(format!("Cleaned up {cleaned} worktree(s).")))
        } else {
            Ok(Self::success(format!(
                "Cleaned up {} worktree(s), {} error(s): {}",
                cleaned,
                errors.len(),
                errors.join(", ")
            )))
        }
    }

    /// Merge worktree back to parent
    ///
    /// Resolves `id` against System A first (the `WorktreeStore`-tracked,
    /// `worktrees.enabled`-gated worktrees created by `worktree_create`).
    /// When that lookup misses, falls back to System B — the
    /// `<cas_root>/worktrees/<assignee>` / `factory/<assignee>` convention
    /// used by `spawn_workers isolate=true`, which is never registered in
    /// the store and doesn't check `worktrees.enabled` at all (cas-1d11).
    /// Without this fallback, spawn happily created isolated worktrees
    /// while the only supervisor-callable merge action refused every one
    /// of them — forcing a manual `git worktree add` + merge + push that
    /// bypassed factory tracking/lease/cleanup entirely.
    pub async fn worktree_merge(&self, id: &str, force: bool) -> Result<CallToolResult, McpError> {
        use crate::config::Config;
        use crate::store::open_worktree_store;
        use crate::worktree::{WorktreeConfig, WorktreeManager};

        let cas_root = self.cas_root.clone();
        let config = Config::load(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to load config: {e}")),
            data: None,
        })?;
        let wt_config = config.worktrees();

        let worktree_store = open_worktree_store(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open worktree store: {e}")),
            data: None,
        })?;

        let cwd = std::env::current_dir().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to get cwd: {e}")),
            data: None,
        })?;

        let manager_config = WorktreeConfig {
            enabled: wt_config.enabled,
            base_path: wt_config.base_path.clone(),
            branch_prefix: wt_config.branch_prefix.clone(),
            auto_merge: true, // Force merge for this operation
            cleanup_on_close: wt_config.cleanup_on_close,
            promote_entries_on_merge: wt_config.promote_entries_on_merge,
        };

        let manager = WorktreeManager::new(&cwd, manager_config).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to create worktree manager: {e}")),
            data: None,
        })?;

        let system_a = match worktree_store.get(id) {
            Ok(wt) => Some(wt),
            Err(_) => worktree_store.get_by_branch(id).map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to get worktree: {e}")),
                data: None,
            })?,
        };

        let (mut worktree, is_system_b) = match system_a {
            Some(wt) => (wt, false),
            None => {
                let assignee = id.strip_prefix("factory/").unwrap_or(id);
                let path = system_b_worktree_path(&cas_root, assignee).ok_or_else(|| McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!(
                        "Worktree not found: {id} (checked System A worktree store and \
                         the System B convention {}/worktrees/{assignee})",
                        cas_root.display()
                    )),
                    data: None,
                })?;
                let parent_branch = Config::configured_epic_base_branch(&cwd)
                    .unwrap_or_else(|| manager.git().detect_default_branch());
                (
                    crate::types::Worktree::new(
                        format!("system-b-{assignee}"),
                        format!("factory/{assignee}"),
                        parent_branch,
                        path,
                    ),
                    true,
                )
            }
        };

        let merge_commit = manager
            .merge_and_cleanup(&mut worktree, force)
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to merge worktree: {e}")),
                data: None,
            })?;

        // Update store — System B worktrees were never registered there, so
        // there's no row to update (and nothing worth persisting: the
        // git-level merge + cleanup above already happened).
        if !is_system_b {
            worktree_store.update(&worktree).map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to update worktree: {e}")),
                data: None,
            })?;
        }

        // Promote entries if configured
        if wt_config.promote_entries_on_merge {
            if let Ok(count) = self.promote_branch_entries(&worktree.branch) {
                if count > 0 {
                    return Ok(Self::success(format!(
                        "Merged worktree {} to {}. Commit: {}\nPromoted {} entries from branch scope.",
                        worktree.id,
                        worktree.parent_branch,
                        merge_commit.as_deref().unwrap_or("none"),
                        count
                    )));
                }
            }
        }

        Ok(Self::success(format!(
            "Merged worktree {} to {}. Commit: {}",
            worktree.id,
            worktree.parent_branch,
            merge_commit.as_deref().unwrap_or("none")
        )))
    }

    /// Get current worktree status
    pub async fn worktree_status(&self) -> Result<CallToolResult, McpError> {
        use crate::config::Config;
        use crate::store::open_worktree_store;
        use crate::worktree::GitOperations;
        use std::collections::HashSet;

        let cas_root = self.cas_root.clone();
        let config = Config::load(&cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to load config: {e}")),
            data: None,
        })?;
        let wt_config = config.worktrees();

        let cwd = std::env::current_dir().unwrap_or_default();
        let git_context = GitOperations::get_context(&cwd).ok();

        let mut output = String::from("WORKTREE STATUS\n\n");

        // Current git context (caller's working directory)
        if let Some(ctx) = git_context {
            output.push_str(&format!("In worktree: {}\n", ctx.is_worktree));
            if let Some(branch) = ctx.branch {
                output.push_str(&format!("Current branch: {branch}\n"));
            }
            output.push('\n');
        }

        // System A — CAS experimental worktrees (config-gated).
        // Explicitly labeled to avoid confusion with System B (factory isolation).
        output.push_str("System A (CAS experimental worktrees):\n");
        output.push_str(&format!("  Enabled:        {}\n", wt_config.enabled));
        output.push_str(&format!("  Base path:      {}\n", wt_config.base_path));
        output.push_str(&format!("  Branch prefix:  {}\n", wt_config.branch_prefix));
        output.push_str(&format!("  Auto-merge:     {}\n", wt_config.auto_merge));
        output.push_str(&format!("  Cleanup:        {}\n", wt_config.cleanup_on_close));

        // Query worktree store for active worktrees
        let mut stored_branches: HashSet<String> = HashSet::new();
        let mut active_count = 0usize;
        let mut branch_names: Vec<String> = Vec::new();

        if let Ok(worktree_store) = open_worktree_store(&cas_root) {
            if let Ok(active_worktrees) = worktree_store.list_active() {
                active_count = active_worktrees.len();
                for wt in &active_worktrees {
                    stored_branches.insert(wt.branch.clone());
                    branch_names.push(wt.branch.clone());
                }
            }
        }

        // System B — factory (isolate=true) worktrees.
        // Scoped to `<cas_root>/worktrees/` so the main checkout is excluded.
        let factory_worktrees_base = cas_root.join("worktrees");
        let mut factory_entries: Vec<(String, std::path::PathBuf)> = Vec::new();
        if let Some(repo_root) = cas_root.parent() {
            if let Ok(git_ops) = GitOperations::detect_repo_root(repo_root).map(GitOperations::new)
            {
                if let Ok(git_worktrees) = git_ops.list_worktrees() {
                    for git_wt in git_worktrees {
                        if !git_wt.path.starts_with(&factory_worktrees_base) {
                            continue;
                        }
                        let branch = git_wt.branch.clone().unwrap_or_default();
                        if !branch.is_empty() && !stored_branches.contains(&branch) {
                            factory_entries.push((branch, git_wt.path.clone()));
                        }
                    }
                }
            }
        }

        // System B summary — always shown so callers can see isolation state
        // regardless of the System A flag.
        output.push_str("\nSystem B (factory isolation worktrees):\n");
        let b_active = factory_entries.len();
        if b_active == 0 {
            output.push_str("  Active: none\n");
        } else {
            output.push_str(&format!("  Active: {b_active}\n"));
            for (branch, path) in &factory_entries {
                output.push_str(&format!("    {} ({})\n", branch, path.display()));
            }
        }

        // System A active worktrees (if any)
        if active_count > 0 {
            output.push_str(&format!(
                "\nSystem A tracked worktrees: {} ({})\n",
                active_count,
                branch_names.join(", ")
            ));
        }

        Ok(Self::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::system_b_worktree_path;
    use tempfile::TempDir;

    #[test]
    fn system_b_worktree_path_resolves_when_git_dir_present() {
        let temp = TempDir::new().unwrap();
        let cas_root = temp.path().to_path_buf();
        let wt_path = cas_root.join("worktrees").join("alice");
        std::fs::create_dir_all(wt_path.join(".git")).unwrap();

        assert_eq!(
            system_b_worktree_path(&cas_root, "alice"),
            Some(wt_path)
        );
    }

    #[test]
    fn system_b_worktree_path_none_when_directory_missing() {
        let temp = TempDir::new().unwrap();
        let cas_root = temp.path().to_path_buf();

        assert_eq!(system_b_worktree_path(&cas_root, "ghost"), None);
    }

    #[test]
    fn system_b_worktree_path_none_when_directory_exists_but_not_a_git_worktree() {
        // A stray non-git directory under worktrees/ (e.g. leftover cruft)
        // must not be mistaken for a live factory worktree.
        let temp = TempDir::new().unwrap();
        let cas_root = temp.path().to_path_buf();
        std::fs::create_dir_all(cas_root.join("worktrees").join("bob")).unwrap();

        assert_eq!(system_b_worktree_path(&cas_root, "bob"), None);
    }
}
