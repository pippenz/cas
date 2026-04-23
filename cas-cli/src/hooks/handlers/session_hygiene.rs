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

/// Extract the first `cas-xxxx` task id from `text`, if any.
///
/// Task ids in commit messages follow the canonical 4-char hex form used
/// throughout the codebase (e.g. `cas-4181`, `cas-a9ab`). Anything past 4
/// lowercase hex chars is rejected so arbitrary strings like `cas-foo`
/// are not falsely matched.
pub(crate) fn extract_task_id(text: &str) -> Option<String> {
    // Tiny hand-rolled scanner to avoid pulling in a regex dep just for one
    // match. Finds `cas-` then up to 4 hex chars followed by a non-hex
    // boundary (whitespace, punctuation, end-of-string).
    let bytes = text.as_bytes();
    let mut i = 0;
    let eq_ci = |a: u8, b: u8| a.eq_ignore_ascii_case(&b);
    while i + 8 <= bytes.len() {
        if eq_ci(bytes[i], b'c')
            && eq_ci(bytes[i + 1], b'a')
            && eq_ci(bytes[i + 2], b's')
            && bytes[i + 3] == b'-'
        {
            let start = i + 4;
            let mut end = start;
            while end < bytes.len() && end - start < 4 {
                let c = bytes[end];
                let is_hex =
                    c.is_ascii_digit() || (b'a'..=b'f').contains(&c) || (b'A'..=b'F').contains(&c);
                if !is_hex {
                    break;
                }
                end += 1;
            }
            if end - start == 4 {
                // Boundary check: next byte must be a non-hex, non-alnum
                // delimiter (or end of string).
                let terminates = end == bytes.len() || {
                    let c = bytes[end];
                    !(c.is_ascii_digit()
                        || (b'a'..=b'z').contains(&c)
                        || (b'A'..=b'Z').contains(&c))
                };
                if terminates {
                    let id = std::str::from_utf8(&bytes[i..end]).ok()?.to_ascii_lowercase();
                    return Some(id);
                }
            }
        }
        i += 1;
    }
    None
}

/// Ask git for the most recent commit that touched `file` and return the
/// first `cas-xxxx` task id from its subject+body, if any. Used to
/// attribute a modified WIP file to the task that most likely left it
/// behind. Returns `None` for untracked files, missing history, or when
/// the last commit message carries no task id.
pub fn attribute_file_to_task(repo: &Path, file: &str) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--format=%s%n%b", "--", file])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    if text.trim().is_empty() {
        return None;
    }
    extract_task_id(&text)
}

/// Maximum WIP entries the SessionStart banner itself renders inline.
///
/// Above this cap we print a "... and N more" suffix and direct the
/// supervisor to `gc_report` for the full list. The cap exists for two
/// reasons surfaced in review:
///
/// 1. Each tracked entry spawns `git log -1` for attribution. On a
///    pathological dirty tree (hundreds of files after a prior-session
///    crash) an uncapped banner turns the SessionStart hook into a
///    multi-second stall, which the user experiences as Claude Code
///    hanging on startup. 20 entries ≤ ~1s at 20–50ms per subprocess.
/// 2. The full SessionStart preview window is limited. Flooding it with
///    attribution lines buries the codemap/overview signals.
const WIP_BANNER_MAX_ENTRIES: usize = 20;

/// Render the SessionStart triage banner for a supervisor session, or
/// `None` when the worktree is clean / git is unavailable / cas_root
/// cannot be resolved. The banner is best-effort and must never fail a
/// session start.
///
/// The banner caps itself at [`WIP_BANNER_MAX_ENTRIES`] inline rows and
/// forwards the overflow count to `gc_report` so the supervisor can
/// paginate on demand — this bounds both `git log` subprocess fan-out
/// and the token budget the banner eats from the context window.
///
/// Output shape (example):
/// ```text
/// ⚠ Prior-factory WIP detected in main worktree (3 files, 2 modified, 1 untracked):
///   [modified]  src/foo.rs                (last touched by cas-a9ab)
///   [modified]  src/bar.rs                (last touched by cas-4181)
///   [untracked] src/baz.rs                (unattributed — no git history)
///
/// Triage BEFORE spawning workers: decide salvage / commit / discard.
/// Full history: .cas/logs/factory-session-{date}.log
/// ```
pub fn build_session_start_wip_banner(cas_root: &Path) -> Option<String> {
    let summary = wip_candidates(cas_root)?;
    if summary.is_clean() {
        return None;
    }
    let mut out = String::new();
    out.push_str(&format!(
        "⚠ Prior-factory WIP detected in main worktree ({} files, {} modified, {} untracked):\n",
        summary.entries.len(),
        summary.modified_count(),
        summary.untracked_count(),
    ));
    let total = summary.entries.len();
    let shown = total.min(WIP_BANNER_MAX_ENTRIES);
    for entry in summary.entries.iter().take(shown) {
        let attribution = if entry.is_untracked() {
            "(unattributed — no git history)".to_string()
        } else {
            match attribute_file_to_task(&summary.worktree, &entry.path) {
                Some(task_id) => format!("(last touched by {task_id})"),
                None => "(no task id in last commit)".to_string(),
            }
        };
        out.push_str(&format!(
            "  [{:15}] {}  {}\n",
            entry.label(),
            entry.path,
            attribution,
        ));
    }
    if total > shown {
        let extra = total - shown;
        out.push_str(&format!(
            "  ... and {extra} more — run `mcp__cas__coordination action=gc_report` for the full list.\n",
        ));
    }
    out.push_str(
        "\nTriage BEFORE spawning workers: decide salvage / commit / discard.\n\
         Full history: .cas/logs/factory-session-{date}.log (see cas-supervisor-checklist)\n",
    );
    Some(out)
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

    /// extract_task_id must pick the first canonical `cas-xxxx` (4 hex) token
    /// and reject non-hex / too-long / non-terminated variants so a commit
    /// subject like "refactor cas-module" does not falsely attribute.
    #[test]
    fn extract_task_id_accepts_canonical_and_rejects_garbage() {
        // Happy path: first 4-hex token wins, case-insensitive, boundary aware.
        assert_eq!(
            extract_task_id("fix(foo): ship cas-a9ab and follow-up cas-4181"),
            Some("cas-a9ab".to_string())
        );
        assert_eq!(
            extract_task_id("CAS-4181 uppercase"),
            Some("cas-4181".to_string())
        );
        assert_eq!(
            extract_task_id("see cas-d0f9."),
            Some("cas-d0f9".to_string())
        );

        // Non-hex characters in the 4-char window are rejected.
        assert_eq!(extract_task_id("cas-zzzz is fake"), None);
        // Too-long hex run (> 4 chars without a boundary) is rejected.
        assert_eq!(extract_task_id("cas-a9abc is nope"), None);
        // Non-boundary alphanumeric (e.g. cas-module) is rejected.
        assert_eq!(extract_task_id("refactor cas-module"), None);
        assert_eq!(extract_task_id("nothing here"), None);
    }

    /// attribute_file_to_task must resolve a modified file to the cas-id in
    /// the last commit that touched it. Confirms the core attribution primitive
    /// used by the SessionStart banner.
    #[test]
    fn attribute_file_to_task_finds_cas_id_in_last_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        fs::write(repo.join("a.txt"), "v1").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "a.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", "feat(a): initial ship (cas-a9ab)"])
            .status()
            .unwrap();

        assert_eq!(
            attribute_file_to_task(repo, "a.txt"),
            Some("cas-a9ab".to_string())
        );

        // An untracked file has no git history → None.
        fs::write(repo.join("new.txt"), "").unwrap();
        assert_eq!(attribute_file_to_task(repo, "new.txt"), None);

        // Commit without a cas-id still returns None (no false positives).
        fs::write(repo.join("b.txt"), "").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "b.txt"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", "chore(b): no task id here"])
            .status()
            .unwrap();
        assert_eq!(attribute_file_to_task(repo, "b.txt"), None);
    }

    /// On a pathological dirty tree (hundreds of files) the banner must
    /// cap its inline rows and direct the supervisor to `gc_report` for
    /// the overflow. Guards against SessionStart latency regressions —
    /// every rendered row spawns `git log -1`, so an uncapped banner
    /// stalls session boot (cas-aeec adversarial P1).
    #[test]
    fn build_session_start_wip_banner_caps_rows_on_large_wip_trees() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);
        // Untracked files don't need committing — faster than seeding
        // real history and exercises the 'unattributed' path which
        // still must be capped.
        let total = WIP_BANNER_MAX_ENTRIES + 7;
        for i in 0..total {
            fs::write(repo.join(format!("wip_{i:03}.tmp")), "").unwrap();
        }
        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();

        let banner =
            build_session_start_wip_banner(&cas_root).expect("banner for dirty tree");
        // The "[" opens each inline row. Count occurrences.
        let rows = banner.matches('[').count();
        assert_eq!(
            rows, WIP_BANNER_MAX_ENTRIES,
            "banner must cap inline rows at WIP_BANNER_MAX_ENTRIES, got {rows}"
        );
        assert!(
            banner.contains(&format!("and {} more", total - WIP_BANNER_MAX_ENTRIES)),
            "banner must announce the overflow count"
        );
        assert!(
            banner.contains("gc_report"),
            "overflow line must direct supervisor to gc_report"
        );
    }

    /// The SessionStart banner must surface the attribution line and the
    /// triage instruction; returning None on a clean tree prevents noise.
    #[test]
    fn build_session_start_wip_banner_renders_attribution_and_skips_clean() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_repo(repo);

        // Seed + commit + modify: modified entry should carry attribution.
        fs::write(repo.join("src.rs"), "v1").unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["add", "src.rs"])
            .status()
            .unwrap();
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["commit", "-q", "-m", "feat: ship (cas-4181)"])
            .status()
            .unwrap();
        fs::write(repo.join("src.rs"), "v2").unwrap();
        // Also drop an untracked file to exercise the unattributed branch.
        fs::write(repo.join("scratch.tmp"), "").unwrap();

        let cas_root = repo.join(".cas");
        fs::create_dir_all(&cas_root).unwrap();

        let banner = build_session_start_wip_banner(&cas_root)
            .expect("banner rendered for dirty worktree");
        assert!(banner.contains("Prior-factory WIP detected"));
        assert!(banner.contains("src.rs"));
        assert!(banner.contains("(last touched by cas-4181)"));
        assert!(banner.contains("scratch.tmp"));
        assert!(banner.contains("unattributed"));
        assert!(banner.contains("Triage BEFORE spawning workers"));

        // Clean the tree → banner suppressed, no noise on normal sessions.
        fs::remove_file(repo.join("scratch.tmp")).unwrap();
        fs::write(repo.join("src.rs"), "v1").unwrap();
        assert!(
            build_session_start_wip_banner(&cas_root).is_none(),
            "clean tree must not emit a banner"
        );
    }
}
