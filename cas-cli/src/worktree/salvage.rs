//! Salvage patch writer for dirty worktrees.
//!
//! Captures all recoverable state from a worktree as a single `.patch` file
//! under `<repo_root>/.cas/salvage/`. The patch combines tracked diffs (vs
//! `HEAD`) and untracked file contents so that `git apply` in a fresh clone
//! of the same commit reproduces the working-tree state.
//!
//! Used by the daemon reaper and the one-shot `cas worktree salvage` CLI to
//! guarantee no worker WIP is lost when a worktree is removed.
//!
//! # Untracked files
//!
//! Untracked files are captured by temporarily marking them as intent-to-add
//! (`git add -N`) so `git diff HEAD --binary` emits them as additions from
//! `/dev/null`. The index state is restored via `git reset -- <paths>` before
//! this function returns, so the worktree is logically unchanged.
//!
//! # Binary files
//!
//! Binary files are encoded using git's native binary patch format
//! (`git diff --binary`). `git apply --binary` (or any recent `git apply`
//! default) replays them byte-for-byte. No separate handling, no corruption.
//!
//! # Large files
//!
//! Untracked files whose size exceeds [`MAX_UNTRACKED_BYTES`] are **elided**:
//! they are omitted from the patch and recorded in
//! [`SalvageOutcome::skipped`]. The rationale is that patch files are meant
//! to be small enough to email / archive; a 1 GB log file doesn't belong in
//! one. Tracked-file modifications are never elided — if the user committed
//! it once, they presumably want the delta preserved regardless of size.
//!
//! # Atomicity
//!
//! The patch is written to `<name>.patch.tmp` and then renamed into place, so
//! a crash mid-write cannot leave a half-written `.patch` that readers would
//! mistake for a complete salvage.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use thiserror::Error;

/// Maximum size of an individual untracked file that will be included in the
/// salvage patch. Files larger than this are elided with a note.
pub const MAX_UNTRACKED_BYTES: u64 = 10 * 1024 * 1024; // 10 MiB

/// Errors that can occur during salvage.
#[derive(Debug, Error)]
pub enum SalvageError {
    #[error("worktree path does not exist: {0}")]
    WorktreeMissing(PathBuf),

    #[error("path is not a git working tree: {0}")]
    NotAWorktree(PathBuf),

    #[error("git command failed: {0}")]
    GitFailed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Reason an untracked file was omitted from the patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// File exceeded [`MAX_UNTRACKED_BYTES`].
    TooLarge { bytes: u64 },
    /// File disappeared between listing and stat.
    Vanished,
}

/// Result of a successful salvage operation.
#[derive(Debug, Clone)]
pub struct SalvageOutcome {
    /// Absolute path of the written `.patch` file.
    pub patch_path: PathBuf,
    /// Untracked files that were elided (not included in the patch).
    pub skipped: Vec<(PathBuf, SkipReason)>,
}

/// Capture the dirty state of `worktree_path` into
/// `<repo_root>/.cas/salvage/<timestamp>-<worker_name>.patch`.
///
/// Returns `Ok(None)` when the worktree is clean (no tracked modifications
/// and no untracked files) — no file is written.
pub fn salvage(
    worktree_path: &Path,
    repo_root: &Path,
    worker_name: &str,
) -> Result<Option<SalvageOutcome>, SalvageError> {
    if !worktree_path.exists() {
        return Err(SalvageError::WorktreeMissing(worktree_path.to_path_buf()));
    }
    ensure_worktree(worktree_path)?;

    let untracked_all = list_untracked(worktree_path)?;
    let tracked_dirty = has_tracked_changes(worktree_path)?;

    if untracked_all.is_empty() && !tracked_dirty {
        return Ok(None);
    }

    // Partition untracked into includable + skipped.
    let mut includable: Vec<PathBuf> = Vec::new();
    let mut skipped: Vec<(PathBuf, SkipReason)> = Vec::new();
    for rel in untracked_all {
        let abs = worktree_path.join(&rel);
        match fs::metadata(&abs) {
            Ok(meta) if meta.len() > MAX_UNTRACKED_BYTES => {
                skipped.push((rel, SkipReason::TooLarge { bytes: meta.len() }));
            }
            Ok(_) => includable.push(rel),
            Err(_) => skipped.push((rel, SkipReason::Vanished)),
        }
    }

    // Intent-to-add includable untracked files so they appear in `git diff`.
    // We must reset them afterward regardless of diff outcome.
    if !includable.is_empty() {
        intent_to_add(worktree_path, &includable)?;
    }
    let diff_result = run_diff(worktree_path);
    if !includable.is_empty() {
        // Best-effort reset; diff outcome is what we actually care about.
        let _ = reset_paths(worktree_path, &includable);
    }
    let patch_bytes = diff_result?;

    if patch_bytes.is_empty() {
        // Nothing tracked-dirty, all untracked skipped → nothing to write.
        if skipped.is_empty() {
            return Ok(None);
        }
        // All untracked skipped but caller probably still wants to know.
        // Fall through and write an empty-marker patch so audit logs show
        // the salvage happened.
    }

    let salvage_dir = repo_root.join(".cas").join("salvage");
    fs::create_dir_all(&salvage_dir)?;

    let patch_path = unique_patch_path(&salvage_dir, worker_name);
    write_atomic(&patch_path, &patch_bytes, &skipped)?;

    Ok(Some(SalvageOutcome {
        patch_path,
        skipped,
    }))
}

fn ensure_worktree(path: &Path) -> Result<(), SalvageError> {
    let out = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(path)
        .output()?;
    if !out.status.success() {
        return Err(SalvageError::NotAWorktree(path.to_path_buf()));
    }
    if String::from_utf8_lossy(&out.stdout).trim() != "true" {
        return Err(SalvageError::NotAWorktree(path.to_path_buf()));
    }
    Ok(())
}

fn has_tracked_changes(path: &Path) -> Result<bool, SalvageError> {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(path)
        .output()?;
    if !out.status.success() {
        return Err(SalvageError::GitFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if !line.starts_with("??") && !line.is_empty() {
            return Ok(true);
        }
    }
    Ok(false)
}

fn list_untracked(path: &Path) -> Result<Vec<PathBuf>, SalvageError> {
    let out = Command::new("git")
        .args(["ls-files", "--others", "--exclude-standard", "-z"])
        .current_dir(path)
        .output()?;
    if !out.status.success() {
        return Err(SalvageError::GitFailed(
            String::from_utf8_lossy(&out.stderr).to_string(),
        ));
    }
    Ok(out
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|bytes| PathBuf::from(String::from_utf8_lossy(bytes).to_string()))
        .collect())
}

fn intent_to_add(path: &Path, files: &[PathBuf]) -> Result<(), SalvageError> {
    let mut args: Vec<String> = vec!["add".into(), "--intent-to-add".into(), "--".into()];
    for f in files {
        args.push(f.to_string_lossy().to_string());
    }
    let out = Command::new("git")
        .args(&args)
        .current_dir(path)
        .output()?;
    if !out.status.success() {
        return Err(SalvageError::GitFailed(format!(
            "git add --intent-to-add failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

fn reset_paths(path: &Path, files: &[PathBuf]) -> Result<(), SalvageError> {
    let mut args: Vec<String> = vec!["reset".into(), "--".into()];
    for f in files {
        args.push(f.to_string_lossy().to_string());
    }
    let out = Command::new("git")
        .args(&args)
        .current_dir(path)
        .output()?;
    if !out.status.success() {
        return Err(SalvageError::GitFailed(format!(
            "git reset failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(())
}

fn run_diff(path: &Path) -> Result<Vec<u8>, SalvageError> {
    let out = Command::new("git")
        .args(["diff", "HEAD", "--binary", "--no-color"])
        .current_dir(path)
        .output()?;
    // `git diff` exits 0 (no diff) or 1 (diff present) on success; anything
    // else is a real error.
    let code = out.status.code().unwrap_or(-1);
    if code != 0 && code != 1 {
        return Err(SalvageError::GitFailed(format!(
            "git diff HEAD failed (exit {code}): {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(out.stdout)
}

fn unique_patch_path(dir: &Path, worker_name: &str) -> PathBuf {
    let safe_worker = sanitize_worker_name(worker_name);
    let base = format!(
        "{ts}-{worker}",
        ts = Utc::now().format("%Y-%m-%d-%H%M%S"),
        worker = safe_worker,
    );
    let first = dir.join(format!("{base}.patch"));
    if !first.exists() {
        return first;
    }
    // Collision (same-second salvage for the same worker). Disambiguate with
    // PID + monotonic counter. We try a bounded number of suffixes before
    // falling back to a nanosecond-suffixed path which is effectively unique.
    let pid = std::process::id();
    for i in 0..32 {
        let candidate = dir.join(format!("{base}-{pid}-{i}.patch"));
        if !candidate.exists() {
            return candidate;
        }
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    dir.join(format!("{base}-{pid}-{nanos}.patch"))
}

fn sanitize_worker_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".to_string()
    } else {
        cleaned
    }
}

fn write_atomic(
    final_path: &Path,
    patch_bytes: &[u8],
    skipped: &[(PathBuf, SkipReason)],
) -> Result<(), SalvageError> {
    let tmp_path = final_path.with_extension("patch.tmp");
    {
        let mut f = fs::File::create(&tmp_path)?;
        if !skipped.is_empty() {
            // Leading comment block so readers see elisions immediately.
            // `git apply` ignores lines before the first `diff --git`.
            writeln!(f, "# cas salvage: elided untracked files")?;
            for (path, reason) in skipped {
                let reason_str = match reason {
                    SkipReason::TooLarge { bytes } => {
                        format!("too large ({bytes} bytes > {MAX_UNTRACKED_BYTES} limit)")
                    }
                    SkipReason::Vanished => "vanished before snapshot".to_string(),
                };
                writeln!(f, "#   {}  — {}", path.display(), reason_str)?;
            }
            writeln!(f, "#")?;
        }
        f.write_all(patch_bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, final_path)?;
    Ok(())
}

#[cfg(test)]
mod tests;
