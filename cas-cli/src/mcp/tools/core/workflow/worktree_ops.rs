use crate::mcp::tools::core::imports::*;

/// Check whether `path` looks like a live git worktree (has a `.git` entry
/// — a file for linked worktrees, pointing back at the main repo's
/// worktree admin dir).
///
/// Used to confirm a System B (`spawn_workers isolate=true`) worktree
/// actually exists at its resolved path before `worktree_merge` acts on
/// it (cas-1d11). Returns `false` for a path that doesn't exist or isn't a
/// git worktree — an unknowable worktree is not treated as a false
/// positive.
fn is_git_worktree(path: &std::path::Path) -> bool {
    path.join(".git").exists()
}

/// Active statuses that count as "this worker still has work tied to an epic"
/// for merge-target inference when the supervisor omits `task_id` (cas-0b32).
fn assignee_task_is_merge_relevant(status: cas_types::TaskStatus) -> bool {
    use cas_types::TaskStatus::*;
    matches!(
        status,
        Open | InProgress | Blocked | AwaitingMerge | PendingSupervisorReview
    )
}

/// Remediation block shared by merge-target rejections (cas-0b32).
fn merge_target_remediation(assignee: &str) -> String {
    format!(
        "Remediation:\n\
         1. Prefer an explicit task: `coordination action=worktree_merge id={assignee} \
         task_id=<task-id>` (or `id=factory/{assignee}`).\n\
         2. Or pin the epic: `coordination action=focus_epic id=<epic-id>` then retry \
         when the worker is a member of that factory session and project_dir matches.\n\
         3. Standalone / trunk merges require explicit intent: pass `allow_trunk=true` \
         (and `task_id` when merging a non-epic task). `force=true` only bypasses dirty \
         worktree protection — it does NOT authorize trunk.\n\
         Never relies on a silent default to main/master/staging."
    )
}

/// Resolve the parent branch a System B worker's branch should merge into
/// (cas-0938, tightened cas-0b32).
///
/// History:
/// - Pre-cas-0938: System-B always merged to trunk → silent wrong-target.
/// - cas-0938: when `task_id` is set, use the task's parent epic branch.
/// - Pre-cas-0b32 residual: **no `task_id` still fell through to trunk** with
///   reason "no task_id given". Live incident 2026-07-11: supervisor merged
///   `hv-director` to main while epic cas-0e22 was focused and the worker's
///   task belonged to that epic.
///
/// Resolution order (cas-0b32):
/// 1. Explicit `task_id` → parent epic branch; if none, **reject** unless
///    `allow_trunk` — standalone trunk needs explicit intent (NOT `force`).
/// 2. Else unique parent-epic branch among the assignee's non-closed tasks
///    (get_parent_epic errors and branchless parents reject — no fall-through).
/// 3. Else focused epic when session project_dir matches cas_root and the
///    worker is a member of that factory session.
/// 4. Else reject with remediation — **never** silent trunk default.
/// 5. Trunk only when `allow_trunk` and no epic context remains.
///
/// Always returns a human-readable reason on success.
fn resolve_system_b_merge_target(
    task_store: &dyn cas_store::TaskStore,
    task_id: Option<&str>,
    assignee: &str,
    focused: Option<&ValidatedFocusedEpic>,
    allow_trunk: bool,
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
        let epic = task_store.get_parent_epic(task_id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!(
                "Failed to resolve parent epic for task {task_id}: {e}"
            )),
            data: None,
        })?;
        if let Some(epic) = epic {
            if let Some(branch) = epic.branch.clone() {
                return Ok((
                    branch.clone(),
                    format!(
                        "epic branch {branch} (task {task_id}'s parent epic {})",
                        epic.id
                    ),
                ));
            }
            return Err(McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!(
                    "task {task_id}'s parent epic {} has no branch field — set the epic \
                     branch before worktree_merge.\n\n{}",
                    epic.id,
                    merge_target_remediation(assignee)
                )),
                data: None,
            });
        }

        // Standalone task (no parent epic): trunk only with allow_trunk=true.
        if allow_trunk {
            let trunk = trunk();
            return Ok((
                trunk.clone(),
                format!(
                    "trunk {trunk} (explicit allow_trunk=true; task {task_id} has no parent epic)"
                ),
            ));
        }
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!(
                "task {task_id} has no parent epic — refusing silent trunk merge.\n\n{}",
                merge_target_remediation(assignee)
            )),
            data: None,
        });
    }

    // No task_id: infer from assignee tasks + focused epic (cas-0b32).
    let all_tasks = task_store.list(None).map_err(|e| McpError {
        code: ErrorCode::INTERNAL_ERROR,
        message: Cow::from(format!("Failed to list tasks for merge target: {e}")),
        data: None,
    })?;

    let mut assignee_epic_branches: Vec<(String, String)> = Vec::new(); // (epic_id, branch)
    let mut branchless_parent_epics: Vec<String> = Vec::new();
    for task in &all_tasks {
        if task.assignee.as_deref() != Some(assignee) {
            continue;
        }
        if !assignee_task_is_merge_relevant(task.status) {
            continue;
        }
        // P2: surface get_parent_epic errors; reject branchless parents —
        // never silently fall through to trunk/focus (cas-0b32 review).
        match task_store.get_parent_epic(&task.id) {
            Ok(Some(epic)) => {
                if let Some(branch) = epic.branch.clone() {
                    if !assignee_epic_branches
                        .iter()
                        .any(|(id, b)| id == &epic.id && b == &branch)
                    {
                        assignee_epic_branches.push((epic.id.clone(), branch));
                    }
                } else if !branchless_parent_epics.contains(&epic.id) {
                    branchless_parent_epics.push(epic.id.clone());
                }
            }
            Ok(None) => {}
            Err(e) => {
                return Err(McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!(
                        "Failed to resolve parent epic for assignee {assignee}'s task {}: {e}\n\n{}",
                        task.id,
                        merge_target_remediation(assignee)
                    )),
                    data: None,
                });
            }
        }
    }

    if !branchless_parent_epics.is_empty() && assignee_epic_branches.is_empty() {
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!(
                "assignee {assignee} has parent epic(s) without a branch field ({}) — \
                 set epic.branch before worktree_merge.\n\n{}",
                branchless_parent_epics.join(", "),
                merge_target_remediation(assignee)
            )),
            data: None,
        });
    }

    // Dedup by branch name for uniqueness checks.
    let unique_branches: Vec<String> = {
        let mut seen = std::collections::BTreeSet::new();
        assignee_epic_branches
            .iter()
            .filter_map(|(_, b)| seen.insert(b.clone()).then(|| b.clone()))
            .collect()
    };

    if unique_branches.len() == 1 {
        let branch = unique_branches[0].clone();
        let epic_id = assignee_epic_branches
            .iter()
            .find(|(_, b)| b == &branch)
            .map(|(id, _)| id.as_str())
            .unwrap_or("?");
        return Ok((
            branch.clone(),
            format!(
                "epic branch {branch} (assignee {assignee}'s task parent epic {epic_id}; \
                 no task_id given)"
            ),
        ));
    }

    if unique_branches.len() > 1 {
        let list = assignee_epic_branches
            .iter()
            .map(|(id, b)| format!("{id}→{b}"))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!(
                "ambiguous merge target for assignee {assignee}: multiple parent epics \
                 ({list}). Pass task_id= to disambiguate.\n\n{}",
                merge_target_remediation(assignee)
            )),
            data: None,
        });
    }

    // No assignee epic: try validated focused epic when unambiguous.
    if let Some(focused) = focused {
        match task_store.get(&focused.epic_id) {
            Ok(epic) if epic.task_type == cas_types::TaskType::Epic => {
                if let Some(branch) = epic.branch.clone() {
                    return Ok((
                        branch.clone(),
                        format!(
                            "epic branch {branch} (focused epic {} in session {}; \
                             no task_id and no assignee epic assignment)",
                            focused.epic_id, focused.session_name
                        ),
                    ));
                }
                return Err(McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!(
                        "focused epic {} has no branch field — set it before merge.\n\n{}",
                        focused.epic_id,
                        merge_target_remediation(assignee)
                    )),
                    data: None,
                });
            }
            Ok(other) => {
                return Err(McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!(
                        "focused id {} is not an Epic (task_type={:?}).\n\n{}",
                        focused.epic_id,
                        other.task_type,
                        merge_target_remediation(assignee)
                    )),
                    data: None,
                });
            }
            Err(e) => {
                return Err(McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!(
                        "focused epic {} not found in this project store: {e}\n\n{}",
                        focused.epic_id,
                        merge_target_remediation(assignee)
                    )),
                    data: None,
                });
            }
        }
    }

    if allow_trunk {
        let trunk = trunk();
        return Ok((
            trunk.clone(),
            format!(
                "trunk {trunk} (explicit allow_trunk=true; no task_id, no assignee epic, \
                 no validated focused epic)"
            ),
        ));
    }

    Err(McpError {
        code: ErrorCode::INVALID_PARAMS,
        message: Cow::from(format!(
            "no merge target for worktree assignee {assignee}: no task_id, no assignee \
             epic assignment, and no validated focused epic — refusing silent trunk \
             default (cas-0b32).\n\n{}",
            merge_target_remediation(assignee)
        )),
        data: None,
    })
}

/// Focused epic that passed session project_dir + worker membership checks.
#[derive(Debug, Clone)]
struct ValidatedFocusedEpic {
    epic_id: String,
    session_name: String,
}

/// Load and validate focused epic for merge-target inference (cas-0b32 review).
///
/// Requires:
/// - `CAS_FACTORY_SESSION` session metadata with a pinned/default epic id
/// - `SessionMetadata.project_dir` matches this `cas_root`'s project (canonical)
/// - `assignee` is a worker member of that session (metadata.workers or
///   agent_store with matching factory_session)
///
/// Cross-project / stale pins return `None` (not used as a target).
fn load_validated_focused_epic(
    cas_root: &std::path::Path,
    assignee: &str,
) -> Option<ValidatedFocusedEpic> {
    use crate::ui::factory::{SessionMetadata, metadata_path};

    let session = std::env::var("CAS_FACTORY_SESSION").ok()?;
    let data = std::fs::read_to_string(metadata_path(&session)).ok()?;
    let meta: SessionMetadata = serde_json::from_str(&data).ok()?;
    let epic_id = meta
        .pinned_epic_id
        .filter(|s| !s.trim().is_empty())
        .or_else(|| meta.epic_id.filter(|s| !s.trim().is_empty()))?;

    // project_dir must match this cas_root's project.
    let project_root = cas_root.parent().unwrap_or(cas_root);
    let meta_project = meta.project_dir.as_ref().filter(|s| !s.trim().is_empty())?;
    let meta_canon = std::fs::canonicalize(meta_project).ok()?;
    let cas_canon = std::fs::canonicalize(project_root).ok()?;
    if meta_canon != cas_canon {
        return None; // cross-project / stale pin
    }

    // Worker membership: session metadata workers list, else agent store.
    let in_meta_workers = meta.workers.iter().any(|w| w.name == assignee);
    if !in_meta_workers {
        let agent_ok = crate::store::open_agent_store(cas_root)
            .ok()
            .and_then(|store| {
                cas_store::AgentStore::list(store.as_ref(), None)
                    .ok()
                    .map(|agents| {
                        agents.iter().any(|a| {
                            a.name == assignee
                                && a.factory_session.as_deref() == Some(session.as_str())
                        })
                    })
            })
            .unwrap_or(false);
        if !agent_ok {
            return None;
        }
    }

    Some(ValidatedFocusedEpic {
        epic_id,
        session_name: session,
    })
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
    /// `spawn_workers isolate=true` convention (branch `factory/<assignee>`,
    /// path resolved via `WorktreeManager::worktree_path_for_worker` so a
    /// customized `worktrees.base_path` still resolves correctly), which is
    /// never registered in the store and doesn't check `worktrees.enabled`
    /// at all (cas-1d11). Without this fallback, spawn happily created
    /// isolated worktrees while the only supervisor-callable merge action
    /// refused every one of them — forcing a manual `git worktree add` +
    /// merge + push that bypassed factory tracking/lease/cleanup entirely.
    ///
    /// A System-B merge target is resolved via `task_id`, assignee epic
    /// assignment, and focused epic (cas-0938 + cas-0b32). Never silently
    /// defaults a factory worker merge to trunk — trunk requires explicit
    /// `allow_trunk=true` (independent of `force`, which only bypasses dirty
    /// worktree protection). The resolved target is always surfaced.
    pub async fn worktree_merge(
        &self,
        id: &str,
        force: bool,
        task_id: Option<&str>,
        allow_trunk: bool,
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
                let focused = load_validated_focused_epic(&cas_root, assignee);
                let (parent_branch, target_reason) = resolve_system_b_merge_target(
                    task_store.as_ref(),
                    task_id,
                    assignee,
                    focused.as_ref(),
                    allow_trunk, // NOT force — dirty bypass stays separate (cas-0b32 review P1)
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
    use super::is_git_worktree;
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
}
