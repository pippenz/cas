//! Integration tests for the sweep module.
//!
//! Each test builds a temp git repo with `.cas/worktrees/<name>` children
//! that are real `git worktree add` products (or plain dirs for negative
//! cases), then exercises `sweep_one_repo` and asserts on the Disposition.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::worktree::sweep::{
    sweep_one_repo, Disposition, SweepOptions,
};

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} in {}: {e}", dir.display()))
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn bootstrap_repo(temp: &TempDir) -> PathBuf {
    let repo = temp.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    git_ok(&repo, &["init", "-q", "-b", "main"]);
    git_ok(&repo, &["config", "user.email", "t@t.com"]);
    git_ok(&repo, &["config", "user.name", "Test"]);
    git_ok(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.txt"), "alpha\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "initial"]);
    repo
}

/// Create a factory-style worktree under `<repo>/.cas/worktrees/<name>` on a
/// new branch forked from main. Returns its path.
fn add_worktree(repo: &Path, name: &str) -> PathBuf {
    let path = repo.join(".cas").join("worktrees").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    git_ok(
        repo,
        &[
            "worktree",
            "add",
            "-b",
            &format!("factory/{name}"),
            path.to_str().unwrap(),
            "main",
        ],
    );
    // Make sure we have an identity inside the worktree for test commits.
    git_ok(&path, &["config", "user.email", "t@t.com"]);
    git_ok(&path, &["config", "user.name", "Test"]);
    git_ok(&path, &["config", "commit.gpgsign", "false"]);
    path
}

#[test]
fn sweep_clean_merged_worktree_removes_it() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let wt = add_worktree(&repo, "worker1");
    // No commits on the worktree branch — it's trivially merged with main.

    let report = sweep_one_repo(&repo, SweepOptions::default());

    assert_eq!(report.worktrees.len(), 1);
    assert!(matches!(report.worktrees[0].disposition, Disposition::Removed));
    assert!(!wt.exists(), "worktree dir must be gone");
    assert!(report.prune_ran, "prune must run when worktrees existed");
}

#[test]
fn sweep_dirty_worktree_skips_without_flag() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let wt = add_worktree(&repo, "worker2");
    fs::write(wt.join("dirty.txt"), "wip\n").unwrap();

    let report = sweep_one_repo(&repo, SweepOptions::default());

    assert_eq!(report.worktrees.len(), 1);
    assert!(
        matches!(
            report.worktrees[0].disposition,
            Disposition::SkippedDirty { modified_files: 1 }
        ),
        "got {:?}",
        report.worktrees[0].disposition,
    );
    assert!(wt.exists(), "dirty worktree must NOT be removed");
}

#[test]
fn sweep_dirty_worktree_salvages_and_removes_with_flag() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let wt = add_worktree(&repo, "worker3");
    fs::write(wt.join("dirty.txt"), "wip\n").unwrap();

    let opts = SweepOptions {
        dry_run: false,
        salvage_dirty: true,
    };
    let report = sweep_one_repo(&repo, opts);

    assert_eq!(report.worktrees.len(), 1);
    match &report.worktrees[0].disposition {
        Disposition::SalvagedAndRemoved { patch_path } => {
            assert!(patch_path.exists(), "salvage patch must be on disk");
            assert!(patch_path.starts_with(repo.join(".cas/salvage")));
        }
        other => panic!("expected SalvagedAndRemoved, got {other:?}"),
    }
    assert!(!wt.exists(), "worktree dir must be removed");
}

#[test]
fn sweep_unmerged_worktree_is_skipped() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let wt = add_worktree(&repo, "worker4");
    // Commit something on the worktree branch so it has unmerged commits vs main.
    fs::write(wt.join("feat.txt"), "feature\n").unwrap();
    git_ok(&wt, &["add", "."]);
    git_ok(&wt, &["commit", "-q", "-m", "feature commit"]);

    let report = sweep_one_repo(&repo, SweepOptions::default());
    match &report.worktrees[0].disposition {
        Disposition::SkippedUnmerged { unmerged_commits } => {
            assert_eq!(*unmerged_commits, 1);
        }
        other => panic!("expected SkippedUnmerged, got {other:?}"),
    }
    assert!(wt.exists(), "unmerged worktree MUST NOT be removed");
}

#[test]
fn sweep_dirty_and_unmerged_is_skipped_even_with_salvage_flag() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let wt = add_worktree(&repo, "worker5");
    // Commit + dirty working tree.
    fs::write(wt.join("feat.txt"), "feature\n").unwrap();
    git_ok(&wt, &["add", "."]);
    git_ok(&wt, &["commit", "-q", "-m", "feature commit"]);
    fs::write(wt.join("dirty.txt"), "extra wip\n").unwrap();

    let opts = SweepOptions {
        dry_run: false,
        salvage_dirty: true, // must not override unmerged guard
    };
    let report = sweep_one_repo(&repo, opts);
    match &report.worktrees[0].disposition {
        Disposition::SkippedDirtyUnmerged {
            modified_files,
            unmerged_commits,
        } => {
            assert_eq!(*modified_files, 1);
            assert_eq!(*unmerged_commits, 1);
        }
        other => panic!("expected SkippedDirtyUnmerged, got {other:?}"),
    }
    assert!(wt.exists(), "unmerged+dirty must not be removed");
}

#[test]
fn dry_run_makes_no_filesystem_changes() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let clean = add_worktree(&repo, "clean");
    let dirty = add_worktree(&repo, "dirty");
    fs::write(dirty.join("x.txt"), "wip\n").unwrap();

    let opts = SweepOptions {
        dry_run: true,
        salvage_dirty: true,
    };
    let report = sweep_one_repo(&repo, opts);

    assert!(clean.exists(), "dry run must not remove clean worktree");
    assert!(dirty.exists(), "dry run must not remove dirty worktree");
    assert!(
        !repo.join(".cas/salvage").exists() || repo.join(".cas/salvage").read_dir().unwrap().next().is_none(),
        "dry run must not write a salvage patch"
    );
    assert!(!report.prune_ran, "dry run must not invoke git prune");
    let kinds: Vec<_> = report
        .worktrees
        .iter()
        .map(|w| std::mem::discriminant(&w.disposition))
        .collect();
    assert_eq!(kinds.len(), 2);
}

#[test]
fn repo_with_no_worktrees_dir_returns_empty_report() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    // Deliberately no .cas/worktrees/.
    let report = sweep_one_repo(&repo, SweepOptions::default());
    assert!(report.worktrees.is_empty());
    assert!(report.repo_error.is_none());
    assert!(!report.prune_ran, "no worktrees → no prune");
}

#[test]
fn sweep_multiple_worktrees_per_repo() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    let _a = add_worktree(&repo, "a");
    let b = add_worktree(&repo, "b");
    let _c = add_worktree(&repo, "c");
    fs::write(b.join("wip.txt"), "wip\n").unwrap(); // dirty

    let report = sweep_one_repo(&repo, SweepOptions::default());
    assert_eq!(report.worktrees.len(), 3);
    assert_eq!(report.removed_count(), 2, "a + c removed");
    assert_eq!(report.skipped_count(), 1, "b skipped dirty");
    assert!(report.prune_ran);
}

#[test]
fn non_worktree_subdir_is_ignored() {
    let temp = TempDir::new().unwrap();
    let repo = bootstrap_repo(&temp);
    // Plain dir without a .git marker — must be skipped (not counted, not an error).
    let plain = repo.join(".cas/worktrees/just-a-dir");
    fs::create_dir_all(&plain).unwrap();
    fs::write(plain.join("README"), "hi").unwrap();

    let report = sweep_one_repo(&repo, SweepOptions::default());
    assert_eq!(report.worktrees.len(), 0);
    assert!(plain.exists());
}
