//! Fork-safe base-SHA resolution for code-review diff computation.
//!
//! The multi-persona code reviewer (Phase 1 Subsystem A) needs a reliable
//! "base" revision to diff the current work against. In a normal local
//! workflow that is usually `origin/main`, but in fork PRs, detached
//! checkouts, worktrees, or CI contexts this can be surprisingly hard to
//! pin down. [`resolve`] tries a prioritized list of strategies and
//! returns the first one that succeeds, or a [`BaseShaError`] describing
//! every branch that was attempted.
//!
//! Strategies, in priority order:
//!
//! 1. **Caller override** — if `base_override` is `Some(s)` and the ref
//!    resolves, we use it. This is the explicit escape hatch for callers
//!    who already know the right base.
//! 2. **`GITHUB_BASE_REF` env var** — set by GitHub Actions on pull
//!    requests (e.g. `main`). We try `origin/<value>` first, then the raw
//!    value, so it works both in CI and when the caller exported it
//!    locally.
//! 3. **`origin/HEAD` symbolic ref** — `git symbolic-ref
//!    refs/remotes/origin/HEAD` returns the remote's default branch. This
//!    is the most reliable "what does upstream consider main" signal.
//! 4. **`gh repo view`** — if the GitHub CLI is on `PATH`, ask it for the
//!    default branch name and try `origin/<name>`.
//! 5. **Common branch fallbacks** — try `origin/main`, `origin/master`,
//!    `origin/develop`, `origin/trunk` in order. The first one that
//!    resolves wins.
//! 6. **`HEAD~1`** — last-resort fallback so reviews can still produce a
//!    (possibly noisy) diff in freshly-cloned repos or topic branches that
//!    have no remote tracking info. A `warn!` is logged so callers know
//!    they did not get a "real" base.
//!
//! The function returns the *resolved commit SHA* (a 40-char hex string),
//! not the symbolic name, so downstream consumers can diff
//! deterministically even if refs move.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;
use tracing::warn;

/// Errors returned by [`resolve`].
#[derive(Debug, Error)]
pub enum BaseShaError {
    /// The target directory is not inside a git working tree.
    #[error("not a git repository: {path}")]
    NotAGitRepo { path: String },

    /// Every resolution strategy was attempted and none produced a usable
    /// base SHA. `attempts` contains one line per strategy with the
    /// reason it failed, suitable for logging directly.
    #[error("could not resolve a base SHA in {path}:\n{}", attempts.join("\n"))]
    AllStrategiesFailed {
        path: String,
        attempts: Vec<String>,
    },
}

/// Resolve a base SHA for a fork-safe code-review diff.
///
/// `base_override` — optional caller-supplied ref or SHA to try first.
/// `project_root` — directory inside the git worktree to resolve against.
///
/// Returns the full 40-char commit SHA of the chosen base.
pub fn resolve(
    base_override: Option<&str>,
    project_root: &Path,
) -> Result<String, BaseShaError> {
    let root = project_root.to_path_buf();
    let path_str = root.display().to_string();

    // Fast sanity check: are we even inside a git repo? This lets us
    // distinguish "not a repo" (caller passed the wrong path) from "repo
    // but every strategy failed" (weird state worth dumping attempts).
    if !is_git_repo(&root) {
        return Err(BaseShaError::NotAGitRepo { path: path_str });
    }

    let mut attempts: Vec<String> = Vec::new();

    // 1. Caller override.
    if let Some(override_ref) = base_override.map(str::trim).filter(|s| !s.is_empty()) {
        match rev_parse(&root, override_ref) {
            Some(sha) => return Ok(sha),
            None => attempts.push(format!(
                "1. caller override '{override_ref}': ref did not resolve"
            )),
        }
    } else {
        attempts.push("1. caller override: not provided".to_string());
    }

    // 2. GITHUB_BASE_REF env var (GitHub Actions PR context, or manual export).
    match std::env::var("GITHUB_BASE_REF") {
        Ok(value) if !value.trim().is_empty() => {
            let value = value.trim().to_string();
            // Prefer the remote-tracking form first; fall back to the bare name.
            let remote_form = format!("origin/{value}");
            if let Some(sha) = rev_parse(&root, &remote_form) {
                return Ok(sha);
            }
            if let Some(sha) = rev_parse(&root, &value) {
                return Ok(sha);
            }
            attempts.push(format!(
                "2. GITHUB_BASE_REF='{value}': neither 'origin/{value}' nor '{value}' resolved"
            ));
        }
        _ => attempts.push("2. GITHUB_BASE_REF: unset".to_string()),
    }

    // 3. git symbolic-ref refs/remotes/origin/HEAD
    match symbolic_ref_origin_head(&root) {
        Some(remote_ref) => {
            if let Some(sha) = rev_parse(&root, &remote_ref) {
                return Ok(sha);
            }
            attempts.push(format!(
                "3. origin/HEAD: symbolic-ref returned '{remote_ref}' but it did not resolve"
            ));
        }
        None => attempts.push(
            "3. origin/HEAD: symbolic-ref not set (run `git remote set-head origin -a`)"
                .to_string(),
        ),
    }

    // 4. gh repo view --json defaultBranchRef -q .defaultBranchRef.name
    //    Only attempted if `gh` is on PATH, per scope boundary.
    if gh_on_path() {
        match gh_default_branch(&root) {
            Some(name) => {
                let remote_form = format!("origin/{name}");
                if let Some(sha) = rev_parse(&root, &remote_form) {
                    return Ok(sha);
                }
                attempts.push(format!(
                    "4. gh default branch '{name}': 'origin/{name}' did not resolve"
                ));
            }
            None => attempts
                .push("4. gh default branch: gh on PATH but query failed".to_string()),
        }
    } else {
        attempts.push("4. gh default branch: gh CLI not on PATH".to_string());
    }

    // 5. Common branch names against origin.
    const COMMON: &[&str] = &["main", "master", "develop", "trunk"];
    let mut common_fail = Vec::new();
    for name in COMMON {
        let remote_form = format!("origin/{name}");
        if let Some(sha) = rev_parse(&root, &remote_form) {
            return Ok(sha);
        }
        common_fail.push(remote_form);
    }
    attempts.push(format!(
        "5. common branches: none of [{}] resolved",
        common_fail.join(", ")
    ));

    // 6. Last resort: HEAD~1 with a warning.
    if let Some(sha) = rev_parse(&root, "HEAD~1") {
        warn!(
            base_sha = %sha,
            project_root = %path_str,
            "base_sha::resolve falling back to HEAD~1; no real upstream base could be determined"
        );
        return Ok(sha);
    }
    attempts.push(
        "6. HEAD~1: could not resolve (repo may have zero or one commits)".to_string(),
    );

    Err(BaseShaError::AllStrategiesFailed {
        path: path_str,
        attempts,
    })
}

/// Return true iff `dir` is inside a git working tree.
fn is_git_repo(dir: &Path) -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(dir)
        .output()
        .map(|out| out.status.success() && String::from_utf8_lossy(&out.stdout).trim() == "true")
        .unwrap_or(false)
}

/// Resolve a ref or SHA to a full commit hash. Returns `None` on any
/// failure (ref doesn't exist, git not on PATH, etc.) so callers can
/// cleanly fall through to the next strategy.
fn rev_parse(dir: &Path, reference: &str) -> Option<String> {
    // `--verify` makes git fail rather than guess when the ref is
    // ambiguous or missing. `^{commit}` forces peel-to-commit so tags
    // and branches both produce a commit SHA.
    let spec = format!("{reference}^{{commit}}");
    let output = Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", &spec])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        Some(sha)
    } else {
        None
    }
}

/// Read `refs/remotes/origin/HEAD` and return e.g. `origin/main`, or
/// `None` if the symbolic ref is not set.
fn symbolic_ref_origin_head(dir: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "refs/remotes/origin/HEAD"])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // git prints e.g. "refs/remotes/origin/main"
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    raw.strip_prefix("refs/remotes/")
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// Is the `gh` CLI on PATH?
fn gh_on_path() -> bool {
    // `gh --version` is cheap and doesn't touch the network.
    Command::new("gh")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Ask gh for the repo's default branch. Returns `None` on any failure.
fn gh_default_branch(dir: &Path) -> Option<String> {
    let output = Command::new("gh")
        .args([
            "repo",
            "view",
            "--json",
            "defaultBranchRef",
            "-q",
            ".defaultBranchRef.name",
        ])
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

// Exposed so tests can reference the module path consistently even if
// the public API is later narrowed.
#[allow(dead_code)]
pub(crate) fn _project_root(p: &Path) -> PathBuf {
    p.to_path_buf()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::TempDir;

    /// Shell out to git with deterministic author/committer so commit
    /// hashes don't depend on the host environment.
    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed in {dir:?}");
    }

    /// Initialize a repo with two commits on `main` so HEAD~1 exists.
    fn init_repo() -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "one\n").unwrap();
        git(p, &["add", "a.txt"]);
        git(p, &["commit", "-q", "-m", "first"]);
        std::fs::write(p.join("a.txt"), "two\n").unwrap();
        git(p, &["add", "a.txt"]);
        git(p, &["commit", "-q", "-m", "second"]);
        dir
    }

    /// Add a fake `origin` remote pointing at a second bare repo so
    /// `origin/<branch>` refs actually exist. Returns the bare repo dir
    /// so the caller keeps it alive.
    fn add_origin_with_branches(work: &Path, branches: &[&str]) -> TempDir {
        let bare = tempfile::tempdir().unwrap();
        git(bare.path(), &["init", "-q", "--bare"]);
        git(
            work,
            &["remote", "add", "origin", bare.path().to_str().unwrap()],
        );
        // Push each requested branch to origin. The first one in the list
        // is pushed from main; additional ones are created as copies.
        for (i, br) in branches.iter().enumerate() {
            if i == 0 {
                git(work, &["push", "-q", "origin", &format!("main:{br}")]);
            } else {
                // Create a local branch pointing at main and push it.
                git(work, &["branch", br, "main"]);
                git(work, &["push", "-q", "origin", br]);
                git(work, &["branch", "-D", br]);
            }
        }
        bare
    }

    /// Lock a scope so the tests that mutate `GITHUB_BASE_REF` don't race
    /// against each other. `std::env` is process-global.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn caller_override_wins() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        let dir = init_repo();
        let p = dir.path();

        // HEAD~1 is a real commit; use it as the override target.
        let head_1 = rev_parse(p, "HEAD~1").expect("HEAD~1 resolves");
        let got = resolve(Some("HEAD~1"), p).expect("resolve ok");
        assert_eq!(got, head_1);
    }

    #[test]
    fn github_base_ref_env_var_path() {
        let _g = env_lock();
        let dir = init_repo();
        let p = dir.path();
        let _bare = add_origin_with_branches(p, &["main"]);

        let expected = rev_parse(p, "origin/main").expect("origin/main resolves");

        unsafe {
            std::env::set_var("GITHUB_BASE_REF", "main");
        }
        let got = resolve(None, p).expect("resolve ok");
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }

        assert_eq!(got, expected);
    }

    #[test]
    fn origin_head_symbolic_ref_path() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        let dir = init_repo();
        let p = dir.path();
        let _bare = add_origin_with_branches(p, &["main"]);

        // Set origin/HEAD -> origin/main.
        git(
            p,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );

        let expected = rev_parse(p, "origin/main").expect("origin/main resolves");
        let got = resolve(None, p).expect("resolve ok");
        assert_eq!(got, expected);
    }

    #[test]
    fn common_name_fallback_resolves_master() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        let dir = init_repo();
        let p = dir.path();
        // Push only `master` so strategies 1-4 all miss and we hit the
        // common-name fallback on the second candidate.
        let _bare = add_origin_with_branches(p, &["master"]);

        // Ensure origin/HEAD is NOT set so strategy 3 falls through.
        // `add_origin_with_branches` doesn't set it, which is what we want.
        let expected = rev_parse(p, "origin/master").expect("origin/master resolves");
        let got = resolve(None, p).expect("resolve ok");
        assert_eq!(got, expected);
    }

    #[test]
    fn all_remote_branches_missing_falls_back_to_head_parent() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        let dir = init_repo();
        let p = dir.path();
        // No origin at all — every remote strategy must miss and we
        // should end up at HEAD~1.
        let expected = rev_parse(p, "HEAD~1").expect("HEAD~1 resolves");
        let got = resolve(None, p).expect("resolve ok");
        assert_eq!(got, expected);
    }

    #[test]
    fn not_a_git_repo_returns_error() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        let dir = tempfile::tempdir().unwrap();
        let err = resolve(None, dir.path()).expect_err("should fail");
        assert!(
            matches!(err, BaseShaError::NotAGitRepo { .. }),
            "expected NotAGitRepo, got {err:?}"
        );
    }

    #[test]
    fn total_failure_single_commit_repo() {
        let _g = env_lock();
        unsafe {
            std::env::remove_var("GITHUB_BASE_REF");
        }
        // Repo with exactly one commit: HEAD~1 doesn't exist, no remote,
        // nothing to fall back on. Must return AllStrategiesFailed.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("a.txt"), "only\n").unwrap();
        git(p, &["add", "a.txt"]);
        git(p, &["commit", "-q", "-m", "only"]);

        let err = resolve(None, p).expect_err("should fail");
        match err {
            BaseShaError::AllStrategiesFailed { attempts, .. } => {
                assert!(
                    attempts.iter().any(|a| a.starts_with("6. HEAD~1")),
                    "expected HEAD~1 attempt line, got {attempts:?}"
                );
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }
}
