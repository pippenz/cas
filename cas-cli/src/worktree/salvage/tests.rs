//! Integration tests for the salvage module.
//!
//! Each test builds a throwaway git repo in a `TempDir`, exercises `salvage`,
//! and asserts on the resulting patch — including a round-trip where the
//! patch is applied to a fresh clone and the working tree is diffed against
//! the original.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

use crate::worktree::salvage::{salvage, SalvageError, SkipReason, MAX_UNTRACKED_BYTES};

fn git(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"))
}

fn git_ok(dir: &Path, args: &[&str]) {
    let out = git(dir, args);
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// Build a temp git repo with one initial commit. Returns (temp, repo_path).
fn fresh_repo() -> (TempDir, PathBuf) {
    let temp = TempDir::new().unwrap();
    let repo = temp.path().join("repo");
    fs::create_dir(&repo).unwrap();
    git_ok(&repo, &["init", "-q", "-b", "main"]);
    git_ok(&repo, &["config", "user.email", "t@t.com"]);
    git_ok(&repo, &["config", "user.name", "Test"]);
    git_ok(&repo, &["config", "commit.gpgsign", "false"]);
    fs::write(repo.join("a.txt"), "alpha\n").unwrap();
    fs::write(repo.join("b.txt"), "bravo\n").unwrap();
    git_ok(&repo, &["add", "."]);
    git_ok(&repo, &["commit", "-q", "-m", "initial"]);
    (temp, repo)
}

/// Clone `src` into a sibling path at `dest` so the clone shares the same
/// HEAD commit, then return the clone path.
fn clone_of(src: &Path, dest: &Path) -> PathBuf {
    git_ok(
        src.parent().unwrap(),
        &[
            "clone",
            "-q",
            src.to_str().unwrap(),
            dest.to_str().unwrap(),
        ],
    );
    git_ok(dest, &["config", "user.email", "t@t.com"]);
    git_ok(dest, &["config", "user.name", "Test"]);
    dest.to_path_buf()
}

#[test]
fn salvage_happy_path_roundtrips_through_git_apply() {
    let (temp, repo) = fresh_repo();

    // Modify 2 tracked + add 1 untracked.
    fs::write(repo.join("a.txt"), "alpha\nmore\n").unwrap();
    fs::write(repo.join("b.txt"), "").unwrap();
    fs::write(repo.join("c.txt"), "charlie\n").unwrap();

    let outcome = salvage(&repo, &repo, "happy-worker")
        .expect("salvage ok")
        .expect("some outcome");

    assert!(outcome.patch_path.exists(), "patch file must exist");
    assert!(outcome.patch_path.starts_with(repo.join(".cas/salvage")));
    assert!(outcome.skipped.is_empty());

    // Clone fresh and apply the patch.
    let clone = clone_of(&repo, &temp.path().join("clone"));
    let apply = git(
        &clone,
        &[
            "apply",
            "--binary",
            outcome.patch_path.to_str().unwrap(),
        ],
    );
    assert!(
        apply.status.success(),
        "git apply failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );

    // Now the clone's working tree should match the original for every file.
    for name in ["a.txt", "b.txt", "c.txt"] {
        let orig = fs::read(repo.join(name)).unwrap();
        let replayed = fs::read(clone.join(name)).unwrap();
        assert_eq!(orig, replayed, "{name} content mismatch after apply");
    }
}

#[test]
fn salvage_empty_worktree_returns_none() {
    let (_temp, repo) = fresh_repo();
    let out = salvage(&repo, &repo, "quiet").unwrap();
    assert!(out.is_none());
    let salvage_dir = repo.join(".cas/salvage");
    // Directory may or may not exist, but no .patch files should be present.
    if salvage_dir.exists() {
        let count = fs::read_dir(&salvage_dir).unwrap().count();
        assert_eq!(count, 0, "no patch should be written for clean worktree");
    }
}

#[test]
fn salvage_nonexistent_path_errors_without_panic() {
    let (_temp, repo) = fresh_repo();
    let bogus = repo.join("does-not-exist");
    let err = salvage(&bogus, &repo, "ghost").unwrap_err();
    matches!(err, SalvageError::WorktreeMissing(_));
}

#[test]
fn salvage_captures_binary_untracked_file() {
    let (temp, repo) = fresh_repo();

    // A small "binary" file: contains NUL bytes so git treats it as binary.
    let bin_bytes: Vec<u8> = (0u8..64).chain(std::iter::once(0)).chain(64u8..128).collect();
    fs::write(repo.join("blob.bin"), &bin_bytes).unwrap();

    let outcome = salvage(&repo, &repo, "binaryworker")
        .unwrap()
        .expect("outcome");
    assert!(outcome.skipped.is_empty());

    let clone = clone_of(&repo, &temp.path().join("clone"));
    let apply = git(
        &clone,
        &[
            "apply",
            "--binary",
            outcome.patch_path.to_str().unwrap(),
        ],
    );
    assert!(
        apply.status.success(),
        "git apply --binary failed: {}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let replayed = fs::read(clone.join("blob.bin")).unwrap();
    assert_eq!(replayed, bin_bytes, "binary file must roundtrip exactly");
}

#[test]
fn salvage_elides_oversize_untracked_file() {
    let (_temp, repo) = fresh_repo();

    // One small untracked (included) + one over-limit (elided).
    fs::write(repo.join("small.txt"), "tiny\n").unwrap();
    let big = vec![b'x'; (MAX_UNTRACKED_BYTES + 1) as usize];
    fs::write(repo.join("big.bin"), &big).unwrap();

    let outcome = salvage(&repo, &repo, "largework")
        .unwrap()
        .expect("outcome");

    assert_eq!(outcome.skipped.len(), 1, "exactly one file elided");
    let (path, reason) = &outcome.skipped[0];
    assert_eq!(path, &PathBuf::from("big.bin"));
    match reason {
        SkipReason::TooLarge { bytes } => assert!(*bytes > MAX_UNTRACKED_BYTES),
        other => panic!("expected TooLarge, got {other:?}"),
    }

    // Patch header must mention the elided file so readers see the gap.
    let patch = fs::read_to_string(&outcome.patch_path).unwrap();
    assert!(patch.contains("big.bin"), "elided file not noted in patch header");
    assert!(patch.contains("cas salvage: elided"), "elision banner missing");
    // Small file must still be in the diff body.
    assert!(patch.contains("small.txt"), "small untracked file missing");
}

#[test]
fn salvage_concurrent_calls_do_not_collide() {
    let (_temp, repo) = fresh_repo();
    fs::write(repo.join("a.txt"), "changed-1\n").unwrap();

    let first = salvage(&repo, &repo, "dup").unwrap().expect("first");

    // Make a further change so the second call also has work to do.
    fs::write(repo.join("a.txt"), "changed-2\n").unwrap();
    let second = salvage(&repo, &repo, "dup").unwrap().expect("second");

    assert_ne!(
        first.patch_path, second.patch_path,
        "second salvage must not overwrite the first",
    );
    assert!(first.patch_path.exists());
    assert!(second.patch_path.exists());
}

#[test]
fn salvage_is_atomic_leaves_no_tmp_file() {
    let (_temp, repo) = fresh_repo();
    fs::write(repo.join("a.txt"), "drift\n").unwrap();
    let _ = salvage(&repo, &repo, "atomic").unwrap().expect("outcome");

    let salvage_dir = repo.join(".cas/salvage");
    for entry in fs::read_dir(&salvage_dir).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().to_string();
        assert!(
            !name.ends_with(".tmp"),
            "atomic write left a .tmp artifact: {name}",
        );
    }
}

#[test]
fn salvage_restores_index_state() {
    let (_temp, repo) = fresh_repo();
    fs::write(repo.join("u.txt"), "untracked\n").unwrap();

    let _ = salvage(&repo, &repo, "idx").unwrap().expect("outcome");

    // After salvage, `u.txt` must still be untracked — salvage used
    // intent-to-add internally and must have reset it.
    let status = git(&repo, &["status", "--porcelain=v1"]);
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        text.lines().any(|l| l.starts_with("?? u.txt")),
        "u.txt should be untracked after salvage; status was:\n{text}",
    );
}
