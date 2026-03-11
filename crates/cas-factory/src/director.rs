//! Data loading for the director panel.
//!
//! This module provides DirectorData aggregation from CAS stores,
//! without any TUI/rendering dependencies.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use cas_store::{
    AgentStore, EventStore, Reminder, ReminderStore, SqliteAgentStore, SqliteEventStore,
    SqliteReminderStore, SqliteTaskStore, SqliteWorktreeStore, TaskStore, WorktreeStore,
};
use cas_types::{
    AgentRole, AgentStatus, DependencyType, Event, EventType, Priority, Task, TaskStatus, TaskType,
    WorktreeStatus,
};

use crate::changes::{FileChangeInfo, GitFileStatus, SourceChangesInfo};

/// A summary of a task for display
#[derive(Debug, Clone)]
pub struct TaskSummary {
    pub id: String,
    pub title: String,
    pub status: TaskStatus,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub task_type: TaskType,
    /// Parent epic ID (if this is a subtask)
    pub epic: Option<String>,
    /// Branch name (for epics)
    pub branch: Option<String>,
}

/// A summary of an agent for display
#[derive(Debug, Clone)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    pub status: AgentStatus,
    pub current_task: Option<String>,
    /// Latest activity (description, timestamp)
    pub latest_activity: Option<(String, chrono::DateTime<chrono::Utc>)>,
    /// Last heartbeat timestamp
    pub last_heartbeat: Option<chrono::DateTime<chrono::Utc>>,
}

/// A group of tasks under an epic
#[derive(Debug, Clone)]
pub struct EpicGroup {
    /// The epic task itself
    pub epic: TaskSummary,
    /// Subtasks belonging to this epic
    pub subtasks: Vec<TaskSummary>,
    /// Whether any subtask is in_progress
    pub has_active: bool,
}

/// Data for the director panel
#[derive(Debug, Clone)]
pub struct DirectorData {
    /// Ready tasks (open, not blocked)
    pub ready_tasks: Vec<TaskSummary>,
    /// In-progress tasks
    pub in_progress_tasks: Vec<TaskSummary>,
    /// Epic tasks (for tracking epic status)
    pub epic_tasks: Vec<TaskSummary>,
    /// Active agents
    pub agents: Vec<AgentSummary>,
    /// Recent activity events
    pub activity: Vec<Event>,
    /// Map of agent ID to name
    pub agent_id_to_name: HashMap<String, String>,
    /// Git changes by source (main repo + worktrees)
    pub changes: Vec<SourceChangesInfo>,
    /// Whether git changes have been loaded (for lazy loading)
    pub git_loaded: bool,
    /// Pending reminders (across all supervisors in this session)
    pub reminders: Vec<Reminder>,
    /// Count of closed subtasks per epic (epic_id -> closed_count)
    pub epic_closed_counts: HashMap<String, usize>,
}

impl DirectorData {
    /// Load data from CAS stores
    ///
    /// # Arguments
    /// * `cas_dir` - Path to the CAS directory
    /// * `worktree_root` - Optional path to the worktree directory for factory workers
    pub fn load(cas_dir: &Path, worktree_root: Option<&Path>) -> anyhow::Result<Self> {
        Self::load_with_git(cas_dir, worktree_root, true)
    }

    /// Fast load without git changes (for initial startup)
    ///
    /// Git changes are loaded lazily on first refresh to speed up daemon startup.
    pub fn load_fast(cas_dir: &Path) -> anyhow::Result<Self> {
        Self::load_with_git(cas_dir, None, false)
    }

    /// Load data with optional git change collection.
    ///
    /// This allows callers to refresh CAS data frequently while throttling expensive git ops.
    pub fn load_with_git(
        cas_dir: &Path,
        worktree_root: Option<&Path>,
        load_git: bool,
    ) -> anyhow::Result<Self> {
        Self::load_with_options(cas_dir, worktree_root, load_git)
    }

    /// Refresh only git changes while preserving already-loaded task/agent/activity data.
    pub fn refresh_git_changes(
        &mut self,
        cas_dir: &Path,
        worktree_root: Option<&Path>,
    ) -> anyhow::Result<()> {
        self.changes = load_all_git_changes(cas_dir, worktree_root, &self.agent_id_to_name)?;
        self.git_loaded = true;
        Ok(())
    }

    /// Load data with configurable options
    fn load_with_options(
        cas_dir: &Path,
        worktree_root: Option<&Path>,
        load_git: bool,
    ) -> anyhow::Result<Self> {
        // Load tasks
        let task_store = SqliteTaskStore::open(cas_dir)?;
        let tasks: Vec<Task> = TaskStore::list(&task_store, None)?;

        // Build assignee to task map for looking up current tasks
        let mut assignee_tasks: HashMap<String, String> = HashMap::new();
        for task in &tasks {
            if task.status == TaskStatus::InProgress
                && let Some(ref assignee) = task.assignee
            {
                assignee_tasks.insert(assignee.clone(), task.id.clone());
            }
        }

        // Load parent-child dependencies to find epic relationships
        let parent_child_deps = task_store.list_dependencies(Some(DependencyType::ParentChild))?;

        // Build map: child_id -> parent_epic_id
        let child_to_epic: HashMap<String, String> = parent_child_deps
            .iter()
            .map(|dep| (dep.from_id.clone(), dep.to_id.clone()))
            .collect();

        // Helper to convert Task to TaskSummary with epic relationship
        let to_summary = |t: &Task| -> TaskSummary {
            TaskSummary {
                id: t.id.clone(),
                title: t.title.clone(),
                status: t.status,
                priority: t.priority,
                assignee: t.assignee.clone(),
                task_type: t.task_type,
                epic: child_to_epic.get(&t.id).cloned(),
                branch: t.branch.clone(),
            }
        };

        // Filter and convert tasks
        let ready_tasks: Vec<TaskSummary> = tasks
            .iter()
            .filter(|t| {
                (t.status == TaskStatus::Open || t.status == TaskStatus::Blocked)
                    && t.task_type != TaskType::Epic
            })
            .map(to_summary)
            .collect();

        let in_progress_tasks: Vec<TaskSummary> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::InProgress && t.task_type != TaskType::Epic)
            .map(to_summary)
            .collect();

        // Epic tasks (for tracking epic lifecycle)
        let epic_tasks: Vec<TaskSummary> = tasks
            .iter()
            .filter(|t| t.task_type == TaskType::Epic)
            .map(to_summary)
            .collect();

        // Count closed subtasks per epic
        let mut epic_closed_counts: HashMap<String, usize> = HashMap::new();
        for task in &tasks {
            if task.status == TaskStatus::Closed
                && task.task_type != TaskType::Epic
                && let Some(epic_id) = child_to_epic.get(&task.id)
            {
                *epic_closed_counts.entry(epic_id.clone()).or_insert(0) += 1;
            }
        }

        // Load recent activity first (needed for agent latest_activity)
        let event_store = SqliteEventStore::open(cas_dir)?;
        let activity = event_store.list_recent(50)?; // Load more to find worker activities

        // Build map of agent_id -> latest worker activity
        let worker_activity_types = [
            EventType::WorkerSubagentSpawned,
            EventType::WorkerSubagentCompleted,
            EventType::WorkerFileEdited,
            EventType::WorkerGitCommit,
            EventType::WorkerVerificationBlocked,
            EventType::VerificationStarted,
            EventType::VerificationAdded,
        ];
        let mut agent_latest_activity: HashMap<String, (String, chrono::DateTime<chrono::Utc>)> =
            HashMap::new();
        for event in &activity {
            if worker_activity_types.contains(&event.event_type)
                && let Some(session_id) = &event.session_id
            {
                // Only keep the latest (first encountered since list is sorted by time desc)
                agent_latest_activity
                    .entry(session_id.clone())
                    .or_insert_with(|| (event.summary.clone(), event.created_at));
            }
        }

        // Load agents
        let agent_store = SqliteAgentStore::open(cas_dir)?;
        let agents_list = AgentStore::list(&agent_store, None)?;

        let mut agent_id_to_name = HashMap::new();
        let agents: Vec<AgentSummary> = agents_list
            .into_iter()
            // Only show factory-relevant agents (not CLI agents with Standard role)
            .filter(|a| {
                (a.status == AgentStatus::Active || a.status == AgentStatus::Idle)
                    && (a.role == AgentRole::Supervisor
                        || a.role == AgentRole::Worker
                        || a.role == AgentRole::Director)
            })
            .map(|a| {
                agent_id_to_name.insert(a.id.clone(), a.name.clone());
                let current_task = assignee_tasks.get(&a.id).cloned();
                let latest_activity = agent_latest_activity.get(&a.id).cloned();
                AgentSummary {
                    id: a.id,
                    name: a.name,
                    status: a.status,
                    current_task,
                    latest_activity,
                    last_heartbeat: Some(a.last_heartbeat),
                }
            })
            .collect();

        // Trim activity to 20 for display
        let activity: Vec<Event> = activity.into_iter().take(20).collect();

        // Load git changes (optionally skip for fast startup)
        let (changes, git_loaded) = if load_git {
            let changes = load_all_git_changes(cas_dir, worktree_root, &agent_id_to_name)?;
            (changes, true)
        } else {
            (Vec::new(), false)
        };

        // Load pending + recently fired reminders (best-effort, non-fatal)
        let reminders = SqliteReminderStore::open(cas_dir)
            .and_then(|store| {
                store.init()?;
                let mut all = store.list_all_pending()?;
                // Include reminders fired within the last 60 seconds so they
                // don't silently vanish from the panel
                all.extend(store.list_recently_fired(60)?);
                Ok(all)
            })
            .unwrap_or_default();

        Ok(Self {
            ready_tasks,
            in_progress_tasks,
            epic_tasks,
            agents,
            activity,
            agent_id_to_name,
            changes,
            git_loaded,
            reminders,
            epic_closed_counts,
        })
    }

    /// Get all tasks (in_progress + ready) grouped by epic
    ///
    /// Returns (epic_groups, standalone_tasks) where:
    /// - epic_groups: Tasks grouped under their parent epic
    /// - standalone_tasks: Tasks without a parent epic
    pub fn tasks_by_epic(&self) -> (Vec<EpicGroup>, Vec<TaskSummary>) {
        // Build a map of epic_id -> subtasks
        let mut epic_subtasks: HashMap<String, Vec<TaskSummary>> = HashMap::new();
        let mut standalone: Vec<TaskSummary> = Vec::new();

        // Collect all tasks (in_progress first, then ready)
        for task in self.in_progress_tasks.iter().chain(self.ready_tasks.iter()) {
            if let Some(epic_id) = &task.epic {
                epic_subtasks
                    .entry(epic_id.clone())
                    .or_default()
                    .push(task.clone());
            } else {
                standalone.push(task.clone());
            }
        }

        // Build epic groups from epic_tasks
        let mut groups: Vec<EpicGroup> = self
            .epic_tasks
            .iter()
            .filter_map(|epic| {
                let subtasks = epic_subtasks.remove(&epic.id).unwrap_or_default();
                // Only include epics that have subtasks in the current view
                if subtasks.is_empty() {
                    return None;
                }
                let has_active = subtasks.iter().any(|t| t.status == TaskStatus::InProgress);
                Some(EpicGroup {
                    epic: epic.clone(),
                    subtasks,
                    has_active,
                })
            })
            .collect();

        // Sort groups: active first, then by epic priority
        groups.sort_by_key(|g| (!g.has_active, g.epic.priority.0));

        (groups, standalone)
    }
}

/// A repo to check for git changes
struct RepoToCheck {
    path: PathBuf,
    name: String,
    agent_name: Option<String>,
}

/// Load git changes from main repo and factory worktrees
///
/// Uses rayon for parallel git operations to support 1000+ workers.
fn load_all_git_changes(
    cas_dir: &Path,
    worktree_root: Option<&Path>,
    agent_id_to_name: &HashMap<String, String>,
) -> anyhow::Result<Vec<SourceChangesInfo>> {
    use rayon::prelude::*;

    let repo_root = cas_dir.parent().unwrap_or(cas_dir);

    // Collect all repos to check (fast, no I/O)
    let mut repos_to_check: Vec<RepoToCheck> = Vec::new();

    // 1. Main repo
    repos_to_check.push(RepoToCheck {
        path: repo_root.to_path_buf(),
        name: "main".to_string(),
        agent_name: None,
    });

    // 2. Worktrees (from CAS database)
    if let Ok(worktree_store) = SqliteWorktreeStore::open(cas_dir)
        && let Ok(worktrees) = worktree_store.list_by_status(WorktreeStatus::Active)
    {
        for wt in worktrees {
            let name = wt
                .branch
                .split('/')
                .next_back()
                .unwrap_or(&wt.branch)
                .to_string();
            let agent_name = wt
                .created_by_agent
                .as_ref()
                .and_then(|id| agent_id_to_name.get(id).cloned());
            repos_to_check.push(RepoToCheck {
                path: PathBuf::from(&wt.path),
                name,
                agent_name,
            });
        }
    }

    // 3. Worktree directories (for factory workers) - only check worktrees matching active agents
    if let Some(wt_dir) = worktree_root
        && wt_dir.exists()
        && wt_dir.is_dir()
    {
        for agent_name in agent_id_to_name.values() {
            let path = wt_dir.join(agent_name);
            if path.is_dir() && path.join(".git").exists() {
                repos_to_check.push(RepoToCheck {
                    path,
                    name: agent_name.clone(),
                    agent_name: Some(agent_name.clone()),
                });
            }
        }
    }

    // Process all repos in parallel using rayon
    let mut sources: Vec<SourceChangesInfo> = repos_to_check
        .into_par_iter()
        .filter_map(|repo| get_source_changes(&repo.path, &repo.name, repo.agent_name))
        .collect();

    // Sort by total changes descending
    sources.sort_by(|a, b| {
        let a_total = a.total_added + a.total_removed;
        let b_total = b.total_added + b.total_removed;
        b_total.cmp(&a_total)
    });

    Ok(sources)
}

/// Get git changes for a single source directory
fn get_source_changes(
    path: &Path,
    name: &str,
    agent_name: Option<String>,
) -> Option<SourceChangesInfo> {
    if !path.exists() {
        return None;
    }

    let changes = get_git_changes(path);
    if changes.is_empty() {
        return None;
    }

    let total_added: usize = changes.iter().map(|c| c.lines_added).sum();
    let total_removed: usize = changes.iter().map(|c| c.lines_removed).sum();

    Some(SourceChangesInfo {
        source_name: name.to_string(),
        source_path: path.to_path_buf(),
        agent_name,
        changes,
        total_added,
        total_removed,
    })
}

/// Get git changes for a directory
fn get_git_changes(repo_path: &Path) -> Vec<FileChangeInfo> {
    // Run git status --porcelain
    let status_output = match Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
    {
        Ok(o) => o,
        Err(_) => return Vec::new(),
    };

    if !status_output.status.success() {
        return Vec::new();
    }

    let status_str = String::from_utf8_lossy(&status_output.stdout);

    // Get line counts from both staged and unstaged diffs
    let mut line_counts: HashMap<String, (usize, usize)> = HashMap::new();

    // Staged changes
    if let Ok(output) = Command::new("git")
        .args(["diff", "--cached", "--numstat"])
        .current_dir(repo_path)
        .output()
        && output.status.success()
    {
        parse_diff_numstat(&String::from_utf8_lossy(&output.stdout), &mut line_counts);
    }

    // Unstaged changes
    if let Ok(output) = Command::new("git")
        .args(["diff", "--numstat"])
        .current_dir(repo_path)
        .output()
        && output.status.success()
    {
        parse_diff_numstat(&String::from_utf8_lossy(&output.stdout), &mut line_counts);
    }

    // Parse status output
    let mut changes: Vec<FileChangeInfo> = Vec::new();

    for line in status_str.lines() {
        if line.len() < 3 {
            continue;
        }

        let status_code = &line[0..2];
        let file_path = line[3..].trim().to_string();

        // Handle renamed files
        let file_path = if file_path.contains(" -> ") {
            file_path
                .split(" -> ")
                .last()
                .unwrap_or(&file_path)
                .to_string()
        } else {
            file_path
        };

        let status = match status_code {
            "M " | " M" | "MM" | "AM" => GitFileStatus::Modified,
            "A " | " A" => GitFileStatus::Added,
            "D " | " D" => GitFileStatus::Deleted,
            "R " | " R" => GitFileStatus::Renamed,
            "??" => GitFileStatus::Untracked,
            _ => continue,
        };

        let first_char = status_code.chars().next().unwrap_or(' ');
        let staged = first_char != ' ' && first_char != '?';

        // Get line counts, or count file lines for new/untracked files
        let (lines_added, lines_removed) = if let Some(&counts) = line_counts.get(&file_path) {
            counts
        } else if status == GitFileStatus::Untracked || status == GitFileStatus::Added {
            count_file_lines(&repo_path.join(&file_path))
        } else {
            (0, 0)
        };

        changes.push(FileChangeInfo {
            file_path,
            lines_added,
            lines_removed,
            status,
            staged,
        });
    }

    // Sort by status then path
    changes.sort_by(|a, b| {
        let status_order = |s: &GitFileStatus| match s {
            GitFileStatus::Modified => 0,
            GitFileStatus::Added => 1,
            GitFileStatus::Deleted => 2,
            GitFileStatus::Renamed => 3,
            GitFileStatus::Untracked => 4,
        };
        status_order(&a.status)
            .cmp(&status_order(&b.status))
            .then_with(|| a.file_path.cmp(&b.file_path))
    });

    changes
}

/// Parse git diff --numstat output
fn parse_diff_numstat(output: &str, line_counts: &mut HashMap<String, (usize, usize)>) {
    for line in output.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let added = parts[0].parse().unwrap_or(0);
            let removed = parts[1].parse().unwrap_or(0);
            let file = parts[2].to_string();
            let entry = line_counts.entry(file).or_insert((0, 0));
            entry.0 += added;
            entry.1 += removed;
        }
    }
}

/// Count lines in a file (for new/untracked files)
fn count_file_lines(path: &Path) -> (usize, usize) {
    use std::fs::File;
    use std::io::{BufRead, BufReader};

    // Skip directories - git status can report untracked directories with trailing slash
    if path.is_dir() {
        return (0, 0);
    }

    match File::open(path) {
        Ok(file) => {
            let reader = BufReader::new(file);
            let line_count = reader.lines().count();
            (line_count, 0)
        }
        Err(_) => (0, 0),
    }
}
