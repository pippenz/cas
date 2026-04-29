//! Process-level lockfile for `cas integrate` invocations.
//!
//! Two parallel `cas integrate <platform> <init|refresh>` runs (a CI hook
//! racing an IDE save, two factory workers, etc.) would otherwise both read
//! the existing SKILL.md, both compute a merge, and both write — second
//! writer wins, first writer's edits silently lost.
//!
//! [`IntegrateLock`] takes an exclusive `fs2`-style advisory lock on
//! `<repo>/.cas/integrate.lock` (or `.cas/integrate.lock` if `.cas/` doesn't
//! exist yet). The lock is held for the lifetime of the [`IntegrateLock`]
//! handle and released on drop. Use [`IntegrateLock::acquire`] to block
//! until available, or [`IntegrateLock::try_acquire`] for a fail-fast
//! variant.
//!
//! Owner: task **cas-7417**. Convention: handlers themselves do NOT take the
//! lock — the orchestration layer (`integrations::run` or a manual
//! `cas integrate` invocation) wraps the call. This keeps platform handlers
//! testable in isolation without requiring filesystem state.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};

use anyhow::Context;
use fs2::FileExt;

const LOCKFILE_NAME: &str = "integrate.lock";

/// Held-while-alive guard. Drops the underlying flock when this value goes
/// out of scope.
pub struct IntegrateLock {
    /// The lock file. `Drop` on `File` closes the descriptor, which releases
    /// the advisory lock on POSIX systems.
    _file: File,
    path: PathBuf,
}

impl IntegrateLock {
    /// Acquire an exclusive lock at `<repo_root>/.cas/integrate.lock`,
    /// blocking until available. Creates `.cas/` and the lockfile if either
    /// does not exist.
    pub fn acquire(repo_root: &Path) -> anyhow::Result<Self> {
        let (file, path) = open_lockfile(repo_root)?;
        file.lock_exclusive()
            .with_context(|| format!("locking {}", path.display()))?;
        Ok(Self { _file: file, path })
    }

    /// Try to acquire the lock without blocking. Returns `None` if another
    /// process currently holds it.
    pub fn try_acquire(repo_root: &Path) -> anyhow::Result<Option<Self>> {
        let (file, path) = open_lockfile(repo_root)?;
        match file.try_lock_exclusive() {
            Ok(()) => Ok(Some(Self { _file: file, path })),
            Err(e) if would_block(&e) => Ok(None),
            Err(e) => Err(anyhow::anyhow!(e))
                .with_context(|| format!("trying lock on {}", path.display())),
        }
    }

    /// Path to the lockfile (mostly for diagnostics).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

fn open_lockfile(repo_root: &Path) -> anyhow::Result<(File, PathBuf)> {
    let cas_dir = repo_root.join(".cas");
    std::fs::create_dir_all(&cas_dir)
        .with_context(|| format!("creating {}", cas_dir.display()))?;
    let path = cas_dir.join(LOCKFILE_NAME);
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)
        .with_context(|| format!("opening {}", path.display()))?;
    Ok((file, path))
}

fn would_block(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::ResourceBusy
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn acquire_creates_cas_dir_and_lockfile() {
        let tmp = TempDir::new().unwrap();
        assert!(!tmp.path().join(".cas").exists());
        let lock = IntegrateLock::acquire(tmp.path()).unwrap();
        assert!(tmp.path().join(".cas/integrate.lock").exists());
        assert_eq!(lock.path(), tmp.path().join(".cas/integrate.lock"));
        drop(lock);
    }

    #[test]
    fn try_acquire_returns_none_when_already_held() {
        let tmp = TempDir::new().unwrap();
        let _held = IntegrateLock::acquire(tmp.path()).unwrap();
        let second = IntegrateLock::try_acquire(tmp.path()).unwrap();
        assert!(second.is_none(), "second lock must fail-fast");
    }

    #[test]
    fn try_acquire_succeeds_after_first_drops() {
        let tmp = TempDir::new().unwrap();
        {
            let _held = IntegrateLock::acquire(tmp.path()).unwrap();
        }
        let lock = IntegrateLock::try_acquire(tmp.path()).unwrap();
        assert!(lock.is_some(), "lock must be re-acquirable after drop");
    }

    #[test]
    fn lock_serializes_two_threads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;
        use std::thread;
        use std::time::Duration;

        let tmp = TempDir::new().unwrap();
        let counter = Arc::new(AtomicUsize::new(0));
        let observed_max = Arc::new(AtomicUsize::new(0));

        let handles: Vec<_> = (0..4)
            .map(|_| {
                let path = tmp.path().to_path_buf();
                let counter = counter.clone();
                let observed_max = observed_max.clone();
                thread::spawn(move || {
                    let _lock = IntegrateLock::acquire(&path).unwrap();
                    let inside = counter.fetch_add(1, Ordering::SeqCst) + 1;
                    observed_max.fetch_max(inside, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    counter.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(
            observed_max.load(Ordering::SeqCst),
            1,
            "exclusive lock must serialize critical section"
        );
    }
}
