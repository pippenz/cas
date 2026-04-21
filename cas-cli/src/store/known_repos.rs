//! Host-scoped repo registry helpers.
//!
//! Thin glue between callers (`cas init`, factory daemon startup, MCP server
//! startup, the `cas known-repos ...` subcommand) and [`SqliteKnownRepoStore`]
//! in `cas-store`. Resolves the host `~/.cas/` directory, ensures it exists,
//! ensures the schema is in place, and exposes a single non-fatal
//! `register_repo` that callers can fire-and-forget without worrying about
//! IO or DB-init order.
//!
//! **Why `dirs::home_dir().join(".cas")` instead of `global_cas_dir()`:**
//! the latter resolves to `~/.config/cas` on Linux / `Application Support`
//! on macOS, which is **not** where the live host CAS state actually lives
//! (sessions, logs, the factory sockets — all under `~/.cas`). Spike A
//! deferred reconciling that inconsistency; this module picks the de-facto
//! root per `ui/factory/session.rs:22-26`.

use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use crate::store::{KnownRepoStore, SqliteKnownRepoStore};

/// Resolve the host-level `~/.cas/` directory. Falls back to `.cas/` under
/// the current directory if the user's home directory cannot be determined,
/// which should only happen in severely sandboxed test environments.
pub fn host_cas_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".cas")
}

/// Open the host-scoped [`SqliteKnownRepoStore`] and ensure its schema.
///
/// Creates `~/.cas/` if missing. Callers that only need to read the registry
/// (e.g. `cas sweep-all`) should use this — it is a read+init path, not a
/// write path.
pub fn open_host_known_repo_store() -> anyhow::Result<SqliteKnownRepoStore> {
    let cas_dir = host_cas_dir();
    std::fs::create_dir_all(&cas_dir)?;
    let store = SqliteKnownRepoStore::open(&cas_dir)?;
    store.init()?;
    Ok(store)
}

/// Register `repo_path` in the host registry, creating the host `~/.cas/`
/// directory + schema on demand.
///
/// **Non-fatal by design.** Every known call site is a best-effort upsert on
/// a startup hot path (`cas init`, factory daemon boot, MCP server boot);
/// losing the upsert must not break the primary operation. Failures are
/// logged at `warn!` and swallowed. If callers need a fatal variant, use
/// [`open_host_known_repo_store`] + [`KnownRepoStore::upsert`] directly.
pub fn register_repo(repo_path: &Path) {
    if let Err(e) = register_repo_strict(repo_path) {
        warn!(
            path = %repo_path.display(),
            error = %e,
            "failed to register repo in host known_repos registry (non-fatal)",
        );
    } else {
        debug!(path = %repo_path.display(), "registered repo in host known_repos");
    }
}

/// Fallible variant of [`register_repo`]. Use this when you actually want
/// to propagate the error (e.g. a CLI `cas known-repos add` explicitly run
/// by the user).
pub fn register_repo_strict(repo_path: &Path) -> anyhow::Result<()> {
    let store = open_host_known_repo_store()?;
    store.upsert(repo_path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // HOME-mutating tests must not run in parallel: cargo test uses a thread
    // pool, and `std::env::set_var` is process-global, so two concurrent
    // tests would see each other's HOME and the second one's upsert would
    // land in the first one's DB (or, worse, a race across canonicalize +
    // shared_db pool keys).
    static HOME_MUTEX: Mutex<()> = Mutex::new(());

    fn with_temp_home<F: FnOnce(&Path)>(f: F) {
        let _guard = HOME_MUTEX
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let temp = TempDir::new().unwrap();
        let prev = std::env::var_os("HOME");
        // SAFETY: the mutex above serializes concurrent writers within this
        // test binary. `register_repo` (the helper under test) reads HOME at
        // most once per call; we restore the prior value after `f` returns.
        unsafe {
            std::env::set_var("HOME", temp.path());
        }
        f(temp.path());
        unsafe {
            match prev {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[test]
    fn host_cas_dir_follows_home() {
        with_temp_home(|home| {
            let resolved = host_cas_dir();
            assert_eq!(resolved, home.join(".cas"));
        });
    }

    #[test]
    fn register_repo_strict_creates_host_dir_and_inserts() {
        with_temp_home(|home| {
            let repo = home.join("myproject");
            std::fs::create_dir_all(&repo).unwrap();

            register_repo_strict(&repo).unwrap();

            let store = open_host_known_repo_store().unwrap();
            let list = store.list().unwrap();
            assert_eq!(list.len(), 1);
            assert_eq!(list[0].path, repo.canonicalize().unwrap());
            assert!(home.join(".cas/cas.db").exists());
        });
    }

    #[test]
    fn register_repo_is_non_fatal_on_bad_path() {
        with_temp_home(|_home| {
            // A path with a NUL byte would fail to canonicalize AND fail to
            // insert — the non-strict variant must not panic.
            register_repo(Path::new("/nonexistent/path/that/is/fine"));
        });
    }
}
