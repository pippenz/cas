use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const DEFAULT_BRANCH_VISIBILITY_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BranchAheadBehind {
    pub(crate) branch: String,
    pub(crate) ahead: u32,
    pub(crate) behind: u32,
}

#[derive(Debug, Clone)]
pub(crate) struct BranchVisibilityCache {
    ttl: Duration,
    snapshot: Arc<Mutex<BranchVisibilitySnapshot>>,
    refresh_in_flight: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Default)]
struct BranchVisibilitySnapshot {
    refreshed_at: Option<Instant>,
    path_branches: HashMap<PathBuf, String>,
    epic_ahead_behind: HashMap<String, BranchAheadBehind>,
}

impl Default for BranchVisibilityCache {
    fn default() -> Self {
        Self {
            ttl: DEFAULT_BRANCH_VISIBILITY_TTL,
            snapshot: Arc::new(Mutex::new(BranchVisibilitySnapshot::default())),
            refresh_in_flight: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl BranchVisibilityCache {
    pub(crate) fn is_fresh(&self, now: Instant) -> bool {
        self.snapshot
            .lock()
            .ok()
            .and_then(|snapshot| snapshot.refreshed_at)
            .is_some_and(|refreshed| now.saturating_duration_since(refreshed) < self.ttl)
    }

    pub(crate) fn branch_for_path(&self, path: &Path) -> Option<String> {
        self.snapshot.lock().ok()?.path_branches.get(path).cloned()
    }

    pub(crate) fn epic_ahead_behind(&self, epic_id: &str) -> Option<BranchAheadBehind> {
        self.snapshot
            .lock()
            .ok()?
            .epic_ahead_behind
            .get(epic_id)
            .cloned()
    }

    pub(crate) fn refresh(
        &self,
        supervisor_path: &Path,
        worker_paths: &[(String, PathBuf)],
        epic_branches: &[(String, String)],
        now: Instant,
    ) {
        if self.is_fresh(now) {
            return;
        }
        if self.refresh_in_flight.swap(true, Ordering::AcqRel) {
            return;
        }

        let supervisor_path = supervisor_path.to_path_buf();
        let worker_paths = worker_paths.to_vec();
        let epic_branches = epic_branches.to_vec();
        let snapshot = Arc::clone(&self.snapshot);
        let refresh_in_flight = Arc::clone(&self.refresh_in_flight);

        std::thread::spawn(move || {
            let next = collect_snapshot(&supervisor_path, &worker_paths, &epic_branches, now);
            if let Ok(mut current) = snapshot.lock() {
                *current = next;
            }
            refresh_in_flight.store(false, Ordering::Release);
        });
    }

    #[cfg(test)]
    fn refresh_now_for_test(
        &self,
        supervisor_path: &Path,
        worker_paths: &[(String, PathBuf)],
        epic_branches: &[(String, String)],
        now: Instant,
    ) {
        let next = collect_snapshot(supervisor_path, worker_paths, epic_branches, now);
        if let Ok(mut snapshot) = self.snapshot.lock() {
            *snapshot = next;
        }
    }

    #[cfg(test)]
    fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn insert_path_branch(&self, path: PathBuf, branch: &str) {
        self.snapshot
            .lock()
            .unwrap()
            .path_branches
            .insert(path, branch.to_string());
    }

    #[cfg(test)]
    pub(crate) fn insert_epic_ahead_behind(
        &self,
        epic_id: &str,
        branch: &str,
        ahead: u32,
        behind: u32,
    ) {
        self.snapshot.lock().unwrap().epic_ahead_behind.insert(
            epic_id.to_string(),
            BranchAheadBehind {
                branch: branch.to_string(),
                ahead,
                behind,
            },
        );
    }

    #[cfg(test)]
    fn mark_refreshed(&self, now: Instant) {
        self.snapshot.lock().unwrap().refreshed_at = Some(now);
    }
}

fn collect_snapshot(
    supervisor_path: &Path,
    worker_paths: &[(String, PathBuf)],
    epic_branches: &[(String, String)],
    now: Instant,
) -> BranchVisibilitySnapshot {
    let mut next_path_branches = HashMap::new();
    if let Some(branch) = current_branch(supervisor_path) {
        next_path_branches.insert(supervisor_path.to_path_buf(), branch);
    }
    for (_, path) in worker_paths {
        if let Some(branch) = current_branch(path) {
            next_path_branches.insert(path.clone(), branch);
        }
    }

    let trunk = detect_trunk(supervisor_path);
    let mut next_epic_ahead_behind = HashMap::new();
    for (epic_id, branch) in epic_branches {
        if let Some((ahead, behind)) = ahead_behind(supervisor_path, branch, &trunk) {
            next_epic_ahead_behind.insert(
                epic_id.clone(),
                BranchAheadBehind {
                    branch: branch.clone(),
                    ahead,
                    behind,
                },
            );
        }
    }

    BranchVisibilitySnapshot {
        refreshed_at: Some(now),
        path_branches: next_path_branches,
        epic_ahead_behind: next_epic_ahead_behind,
    }
}

pub(crate) fn branch_for_worker_title(
    cache: &BranchVisibilityCache,
    worktree_path: Option<&Path>,
    project_dir: &Path,
) -> Option<String> {
    match worktree_path {
        Some(path) => cache.branch_for_path(path),
        None => cache.branch_for_path(project_dir),
    }
}

pub(crate) fn truncate_branch_middle(branch: &str, max_chars: usize) -> String {
    let len = branch.chars().count();
    if len <= max_chars {
        return branch.to_string();
    }
    if max_chars <= 1 {
        return "…".to_string();
    }
    if max_chars <= 4 {
        return branch
            .chars()
            .take(max_chars - 1)
            .chain("…".chars())
            .collect();
    }

    let side = (max_chars - 1) / 2;
    let tail = max_chars - 1 - side;
    let head: String = branch.chars().take(side).collect();
    let suffix: String = branch
        .chars()
        .rev()
        .take(tail)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{suffix}")
}

pub(crate) fn format_pane_title_with_branch(name: &str, branch: Option<&str>) -> String {
    match branch {
        Some(branch) if !branch.is_empty() => {
            format!("{name} [{}]", truncate_branch_middle(branch, 28))
        }
        _ => name.to_string(),
    }
}

fn current_branch(path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!branch.is_empty() && branch != "HEAD").then_some(branch)
}

fn detect_trunk(path: &Path) -> String {
    let origin_head = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD", "--short"])
        .current_dir(path)
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|branch| !branch.is_empty());

    origin_head
        .or_else(|| ref_exists(path, "refs/remotes/origin/main").then(|| "origin/main".to_string()))
        .unwrap_or_else(|| "main".to_string())
}

fn ref_exists(path: &Path, reference: &str) -> bool {
    Command::new("git")
        .args(["show-ref", "--verify", "--quiet", reference])
        .current_dir(path)
        .status()
        .is_ok_and(|status| status.success())
}

fn ahead_behind(path: &Path, branch: &str, base: &str) -> Option<(u32, u32)> {
    let output = Command::new("git")
        .args([
            "rev-list",
            "--left-right",
            "--count",
            &format!("{base}...{branch}"),
        ])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut parts = stdout.split_whitespace();
    let behind = parts.next()?.parse::<u32>().ok()?;
    let ahead = parts.next()?.parse::<u32>().ok()?;
    Some((ahead, behind))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn git(repo: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?} failed to spawn: {err}"));
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn commit_file(repo: &Path, name: &str, contents: &str) {
        fs::write(repo.join(name), contents).unwrap();
        git(repo, &["add", name]);
        git(repo, &["commit", "-m", name]);
    }

    fn scratch_diverged_repo() -> tempfile::TempDir {
        let tempdir = tempfile::TempDir::new().unwrap();
        let repo = tempdir.path();
        git(repo, &["init", "-b", "main"]);
        git(repo, &["config", "user.email", "test@example.invalid"]);
        git(repo, &["config", "user.name", "Test User"]);
        commit_file(repo, "base.txt", "base");
        git(repo, &["checkout", "-b", "topic"]);
        commit_file(repo, "topic.txt", "topic");
        git(repo, &["checkout", "main"]);
        commit_file(repo, "main-1.txt", "main 1");
        commit_file(repo, "main-2.txt", "main 2");
        commit_file(repo, "main-3.txt", "main 3");
        tempdir
    }

    #[test]
    fn cache_ttl_and_miss_degrade() {
        let now = Instant::now();
        let cache = BranchVisibilityCache::with_ttl(Duration::from_secs(10));
        let path = PathBuf::from("/tmp/worker");

        assert!(!cache.is_fresh(now));
        assert_eq!(cache.branch_for_path(&path), None);

        cache.insert_path_branch(path.clone(), "factory/worker");
        cache.mark_refreshed(now);
        assert!(cache.is_fresh(now + Duration::from_secs(9)));
        assert!(!cache.is_fresh(now + Duration::from_secs(10)));
        assert_eq!(
            cache.branch_for_path(&path).as_deref(),
            Some("factory/worker")
        );
        assert_eq!(cache.branch_for_path(Path::new("/tmp/missing")), None);
    }

    #[test]
    fn branch_title_format_truncates_middle() {
        assert_eq!(
            format_pane_title_with_branch("worker", Some("factory/short")),
            "worker [factory/short]"
        );
        assert_eq!(format_pane_title_with_branch("worker", None), "worker");

        let title = format_pane_title_with_branch(
            "worker",
            Some("factory/extremely-long-branch-name-for-worker"),
        );
        assert!(title.starts_with("worker [factory/extre"));
        assert!(title.ends_with("r-worker]"));
        assert!(title.contains('…'));
    }

    #[test]
    fn worker_title_cache_miss_does_not_fall_back_to_supervisor_branch() {
        let cache = BranchVisibilityCache::default();
        let project_dir = PathBuf::from("/repo");
        let worker_dir = PathBuf::from("/repo/.cas/worktrees/worker");
        cache.insert_path_branch(project_dir.clone(), "main");

        assert_eq!(
            branch_for_worker_title(&cache, Some(&worker_dir), &project_dir),
            None
        );
        assert_eq!(
            branch_for_worker_title(&cache, None, &project_dir).as_deref(),
            Some("main")
        );
    }

    #[test]
    fn ahead_behind_orientation_matches_git_left_right_count() {
        let tempdir = scratch_diverged_repo();

        assert_eq!(ahead_behind(tempdir.path(), "topic", "main"), Some((1, 3)));
    }

    #[test]
    fn refresh_collects_current_branches_and_focused_epic_ahead_behind() {
        let tempdir = scratch_diverged_repo();
        let now = Instant::now();
        let cache = BranchVisibilityCache::default();

        cache.refresh_now_for_test(
            tempdir.path(),
            &[],
            &[("cas-epic".to_string(), "topic".to_string())],
            now,
        );

        assert_eq!(
            cache.branch_for_path(tempdir.path()).as_deref(),
            Some("main")
        );
        assert_eq!(
            cache.epic_ahead_behind("cas-epic"),
            Some(BranchAheadBehind {
                branch: "topic".to_string(),
                ahead: 1,
                behind: 3,
            })
        );
        assert!(cache.is_fresh(now));
    }
}
