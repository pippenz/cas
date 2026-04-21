//! Cross-repo worktree sweep.
//!
//! Given a single repo root, enumerate every factory/agent worktree directory
//! underneath (`<repo>/.cas/worktrees/*`), classify each by merge-status and
//! dirty-status, and remove the ones that are safe. Optionally capture dirty
//! worktree state via [`crate::worktree::salvage::salvage`] first (Unit 2)
//! so no WIP is lost.
//!
//! The module exposes a `sweep_one_repo` entry point so the CLI (`cas
//! worktree sweep` / `cas sweep-all`) and Unit 3's opportunistic trigger can
//! share exactly the same per-repo logic. A cross-repo driver lives on top:
//! `sweep_all_known` loops over [`crate::worktree::discovery`] and returns a
//! [`SweepAllReport`].

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use tracing::{debug, warn};

use crate::worktree::salvage::{self, SalvageOutcome};

/// Options controlling what the sweep is allowed to do.
#[derive(Debug, Clone, Copy, Default)]
pub struct SweepOptions {
    /// Report only — do not remove any worktree, do not write any patch.
    pub dry_run: bool,
    /// If a worktree is dirty, first write a salvage patch under
    /// `<repo>/.cas/salvage/` and then remove it. Clean+merged worktrees
    /// are removed regardless of this flag; unmerged worktrees are never
    /// removed regardless of this flag.
    pub salvage_dirty: bool,
}

/// What the sweep decided to do with a single worktree.
#[derive(Debug, Clone)]
pub enum Disposition {
    /// Clean + merged → removed.
    Removed,
    /// Dirty; `--salvage-dirty` was set → salvage patch written then removed.
    SalvagedAndRemoved { patch_path: PathBuf },
    /// Dirty; `--salvage-dirty` was NOT set → skipped on disk.
    SkippedDirty { modified_files: usize },
    /// Branch has commits not merged to the parent branch → skipped.
    SkippedUnmerged { unmerged_commits: usize },
    /// Dirty AND unmerged → skipped (reports both).
    SkippedDirtyUnmerged {
        modified_files: usize,
        unmerged_commits: usize,
    },
    /// Dry-run preview of what would happen without the flag.
    WouldRemove,
    WouldSalvageAndRemove,
    /// Worktree path vanished or is otherwise unreadable (reaped by another
    /// process mid-scan, permission error, etc.).
    Error { reason: String },
}

impl Disposition {
    pub fn is_skip(&self) -> bool {
        matches!(
            self,
            Disposition::SkippedDirty { .. }
                | Disposition::SkippedUnmerged { .. }
                | Disposition::SkippedDirtyUnmerged { .. }
        )
    }

    pub fn is_removed(&self) -> bool {
        matches!(
            self,
            Disposition::Removed | Disposition::SalvagedAndRemoved { .. }
        )
    }
}

/// Per-worktree sweep record.
#[derive(Debug, Clone)]
pub struct WorktreeSweepRecord {
    pub worktree_path: PathBuf,
    pub disposition: Disposition,
    /// Size reclaimed from disk, in bytes. Populated only for
    /// `Removed` / `SalvagedAndRemoved` in non-dry-run mode.
    pub bytes_reclaimed: u64,
}

/// Aggregate report for one repo.
#[derive(Debug, Clone, Default)]
pub struct RepoSweepReport {
    pub repo_root: PathBuf,
    pub worktrees: Vec<WorktreeSweepRecord>,
    pub prune_ran: bool,
    /// Fatal errors at repo level (e.g. `.cas/worktrees` unreadable).
    pub repo_error: Option<String>,
}

impl RepoSweepReport {
    pub fn removed_count(&self) -> usize {
        self.worktrees.iter().filter(|w| w.disposition.is_removed()).count()
    }
    pub fn skipped_count(&self) -> usize {
        self.worktrees.iter().filter(|w| w.disposition.is_skip()).count()
    }
    pub fn bytes_reclaimed(&self) -> u64 {
        self.worktrees.iter().map(|w| w.bytes_reclaimed).sum()
    }
}

/// Aggregate report across every repo processed in a cross-repo sweep.
#[derive(Debug, Clone, Default)]
pub struct SweepAllReport {
    pub repos: Vec<RepoSweepReport>,
}

impl SweepAllReport {
    pub fn total_removed(&self) -> usize {
        self.repos.iter().map(|r| r.removed_count()).sum()
    }
    pub fn total_skipped(&self) -> usize {
        self.repos.iter().map(|r| r.skipped_count()).sum()
    }
    pub fn total_bytes_reclaimed(&self) -> u64 {
        self.repos.iter().map(|r| r.bytes_reclaimed()).sum()
    }
}

/// Sweep every factory-style worktree under a single repo root.
///
/// `repo_root` must be the repo's working-copy root (the directory holding
/// `.cas/` and `.git/`), not the `.cas/` dir itself.
pub fn sweep_one_repo(repo_root: &Path, opts: SweepOptions) -> RepoSweepReport {
    let mut report = RepoSweepReport {
        repo_root: repo_root.to_path_buf(),
        ..Default::default()
    };

    let wt_root = repo_root.join(".cas").join("worktrees");
    if !wt_root.exists() {
        // Nothing to do — not an error, just a repo with no factory worktrees.
        return report;
    }

    let entries = match std::fs::read_dir(&wt_root) {
        Ok(e) => e,
        Err(e) => {
            report.repo_error = Some(format!(
                "cannot read {}: {e}",
                wt_root.display()
            ));
            return report;
        }
    };

    let mut any_worktrees = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip non-worktree directories (no .git marker).
        let git_marker = path.join(".git");
        if !git_marker.exists() {
            continue;
        }
        any_worktrees = true;

        let rec = sweep_one_worktree(repo_root, &path, opts);
        report.worktrees.push(rec);
    }

    // Always `git worktree prune` at the end — reaps /tmp/cas-*-wt style
    // prunables and stale entries even when we removed nothing ourselves.
    if any_worktrees && !opts.dry_run {
        match Command::new("git")
            .args(["worktree", "prune"])
            .current_dir(repo_root)
            .output()
        {
            Ok(out) if out.status.success() => report.prune_ran = true,
            Ok(out) => warn!(
                repo = %repo_root.display(),
                stderr = %String::from_utf8_lossy(&out.stderr),
                "git worktree prune failed",
            ),
            Err(e) => warn!(
                repo = %repo_root.display(),
                error = %e,
                "git worktree prune could not be spawned",
            ),
        }
    }

    report
}

/// Sweep every known repo in the host registry, in `last_touched` order.
/// Repos with a missing `.cas/` dir (moved or deleted) are recorded with
/// `repo_error = Some(...)` and do not abort the loop.
pub fn sweep_all_known(opts: SweepOptions) -> Result<SweepAllReport> {
    let mut report = SweepAllReport::default();
    for repo in crate::worktree::discovery::list_tracked_repos()? {
        if !repo.healthy {
            let mut r = RepoSweepReport {
                repo_root: repo.path.clone(),
                ..Default::default()
            };
            r.repo_error = Some(format!(
                ".cas/ missing at {} — repo moved or deleted; skipping",
                repo.path.display()
            ));
            report.repos.push(r);
            continue;
        }
        report.repos.push(sweep_one_repo(&repo.path, opts));
    }
    Ok(report)
}

// ---------- internals ----------

fn sweep_one_worktree(
    repo_root: &Path,
    worktree_path: &Path,
    opts: SweepOptions,
) -> WorktreeSweepRecord {
    let mut rec = WorktreeSweepRecord {
        worktree_path: worktree_path.to_path_buf(),
        disposition: Disposition::Removed, // overwritten below
        bytes_reclaimed: 0,
    };

    // Classify state.
    let modified_files = match uncommitted_count(worktree_path) {
        Ok(n) => n,
        Err(e) => {
            rec.disposition = Disposition::Error {
                reason: format!("git status failed: {e}"),
            };
            return rec;
        }
    };
    let parent = resolve_parent_branch(repo_root);
    let unmerged = unmerged_count(worktree_path, &parent).unwrap_or(0);

    let dirty = modified_files > 0;
    let unmerged_commits = unmerged > 0;

    match (dirty, unmerged_commits) {
        (true, true) => {
            rec.disposition = Disposition::SkippedDirtyUnmerged {
                modified_files,
                unmerged_commits: unmerged,
            };
            return rec;
        }
        (false, true) => {
            rec.disposition = Disposition::SkippedUnmerged {
                unmerged_commits: unmerged,
            };
            return rec;
        }
        (true, false) => {
            if !opts.salvage_dirty {
                rec.disposition = Disposition::SkippedDirty { modified_files };
                return rec;
            }
            // Dirty + salvage-dirty → salvage then remove.
            if opts.dry_run {
                rec.disposition = Disposition::WouldSalvageAndRemove;
                return rec;
            }
            let worker = worktree_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            match salvage::salvage(worktree_path, repo_root, worker) {
                Ok(Some(SalvageOutcome { patch_path, .. })) => {
                    rec.bytes_reclaimed = dir_size(worktree_path);
                    if let Err(e) = remove_worktree_forced(repo_root, worktree_path) {
                        rec.disposition = Disposition::Error {
                            reason: format!("salvage ok but remove failed: {e}"),
                        };
                        return rec;
                    }
                    rec.disposition = Disposition::SalvagedAndRemoved { patch_path };
                }
                Ok(None) => {
                    // Race: dirty at status time, clean at salvage time.
                    // Proceed with plain remove.
                    rec.bytes_reclaimed = dir_size(worktree_path);
                    if let Err(e) = remove_worktree_forced(repo_root, worktree_path) {
                        rec.disposition = Disposition::Error {
                            reason: format!("remove failed: {e}"),
                        };
                        return rec;
                    }
                    rec.disposition = Disposition::Removed;
                }
                Err(e) => {
                    rec.disposition = Disposition::Error {
                        reason: format!("salvage failed: {e}"),
                    };
                }
            }
        }
        (false, false) => {
            if opts.dry_run {
                rec.disposition = Disposition::WouldRemove;
                return rec;
            }
            rec.bytes_reclaimed = dir_size(worktree_path);
            if let Err(e) = remove_worktree_forced(repo_root, worktree_path) {
                rec.disposition = Disposition::Error {
                    reason: format!("remove failed: {e}"),
                };
                return rec;
            }
            rec.disposition = Disposition::Removed;
        }
    }
    rec
}

fn uncommitted_count(worktree_path: &Path) -> std::io::Result<usize> {
    let out = Command::new("git")
        .args(["status", "--porcelain=v1"])
        .current_dir(worktree_path)
        .output()?;
    if !out.status.success() {
        return Err(std::io::Error::other(String::from_utf8_lossy(&out.stderr).to_string()));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| !l.is_empty())
        .count())
}

fn unmerged_count(worktree_path: &Path, parent: &str) -> std::io::Result<usize> {
    let out = Command::new("git")
        .args(["rev-list", "--count", &format!("{parent}..HEAD")])
        .current_dir(worktree_path)
        .output()?;
    if !out.status.success() {
        // Unknown parent, detached HEAD, etc. — treat as 0 (not a failure).
        debug!(
            parent = parent,
            stderr = %String::from_utf8_lossy(&out.stderr),
            "unmerged_count: git rev-list exited non-zero; assuming 0"
        );
        return Ok(0);
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .unwrap_or(0))
}

/// Resolve the "parent" branch to compare against. Order of preference:
/// 1. `origin/HEAD` symref target.
/// 2. `HEAD` symref target on the repo.
/// 3. First of `main`, `master`, `develop`, `trunk` that exists.
/// 4. `"main"` as a last resort.
fn resolve_parent_branch(repo_root: &Path) -> String {
    if let Ok(out) = Command::new("git")
        .args(["symbolic-ref", "refs/remotes/origin/HEAD"])
        .current_dir(repo_root)
        .output()
    {
        if out.status.success() {
            let r = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if let Some(b) = r.strip_prefix("refs/remotes/origin/") {
                if !b.is_empty() {
                    return b.to_string();
                }
            }
        }
    }
    for candidate in ["main", "master", "develop", "trunk"] {
        let out = Command::new("git")
            .args(["rev-parse", "--verify", candidate])
            .current_dir(repo_root)
            .output();
        if matches!(out, Ok(ref o) if o.status.success()) {
            return candidate.to_string();
        }
    }
    "main".to_string()
}

fn remove_worktree_forced(repo_root: &Path, worktree_path: &Path) -> std::io::Result<()> {
    let out = Command::new("git")
        .args([
            "worktree",
            "remove",
            "--force",
            worktree_path
                .to_str()
                .ok_or_else(|| std::io::Error::other("non-utf8 path"))?,
        ])
        .current_dir(repo_root)
        .output()?;
    if out.status.success() {
        return Ok(());
    }
    // Fall back to plain directory removal if git refuses (e.g. stale
    // registration, worktree not tracked by git).
    let _ = std::fs::remove_dir_all(worktree_path);
    Ok(())
}

fn dir_size(path: &Path) -> u64 {
    fn walk(p: &Path) -> u64 {
        let Ok(md) = std::fs::symlink_metadata(p) else {
            return 0;
        };
        if md.is_symlink() {
            return md.len();
        }
        if md.is_dir() {
            let mut total: u64 = 0;
            if let Ok(rd) = std::fs::read_dir(p) {
                for e in rd.flatten() {
                    total = total.saturating_add(walk(&e.path()));
                }
            }
            total
        } else {
            md.len()
        }
    }
    walk(path)
}

#[cfg(test)]
mod tests;
