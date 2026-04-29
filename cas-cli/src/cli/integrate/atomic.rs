//! Atomic file write + multi-file commit helpers.
//!
//! `init` writes 3 files (claude SKILL, references, cursor SKILL). A process
//! kill or disk-full mid-sequence leaves a partial state that the
//! `AlreadyConfigured` guard refuses to retry. This module provides:
//!
//! - [`atomic_write`] — write a single file via `<path>.tmp.<pid>.<nonce>`
//!   + `rename(2)`. POSIX rename is atomic on the same filesystem.
//! - [`atomic_write_all`] — stage many files into temp paths, fsync them,
//!   then rename in order. Best-effort multi-file atomicity: if any rename
//!   succeeds and a later rename fails, the orchestration layer should
//!   `cas integrate <platform> refresh` to converge state.
//!
//! **TODO (cas-fc38 cross-cutting):** quick-hound-41 is hoisting these into
//! a shared `cli/integrate/fs.rs` alongside `read_capped`, `escape_md_cell`,
//! and friends. When that lands, this module can be replaced with
//! re-exports. Until then, kept local to cas-7417.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Atomically write `content` to `path`. The file is first written to a
/// `<path>.tmp.<nonce>` sibling, fsynced, and then renamed into place.
/// `rename(2)` on the same filesystem is atomic; the user can never observe
/// a half-written file under the final path.
///
/// Creates parent directories as needed (the parent-create itself is not
/// atomic, but is idempotent).
pub fn atomic_write(path: &Path, content: &str) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let tmp = staging_path(path);
    write_then_sync(&tmp, content)?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

/// Stage `(path, content)` pairs into temp files, then rename each in
/// sequence. On a panic / kill, no partially-renamed file is observed at the
/// final path (each rename is atomic), but a kill BETWEEN renames can leave
/// an inconsistent set of finals visible.
///
/// This is "atomic per file" — strictly stronger than three sequential
/// `std::fs::write`. Full all-or-nothing semantics across files would require
/// a directory-level snapshot which POSIX does not offer; the orchestration
/// layer compensates by running `refresh` to converge.
pub fn atomic_write_all(items: &[(PathBuf, String)]) -> anyhow::Result<Vec<PathBuf>> {
    let mut staged: Vec<(PathBuf, PathBuf)> = Vec::with_capacity(items.len());
    let mut written: Vec<PathBuf> = Vec::with_capacity(items.len());

    // Stage all temp files first; if any staging step fails, clean up.
    for (final_path, content) in items {
        if let Some(parent) = final_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let tmp = staging_path(final_path);
        if let Err(e) = write_then_sync(&tmp, content) {
            // Clean up any temps we already staged, then bail.
            for (_, t) in &staged {
                let _ = std::fs::remove_file(t);
            }
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        staged.push((final_path.clone(), tmp));
    }

    // Promote each temp to its final path. On rename failure, drain
    // remaining staged temps so we never leave `<path>.cas-tmp.*` siblings
    // beside the final files. (Already-renamed final files survive — that's
    // the documented per-file-atomic guarantee. Orchestration layer
    // converges via `refresh`.)
    let mut promote_iter = staged.into_iter();
    while let Some((final_path, tmp)) = promote_iter.next() {
        match std::fs::rename(&tmp, &final_path) {
            Ok(()) => written.push(final_path),
            Err(e) => {
                // Clean up the failing temp + everything still staged.
                let _ = std::fs::remove_file(&tmp);
                for (_, remaining) in promote_iter {
                    let _ = std::fs::remove_file(remaining);
                }
                return Err(anyhow::Error::from(e))
                    .with_context(|| format!("rename {} -> {}", tmp.display(), final_path.display()));
            }
        }
    }
    Ok(written)
}

fn write_then_sync(tmp: &Path, content: &str) -> anyhow::Result<()> {
    let mut f: File = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(tmp)
        .with_context(|| format!("opening {}", tmp.display()))?;
    f.write_all(content.as_bytes())
        .with_context(|| format!("writing {}", tmp.display()))?;
    f.sync_all()
        .with_context(|| format!("fsync {}", tmp.display()))?;
    Ok(())
}

/// Process-local monotonic counter for staging-path uniqueness. Replaces
/// the earlier `subsec_nanos` nonce, which collided when two
/// `staging_path` calls hit the same nanosecond bucket (notably in the
/// tight loop inside [`atomic_write_all`]).
static STAGING_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn staging_path(final_path: &Path) -> PathBuf {
    let pid = std::process::id();
    let nonce = STAGING_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut s = final_path.as_os_str().to_owned();
    s.push(format!(".cas-tmp.{pid}.{nonce}"));
    PathBuf::from(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn atomic_write_creates_parents_and_replaces_existing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("a/b/c/file.md");
        atomic_write(&path, "hello").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
        atomic_write(&path, "replaced").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replaced");
    }

    #[test]
    fn atomic_write_leaves_no_tmp_files_on_success() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.md");
        atomic_write(&path, "ok").unwrap();
        for entry in std::fs::read_dir(tmp.path()).unwrap() {
            let p = entry.unwrap().path();
            assert!(
                !p.to_string_lossy().contains(".cas-tmp."),
                "stray tmp left: {}",
                p.display()
            );
        }
    }

    #[test]
    fn atomic_write_all_writes_all_or_cleans_up_temps() {
        let tmp = TempDir::new().unwrap();
        let items = vec![
            (tmp.path().join("one.md"), "1".to_string()),
            (tmp.path().join("nested/two.md"), "2".to_string()),
        ];
        let written = atomic_write_all(&items).unwrap();
        assert_eq!(written.len(), 2);
        assert_eq!(std::fs::read_to_string(&items[0].0).unwrap(), "1");
        assert_eq!(std::fs::read_to_string(&items[1].0).unwrap(), "2");

        // No tmp leftovers at top level.
        for entry in std::fs::read_dir(tmp.path()).unwrap() {
            let p = entry.unwrap().path();
            assert!(!p.to_string_lossy().contains(".cas-tmp."));
        }
    }

    #[test]
    fn staging_path_is_unique_per_call() {
        let p = Path::new("/tmp/foo.md");
        let a = staging_path(p);
        let b = staging_path(p);
        let c = staging_path(p);
        // Monotonic counter must produce three distinct paths.
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_ne!(a, c);
        // All paths share the documented suffix shape.
        for path in [&a, &b, &c] {
            assert!(path.to_string_lossy().contains(".cas-tmp."));
            assert!(path.to_string_lossy().starts_with("/tmp/foo.md.cas-tmp."));
        }
    }

    #[test]
    fn atomic_write_all_cleans_temps_when_rename_fails() {
        // Force a rename failure by making the second target path a
        // pre-existing directory: `rename <file> <existing-dir>` errors on
        // most platforms (ENOTEMPTY / ENOTDIR / EISDIR depending on libc).
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir_all(tmp.path().join("blocker")).unwrap();
        // Put a file inside the blocker dir so rename-onto-dir cannot win.
        std::fs::write(tmp.path().join("blocker/anchor"), "x").unwrap();

        let items = vec![
            (tmp.path().join("first.md"), "1".to_string()),
            (tmp.path().join("blocker"), "2".to_string()), // forces rename failure
            (tmp.path().join("never_reached.md"), "3".to_string()),
        ];
        let result = atomic_write_all(&items);
        assert!(result.is_err(), "rename onto non-empty dir must fail");

        // First file IS visible at its final path (per-file atomic).
        assert_eq!(std::fs::read_to_string(&items[0].0).unwrap(), "1");
        // Third item never made it past staging; no final.
        assert!(!items[2].0.exists());

        // Critically: no stray `.cas-tmp.*` files remain anywhere under tmp.
        let mut leftover = Vec::new();
        for entry in walkdir::WalkDir::new(tmp.path()) {
            let entry = entry.unwrap();
            if entry
                .path()
                .to_string_lossy()
                .contains(".cas-tmp.")
            {
                leftover.push(entry.path().to_path_buf());
            }
        }
        assert!(
            leftover.is_empty(),
            "rename-failure path leaked staged temps: {leftover:?}"
        );
    }
}
