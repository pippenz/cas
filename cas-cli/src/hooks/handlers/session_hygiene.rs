//! Factory session hygiene — surface and record the main worktree's state
//! around session boundaries so supervisors can attribute leftover
//! uncommitted work from crashed/interrupted prior factory sessions.
//!
//! Two features live here:
//!
//! 1. A **session-end manifest** appended to
//!    `.cas/logs/factory-session-{YYYY-MM-DD}.log`, capturing
//!    `git status --porcelain` of the main worktree when a session ends.
//!    This gives the next supervisor a durable record of what was left
//!    behind (see task cas-a9ab, report §3).
//!
//! 2. A **WIP candidates** helper used by `coordination action=gc_report`
//!    (and consumable by `SessionStart` triage for task cas-aeec) that
//!    lists uncommitted entries in the main worktree so they can be
//!    surfaced — never auto-deleted.
//!
//! The module is best-effort: I/O and git failures are swallowed because
//! hygiene instrumentation must never break a session-end hook.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Single `git status --porcelain` entry.
///
/// `status` is the raw two-char porcelain code (e.g. `"??"`, `" M"`, `"M "`,
/// `"A "`). `path` is the file path relative to the worktree root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PorcelainEntry {
    pub status: String,
    pub path: String,
}

impl PorcelainEntry {
    /// True if this is an untracked file (`??` status).
    pub fn is_untracked(&self) -> bool {
        self.status.starts_with("??")
    }

    /// Short human label for the entry's state.
    pub fn label(&self) -> &'static str {
        match self.status.as_str() {
            "??" => "untracked",
            " M" => "modified",
            "M " | "MM" | "AM" => "modified-staged",
            "A " => "added",
            "D " | " D" => "deleted",
            _ => "changed",
        }
    }
}

/// Resolve the main repo root for this CAS installation.
///
/// By convention, the CAS root sits at `<repo>/.cas`, so the main
/// worktree is its parent directory. Returns `None` if the layout is
/// unexpected.
pub fn main_worktree_path(cas_root: &Path) -> Option<PathBuf> {
    let repo_adjacent = cas_root.parent()?;

    // Ask git for the *common* git dir; in a linked worktree this points at
    // the main repo's `.git`, whereas `--git-dir` would point at
    // `.git/worktrees/<name>`. The main worktree then lives one dir above
    // the common dir (assuming the normal `<repo>/.git` layout). Falls
    // back to `cas_root.parent()` when git is unavailable or the layout is
    // unexpected, preserving the prior best-effort behaviour.
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_adjacent)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()?;
    if !out.status.success() {
        return Some(repo_adjacent.to_path_buf());
    }
    let common = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if common.is_empty() {
        return Some(repo_adjacent.to_path_buf());
    }
    let common_path = PathBuf::from(common);
    // `.git` common dir → main worktree is its parent.
    if common_path.file_name().and_then(|s| s.to_str()) == Some(".git") {
        if let Some(parent) = common_path.parent() {
            return Some(parent.to_path_buf());
        }
    }
    // Bare repo or unusual layout — give up safely.
    Some(repo_adjacent.to_path_buf())
}

/// Run `git status --porcelain=v1` in `repo` and parse the output.
///
/// Returns `None` if git is unavailable, the directory is not a repo,
/// or the command fails. On success, returns an empty vec for a clean
/// tree.
pub fn porcelain_status(repo: &Path) -> Option<Vec<PorcelainEntry>> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["status", "--porcelain=v1"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries = Vec::new();
    for line in text.lines() {
        if line.len() < 3 {
            continue;
        }
        // Porcelain v1: "XY path", where XY are exactly 2 chars and then a space.
        let (status, rest) = line.split_at(2);
        // `rest` starts with a space; strip it.
        let path = rest.trim_start().to_string();
        entries.push(PorcelainEntry {
            status: status.to_string(),
            path,
        });
    }
    Some(entries)
}

/// Append a session-end manifest entry to
/// `<cas_root>/logs/factory-session-{YYYY-MM-DD}.log`.
///
/// The manifest is human-readable YAML-ish text, one block per session end,
/// separated by `---`. The block always includes the session id, the agent
/// (if known), the worktree path, and a porcelain status dump. A clean
/// worktree is recorded as `git_status: (clean)` for later auditing.
///
/// Returns the log path on success, or `None` if the worktree could not be
/// resolved or the git probe failed. I/O errors are swallowed by design.
pub fn write_session_end_manifest(
    cas_root: &Path,
    session_id: &str,
    agent_name: Option<&str>,
    agent_role: Option<&str>,
) -> Option<PathBuf> {
    let repo = main_worktree_path(cas_root)?;
    let entries = porcelain_status(&repo)?;

    let now = chrono::Utc::now();
    let log_dir = cas_root.join("logs");
    std::fs::create_dir_all(&log_dir).ok()?;
    let log_path = log_dir.join(format!("factory-session-{}.log", now.format("%Y-%m-%d")));

    let mut body = String::new();
    body.push_str("---\n");
    body.push_str(&format!("session_end: {}\n", now.to_rfc3339()));
    body.push_str(&format!("session_id: {session_id}\n"));
    body.push_str(&format!(
        "agent: {} ({})\n",
        agent_name.unwrap_or("unknown"),
        agent_role.unwrap_or("unknown"),
    ));
    body.push_str(&format!("worktree: {}\n", repo.display()));
    if entries.is_empty() {
        body.push_str("git_status: (clean)\n");
    } else {
        body.push_str(&format!("git_status: {} entries\n", entries.len()));
        for e in &entries {
            body.push_str(&format!("  {} {}\n", e.status, e.path));
        }
    }

    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()?;
    f.write_all(body.as_bytes()).ok()?;
    Some(log_path)
}

/// Summary of WIP candidates in the main worktree.
///
/// Returned by [`wip_candidates`] so callers can render a concise report
/// without re-running git. `entries` preserves the porcelain output order.
#[derive(Debug, Clone, Default)]
pub struct WipSummary {
    pub worktree: PathBuf,
    pub entries: Vec<PorcelainEntry>,
}

impl WipSummary {
    pub fn is_clean(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn untracked_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_untracked()).count()
    }

    pub fn modified_count(&self) -> usize {
        self.entries.iter().filter(|e| !e.is_untracked()).count()
    }
}

/// Inspect the main worktree and return a [`WipSummary`].
///
/// Returns `None` if the worktree path can't be resolved or git is
/// unavailable. Clean trees return `Some(WipSummary { entries: [] })`
/// so callers can still report "clean".
pub fn wip_candidates(cas_root: &Path) -> Option<WipSummary> {
    let repo = main_worktree_path(cas_root)?;
    let entries = porcelain_status(&repo)?;
    Some(WipSummary {
        worktree: repo,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn init_repo(dir: &Path) {
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["init", "-q", "-b", "main"])
            .status();
        // Minimal identity so commits don't fail.
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.email", "test@example.com"])
            .status();
        let _ = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["config", "user.name", "test"])
            .status();
    }

    #[test]
    fn porcelain_clean_tree_returns_empty() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());
        // Empty repo has no changes.
        let entries = porcelain_status(tmp.path()).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn porcelain_reports_untracked_and_modified() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo(tmp.path());

        // Commit an initial file.
        fs::write(tmp.path().join("a.txt"), "hello").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["add", "a.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(tmp.path())
            .args(["commit", "-q", "-m", "init"])
            .status()
            .unwrap();

        // Modify committed file and drop an untracked one.
        fs::write(tmp.path().join("a.txt"), "changed").unwrap();
        fs::write(tmp.path().join("b.txt"), "new").unwrap();

        let entries = porcelain_status(tmp.path()).unwrap();
        let untracked = entries.iter().filter(|e| e.is_untracked()).count();
        let modified = entries.iter().filter(|e| !e.is_untracked()).count();
        assert_eq!(untracked, 1);
        assert_eq!(modified, 1);
    }

    #[test]
    fn write_session_end_manifest_appends_to_daily_log() {
        let tmp = tempfile::tempdir().unwrap();
        // cas_root lives *inside* the repo, so repo == cas_root.parent().
        let repo = tmp.path();
        init_repo(repo);
        fs::write(repo.join("leftover.txt"), "oops").unwrap();

        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();

        let path = write_session_end_manifest(
            &cas_root,
            "session-abc",
            Some("lively-pelican-94"),
            Some("worker"),
        )
        .expect("manifest written");

        assert!(path.exists());
        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("session_id: session-abc"));
        assert!(contents.contains("lively-pelican-94"));
        assert!(contents.contains("leftover.txt"));
    }

    #[test]
    fn wip_candidates_surfaces_untracked() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        fs::write(repo.join("wip.rs"), "// todo").unwrap();

        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();

        let summary = wip_candidates(&cas_root).expect("summary");
        assert!(!summary.is_clean());
        assert_eq!(summary.untracked_count(), 1);
        assert_eq!(summary.modified_count(), 0);
    }

    /// Table-drive `label()` across every documented porcelain code so a silent
    /// rename of an arm (e.g. 'modified-staged' → 'staged') fails loudly.
    #[test]
    fn porcelain_entry_label_covers_documented_codes() {
        let cases: &[(&str, &str)] = &[
            ("??", "untracked"),
            (" M", "modified"),
            ("M ", "modified-staged"),
            ("MM", "modified-staged"),
            ("AM", "modified-staged"),
            ("A ", "added"),
            ("D ", "deleted"),
            (" D", "deleted"),
            ("R ", "changed"), // Rename falls through today; guard arm.
            ("UU", "changed"), // Unmerged falls through today.
        ];
        for (code, expected) in cases {
            let entry = PorcelainEntry {
                status: (*code).to_string(),
                path: "x".into(),
            };
            assert_eq!(
                entry.label(),
                *expected,
                "label mismatch for porcelain code {code:?}"
            );
        }
    }

    /// Multiple `write_session_end_manifest` calls in the same day must append
    /// rather than overwrite — the daily log is the cross-session breadcrumb
    /// trail; losing history silently defeats the feature.
    #[test]
    fn manifest_is_append_only_across_multiple_sessions_same_day() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();

        let p1 = write_session_end_manifest(&cas_root, "sess-one", None, None)
            .expect("first manifest");
        let p2 = write_session_end_manifest(&cas_root, "sess-two", Some("worker-b"), Some("worker"))
            .expect("second manifest");
        assert_eq!(p1, p2, "same daily log path expected");

        let body = fs::read_to_string(&p1).unwrap();
        assert!(body.contains("session_id: sess-one"));
        assert!(body.contains("session_id: sess-two"));
        assert!(body.contains("agent: unknown (unknown)")); // first call, None/None
        assert!(body.contains("agent: worker-b (worker)")); // second call
        assert_eq!(
            body.matches("---").count(),
            2,
            "each session-end must produce its own '---' block"
        );
    }

    /// A clean worktree records `git_status: (clean)` so audits can tell the
    /// difference between "nothing was wrong" and "manifest never wrote".
    #[test]
    fn manifest_records_clean_tree_marker() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        // Commit so the tree is fully clean (empty repo also counts, but
        // committing exercises the "tree exists + clean" code path).
        fs::write(repo.join(".gitkeep"), "").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", ".gitkeep"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", "init"])
            .status()
            .unwrap();

        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();
        let log = write_session_end_manifest(&cas_root, "sess-clean", None, None)
            .expect("manifest written");
        let body = fs::read_to_string(&log).unwrap();
        assert!(
            body.contains("git_status: (clean)"),
            "clean worktree should be recorded, got: {body}"
        );
    }

    /// When `cas_root` lives under a linked worktree (the factory layout:
    /// `<repo>/.cas/worktrees/<name>/.cas`), `main_worktree_path` must resolve
    /// to the main repo — not the linked worktree — otherwise the hygiene
    /// manifest attributes the worker's own WIP as "main worktree" and
    /// inverts the supervisor triage promise (cas-a9ab adversarial finding).
    #[test]
    fn main_worktree_path_resolves_to_main_repo_from_linked_worktree() {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        fs::create_dir_all(&main).unwrap();
        init_repo(&main);
        // A commit is required so the repo has HEAD before linking a worktree.
        fs::write(main.join("seed.txt"), "").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["add", "seed.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["commit", "-q", "-m", "seed"])
            .status()
            .unwrap();

        let linked = tmp.path().join("linked");
        let status = Command::new("git")
            .arg("-C")
            .arg(&main)
            .args(["worktree", "add", "-b", "feature"])
            .arg(&linked)
            .status()
            .unwrap();
        assert!(status.success(), "git worktree add must succeed for this test");

        // Worker-style layout: cas_root is <linked>/.cas.
        let linked_cas = linked.join(".cas");
        fs::create_dir_all(&linked_cas).unwrap();

        let resolved = main_worktree_path(&linked_cas).expect("main path resolved");
        assert_eq!(
            resolved.canonicalize().unwrap(),
            main.canonicalize().unwrap(),
            "linked-worktree cas_root must resolve upward to the main repo, got {resolved:?}"
        );
    }
}
