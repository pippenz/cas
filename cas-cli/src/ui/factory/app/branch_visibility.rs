use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
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
    refreshed_at: Option<Instant>,
    ttl: Duration,
    path_branches: HashMap<PathBuf, String>,
    epic_ahead_behind: HashMap<String, BranchAheadBehind>,
}

impl Default for BranchVisibilityCache {
    fn default() -> Self {
        Self {
            refreshed_at: None,
            ttl: DEFAULT_BRANCH_VISIBILITY_TTL,
            path_branches: HashMap::new(),
            epic_ahead_behind: HashMap::new(),
        }
    }
}

impl BranchVisibilityCache {
    pub(crate) fn is_fresh(&self, now: Instant) -> bool {
        self.refreshed_at
            .is_some_and(|refreshed| now.saturating_duration_since(refreshed) < self.ttl)
    }

    pub(crate) fn branch_for_path(&self, path: &Path) -> Option<&str> {
        self.path_branches.get(path).map(String::as_str)
    }

    pub(crate) fn epic_ahead_behind(&self, epic_id: &str) -> Option<&BranchAheadBehind> {
        self.epic_ahead_behind.get(epic_id)
    }

    pub(crate) fn refresh(
        &mut self,
        supervisor_path: &Path,
        worker_paths: &[(String, PathBuf)],
        epic_branches: &[(String, String)],
        now: Instant,
    ) {
        if self.is_fresh(now) {
            return;
        }

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
        let mut next_epic = HashMap::new();
        for (epic_id, branch) in epic_branches {
            if let Some((ahead, behind)) = ahead_behind(supervisor_path, branch, &trunk) {
                next_epic.insert(
                    epic_id.clone(),
                    BranchAheadBehind {
                        branch: branch.clone(),
                        ahead,
                        behind,
                    },
                );
            }
        }

        self.path_branches = next_path_branches;
        self.epic_ahead_behind = next_epic;
        self.refreshed_at = Some(now);
    }

    #[cfg(test)]
    fn with_ttl(ttl: Duration) -> Self {
        Self {
            ttl,
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn insert_path_branch(&mut self, path: PathBuf, branch: &str) {
        self.path_branches.insert(path, branch.to_string());
    }

    #[cfg(test)]
    fn mark_refreshed(&mut self, now: Instant) {
        self.refreshed_at = Some(now);
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

    #[test]
    fn cache_ttl_and_miss_degrade() {
        let now = Instant::now();
        let mut cache = BranchVisibilityCache::with_ttl(Duration::from_secs(10));
        let path = PathBuf::from("/tmp/worker");

        assert!(!cache.is_fresh(now));
        assert_eq!(cache.branch_for_path(&path), None);

        cache.insert_path_branch(path.clone(), "factory/worker");
        cache.mark_refreshed(now);
        assert!(cache.is_fresh(now + Duration::from_secs(9)));
        assert!(!cache.is_fresh(now + Duration::from_secs(10)));
        assert_eq!(cache.branch_for_path(&path), Some("factory/worker"));
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
}
