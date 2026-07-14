use crate::mcp::tools::core::imports::*;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Check whether `path` looks like a live git worktree (has a `.git` entry
/// — a file for linked worktrees, pointing back at the main repo's
/// worktree admin dir).
///
/// Used to confirm a System B (`spawn_workers isolate=true`) worktree
/// actually exists at its resolved path before `worktree_merge` acts on
/// it (cas-1d11). Returns `false` for a path that doesn't exist or isn't a
/// git worktree — an unknowable worktree is not treated as a false
/// positive.
fn is_git_worktree(path: &Path) -> bool {
    path.join(".git").exists()
}

/// Path prefix match with canonicalize fallback (symlinks / relative forms).
fn path_is_under(path: &Path, base: &Path) -> bool {
    if path.starts_with(base) {
        return true;
    }
    match (std::fs::canonicalize(path), std::fs::canonicalize(base)) {
        (Ok(p), Ok(b)) => p.starts_with(b),
        _ => false,
    }
}

fn paths_equal(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => false,
    }
}

/// Resolve the configured factory worktree base for this project.
///
/// Matches `spawn_workers isolate=true` / `WorktreeManager` resolution so
/// `worktree_list` does not hardcode `<cas_root>/worktrees` (cas-d1a0).
fn resolve_factory_worktree_base(cas_root: &Path) -> PathBuf {
    use crate::config::Config;

    let Some(project_dir) = cas_root.parent() else {
        return cas_root.join("worktrees");
    };
    let config = Config::load(cas_root).unwrap_or_default();
    config.worktrees().resolve_base_path(project_dir)
}

/// Whether a live git worktree looks CAS-managed (factory, epic, cas/*, or
/// under known CAS worktree roots) and should appear in worktree_list even
/// without a WorktreeStore row (sibling/predecessor sessions).
fn is_cas_pattern_worktree(
    path: &Path,
    branch: Option<&str>,
    cas_root: &Path,
    factory_base: &Path,
    repo_root: &Path,
) -> bool {
    // Main checkout is never listed as a managed worktree entry.
    if paths_equal(path, repo_root) {
        return false;
    }

    if path_is_under(path, factory_base) {
        return true;
    }
    // Default System B layout — still scanned when base_path is customized.
    if path_is_under(path, &cas_root.join("worktrees")) {
        return true;
    }
    // Claude Code agent isolation dirs (also swept by factory cleanup).
    if path_is_under(path, &repo_root.join(".claude").join("worktrees")) {
        return true;
    }

    if let Some(b) = branch {
        if b.starts_with("factory/") || b.starts_with("epic/") || b.starts_with("cas/") {
            return true;
        }
    }

    false
}

fn is_factory_style_worktree(
    path: &Path,
    branch: &str,
    cas_root: &Path,
    factory_base: &Path,
) -> bool {
    branch.starts_with("factory/")
        || path_is_under(path, factory_base)
        || path_is_under(path, &cas_root.join("worktrees"))
}

/// Reconcile live git worktrees that match CAS patterns but are missing from
/// the SQLite WorktreeStore (System B never registers; System A rows are
/// project-scoped but may be absent for sibling-session worktrees).
///
/// Returns transient `Worktree` rows with `git:` id prefix for display.
fn collect_untracked_git_worktrees(
    cas_root: &Path,
    factory_base: &Path,
    tracked_branches: &HashSet<String>,
    tracked_paths: &HashSet<PathBuf>,
) -> Vec<crate::types::Worktree> {
    use crate::types::Worktree;
    use crate::worktree::GitOperations;

    let mut out = Vec::new();
    let Some(project_dir) = cas_root.parent() else {
        return out;
    };
    let Ok(repo_root) = GitOperations::detect_repo_root(project_dir) else {
        return out;
    };
    let git_ops = GitOperations::new(repo_root.clone());
    let Ok(git_worktrees) = git_ops.list_worktrees() else {
        return out;
    };

    for git_wt in git_worktrees {
        if !is_cas_pattern_worktree(
            &git_wt.path,
            git_wt.branch.as_deref(),
            cas_root,
            factory_base,
            &repo_root,
        ) {
            continue;
        }

        let branch = git_wt.branch.clone().unwrap_or_default();
        if !branch.is_empty() && tracked_branches.contains(&branch) {
            continue;
        }
        if tracked_paths.iter().any(|p| paths_equal(p, &git_wt.path)) {
            continue;
        }

        let id_key = if !branch.is_empty() {
            branch.clone()
        } else {
            git_wt.path.display().to_string()
        };
        let display_branch = if branch.is_empty() {
            "(detached)".to_string()
        } else {
            branch
        };

        out.push(Worktree::new(
            format!("git:{id_key}"),
            display_branch,
            "unknown".to_string(),
            git_wt.path,
        ));
    }

    out
}

/// Resolve the parent branch a System B worker's branch should merge into
/// (cas-0938).
///
/// Before this fix, `worktree_merge`'s System-B fallback always merged
/// into the configured/detected TRUNK — even for epic workers whose
/// close-gate demands merging into the task's EPIC branch. That produced
/// a wrong-target merge that also deleted the branch (`cleanup_on_close`),
/// worse than cas-1d11's pre-fix refusal: unreviewed worker code silently
/// landed on trunk with no way to recover it.
///
/// When `task_id` is supplied, resolves via the task's parent epic
/// (`TaskStore::get_parent_epic` — the exact lookup the close-gate itself
/// uses in `close_ops.rs`). Refuses when `task_id` is supplied but doesn't
/// resolve to a real task: the caller asserted a specific epic context we
/// can't verify, so silently falling back to trunk there would repeat the
/// original defect under a different disguise. Falls back to the
/// configured/detected trunk only when no `task_id` was given, or the
/// given task genuinely has no parent epic — both are legitimately "no
/// epic in play". Always returns a human-readable reason so the caller
/// can see which branch (and why) without opening any other tool.
fn resolve_system_b_merge_target(
    task_store: &dyn cas_store::TaskStore,
    task_id: Option<&str>,
    trunk: impl FnOnce() -> String,
) -> Result<(String, String), McpError> {
    if let Some(task_id) = task_id {
        task_store.get(task_id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!(
                "task_id {task_id} not found — refusing to guess a merge target: {e}"
            )),
            data: None,
        })?;
        let epic_branch = task_store
            .get_parent_epic(task_id)
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!(
                    "Failed to resolve parent epic for task {task_id}: {e}"
                )),
                data: None,
            })?
            .and_then(|epic| epic.branch);

        if let Some(branch) = epic_branch {
            return Ok((
                branch.clone(),
                format!("epic branch {branch} (task {task_id}'s parent epic)"),
            ));
        }

        // Task exists but has no parent epic — legitimately "no epic in play".
        let trunk = trunk();
        return Ok((
            trunk.clone(),
            format!("trunk {trunk} (task {task_id} has no parent epic)"),
        ));
    }

    let trunk = trunk();
    Ok((
        trunk.clone(),
        format!("trunk {trunk} (no task_id given — no epic context to resolve)"),
    ))
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
    ///
    /// Combines the project-scoped WorktreeStore (System A) with a live
    /// `git worktree list` reconcile for CAS-pattern paths/branches that were
    /// never registered (System B factory workers, sibling-session epic
    /// worktrees). Registry rows live in `.cas/cas.db` — shared by every
    /// session in the project; git is the second source of truth when a
    /// session never wrote a row (cas-d1a0).
    pub async fn worktree_list(
        &self,
        all: bool,
        status_filter: Option<&str>,
        orphans_only: bool,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_task_store, open_worktree_store};
        use crate::types::{AgentStatus, TaskStatus, WorktreeStatus};

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

        let parsed_status: Option<WorktreeStatus> = if let Some(status_str) = status_filter {
            Some(status_str.parse().map_err(|_| McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Invalid status: {status_str}")),
                data: None,
            })?)
        } else {
            None
        };

        let mut worktrees = if let Some(status) = parsed_status {
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

        // Live git reconcile only for views that include active worktrees.
        // Non-active status filters (merged/abandoned/…) must not gain
        // transient Active git rows.
        let should_reconcile_git = match parsed_status {
            None => true,
            Some(WorktreeStatus::Active) => true,
            Some(_) => false,
        };

        let factory_base = resolve_factory_worktree_base(&cas_root);
        if should_reconcile_git {
            let tracked_branches: HashSet<String> =
                worktrees.iter().map(|wt| wt.branch.clone()).collect();
            let tracked_paths: HashSet<PathBuf> =
                worktrees.iter().map(|wt| wt.path.clone()).collect();
            worktrees.extend(collect_untracked_git_worktrees(
                &cas_root,
                &factory_base,
                &tracked_branches,
                &tracked_paths,
            ));
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
            // git: prefix = reconciled from live git (not in WorktreeStore).
            // Factory-style vs other CAS patterns get distinct labels so
            // supervisors can tell spawn workers from untracked epic trees.
            let type_indicator = if wt.id.starts_with("git:") {
                if is_factory_style_worktree(&wt.path, &wt.branch, &cas_root, &factory_base) {
                    " [factory]"
                } else {
                    " [untracked]"
                }
            } else {
                ""
            };
            output.push_str(&format!(
                "{} {} - {} {}{}{}\n   Epic: {}\n   Path: {}\n\n",
                status_icon,
                wt.id,
                wt.branch,
                wt.status,
                path_status,
                type_indicator,
                wt.epic_id.as_deref().unwrap_or("-"),
                wt.path.display()
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
    /// `spawn_workers isolate=true` convention (branch `factory/<assignee>`,
    /// path resolved via `WorktreeManager::worktree_path_for_worker` so a
    /// customized `worktrees.base_path` still resolves correctly), which is
    /// never registered in the store and doesn't check `worktrees.enabled`
    /// at all (cas-1d11). Without this fallback, spawn happily created
    /// isolated worktrees while the only supervisor-callable merge action
    /// refused every one of them — forcing a manual `git worktree add` +
    /// merge + push that bypassed factory tracking/lease/cleanup entirely.
    ///
    /// A System-B merge target is resolved via `task_id` (cas-0938): pass
    /// the worker's task and this merges into that task's parent EPIC
    /// branch, not the repo trunk — merging an epic worker's commits to
    /// trunk instead of its epic branch is a silent-wrong-target class of
    /// bug (the close-gate still rejects, and the branch is gone). Falls
    /// back to trunk only when no `task_id` is given or the task has no
    /// parent epic; the resolved target is always surfaced in the result.
    pub async fn worktree_merge(
        &self,
        id: &str,
        force: bool,
        task_id: Option<&str>,
    ) -> Result<CallToolResult, McpError> {
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

        // Repo root: `cas_root` is `<repo>/.cas`, so its parent is the repo
        // — consistent with close_ops.rs's `close_project_root` and every
        // other cas_root-anchored lookup in this handler. `cwd` is
        // process-global on the long-lived MCP server and must not be
        // trusted to match the intended repo (cas-0938).
        let cwd = cas_root.parent().unwrap_or(&cas_root).to_path_buf();

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

        let (mut worktree, is_system_b, target_reason) = match system_a {
            Some(wt) => (wt, false, String::new()),
            None => {
                let assignee = id.strip_prefix("factory/").unwrap_or(id);
                let path = manager.worktree_path_for_worker(assignee);
                if !is_git_worktree(&path) {
                    return Err(McpError {
                        code: ErrorCode::INVALID_PARAMS,
                        message: Cow::from(format!(
                            "Worktree not found: {id} (checked System A worktree store and \
                             the System B path {})",
                            path.display()
                        )),
                        data: None,
                    });
                }
                let task_store = self.open_task_store()?;
                let (parent_branch, target_reason) = resolve_system_b_merge_target(
                    task_store.as_ref(),
                    task_id,
                    || {
                        Config::configured_epic_base_branch(&cwd)
                            .unwrap_or_else(|| manager.git().detect_default_branch())
                    },
                )?;
                (
                    crate::types::Worktree::new(
                        format!("system-b-{assignee}"),
                        format!("factory/{assignee}"),
                        parent_branch,
                        path,
                    ),
                    true,
                    target_reason,
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

        // Always surface the resolved target for System-B merges — the
        // wrong-target-to-trunk defect (cas-0938) was invisible precisely
        // because the tool didn't say which branch it actually used.
        let target_suffix = if is_system_b {
            format!(" [resolved via: {target_reason}]")
        } else {
            String::new()
        };

        // Promote entries if configured
        if wt_config.promote_entries_on_merge {
            if let Ok(count) = self.promote_branch_entries(&worktree.branch) {
                if count > 0 {
                    return Ok(Self::success(format!(
                        "Merged worktree {} to {}.{} Commit: {}\nPromoted {} entries from branch scope.",
                        worktree.id,
                        worktree.parent_branch,
                        target_suffix,
                        merge_commit.as_deref().unwrap_or("none"),
                        count
                    )));
                }
            }
        }

        Ok(Self::success(format!(
            "Merged worktree {} to {}.{} Commit: {}",
            worktree.id,
            worktree.parent_branch,
            target_suffix,
            merge_commit.as_deref().unwrap_or("none")
        )))
    }

    /// Get current worktree status
    pub async fn worktree_status(&self) -> Result<CallToolResult, McpError> {
        use crate::config::Config;
        use crate::store::open_worktree_store;
        use crate::worktree::GitOperations;

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
        let mut stored_paths: HashSet<PathBuf> = HashSet::new();
        let mut active_count = 0usize;
        let mut branch_names: Vec<String> = Vec::new();

        if let Ok(worktree_store) = open_worktree_store(&cas_root) {
            if let Ok(active_worktrees) = worktree_store.list_active() {
                active_count = active_worktrees.len();
                for wt in &active_worktrees {
                    stored_branches.insert(wt.branch.clone());
                    stored_paths.insert(wt.path.clone());
                    branch_names.push(wt.branch.clone());
                }
            }
        }

        // Live git reconcile — same CAS-pattern rules as worktree_list (cas-d1a0).
        let factory_base = resolve_factory_worktree_base(&cas_root);
        let untracked = collect_untracked_git_worktrees(
            &cas_root,
            &factory_base,
            &stored_branches,
            &stored_paths,
        );
        let mut factory_entries: Vec<(String, PathBuf)> = Vec::new();
        let mut other_untracked: Vec<(String, PathBuf)> = Vec::new();
        for wt in untracked {
            if is_factory_style_worktree(&wt.path, &wt.branch, &cas_root, &factory_base) {
                factory_entries.push((wt.branch, wt.path));
            } else {
                other_untracked.push((wt.branch, wt.path));
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

        // Untracked CAS-pattern worktrees (e.g. epic/* outside factory base)
        // from sibling sessions — visible for management without a store row.
        if !other_untracked.is_empty() {
            output.push_str("\nUntracked CAS-pattern worktrees:\n");
            for (branch, path) in &other_untracked {
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
    use super::{
        is_cas_pattern_worktree, is_factory_style_worktree, is_git_worktree, path_is_under,
    };
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn is_git_worktree_true_when_git_entry_present() {
        let temp = TempDir::new().unwrap();
        let wt_path = temp.path().join("alice");
        std::fs::create_dir_all(wt_path.join(".git")).unwrap();

        assert!(is_git_worktree(&wt_path));
    }

    #[test]
    fn is_git_worktree_false_when_path_missing() {
        let temp = TempDir::new().unwrap();
        assert!(!is_git_worktree(&temp.path().join("ghost")));
    }

    #[test]
    fn is_git_worktree_false_when_directory_exists_but_not_a_git_worktree() {
        // A stray non-git directory (e.g. leftover cruft) must not be
        // mistaken for a live factory worktree.
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("bob");
        std::fs::create_dir_all(&path).unwrap();

        assert!(!is_git_worktree(&path));
    }

    #[test]
    fn cas_pattern_matches_factory_branch_outside_cas_dir() {
        let repo = Path::new("/repo");
        let cas = Path::new("/repo/.cas");
        let factory_base = Path::new("/repo/.cas/worktrees");
        let path = Path::new("/tmp/elsewhere/worker");
        assert!(is_cas_pattern_worktree(
            path,
            Some("factory/hv-food-qa"),
            cas,
            factory_base,
            repo,
        ));
    }

    #[test]
    fn cas_pattern_matches_epic_branch_outside_cas_dir() {
        let repo = Path::new("/repo");
        let cas = Path::new("/repo/.cas");
        let factory_base = Path::new("/repo/.cas/worktrees");
        let path = Path::new("/tmp/ozer-epic-ea3e-hv");
        assert!(is_cas_pattern_worktree(
            path,
            Some("epic/integrate-cas-ea3e"),
            cas,
            factory_base,
            repo,
        ));
        assert!(!is_factory_style_worktree(
            path,
            "epic/integrate-cas-ea3e",
            cas,
            factory_base,
        ));
    }

    #[test]
    fn cas_pattern_rejects_main_checkout_and_unrelated_branches() {
        let repo = Path::new("/repo");
        let cas = Path::new("/repo/.cas");
        let factory_base = Path::new("/repo/.cas/worktrees");
        assert!(!is_cas_pattern_worktree(
            repo,
            Some("staging"),
            cas,
            factory_base,
            repo,
        ));
        assert!(!is_cas_pattern_worktree(
            Path::new("/tmp/hand-made"),
            Some("feature/hand-made"),
            cas,
            factory_base,
            repo,
        ));
    }

    #[test]
    fn path_is_under_matches_prefix() {
        let base = Path::new("/proj/.cas/worktrees");
        assert!(path_is_under(
            Path::new("/proj/.cas/worktrees/alice"),
            base
        ));
        assert!(!path_is_under(Path::new("/proj/other"), base));
    }
}
