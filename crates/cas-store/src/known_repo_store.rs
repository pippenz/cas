//! Host-scoped registry of CAS-aware repositories.
//!
//! Records every repo directory where `cas init` has run, where a factory
//! daemon has launched, or where an MCP server has started with a `.cas/`
//! directory in CWD. Used by cross-repo sweep tooling (`cas sweep-all`,
//! `cas worktree sweep --all-repos`) so the sweeper can enumerate every
//! candidate repo without relying on a filesystem scan.
//!
//! **Scope: host, not repo.** All callers pass `dirs::home_dir().join(".cas")`
//! as the `cas_dir`; the backing DB is `~/.cas/cas.db`, shared across every
//! project on the machine. This is deliberate — the whole point of the
//! registry is that one repo's daemon can discover every *other* repo.
//!
//! See `docs/brainstorms/2026-04-21-worktree-leak-and-supervisor-discipline-spike-a.md`
//! for the design rationale.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};

use crate::error::StoreError;

type Result<T> = std::result::Result<T, StoreError>;

/// A repository known to the host CAS installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KnownRepo {
    /// Canonicalized absolute path to the repo root (the directory containing
    /// `.cas/`, not the `.cas/` directory itself).
    pub path: PathBuf,
    /// UTC timestamp of the first upsert.
    pub first_seen_at: DateTime<Utc>,
    /// UTC timestamp of the most recent upsert.
    pub last_touched_at: DateTime<Utc>,
    /// Number of times this path has been upserted.
    pub touch_count: u64,
}

/// Registry trait for host-scoped repo discovery.
pub trait KnownRepoStore: Send + Sync {
    /// Ensure the schema exists. Production paths rely on migrations;
    /// this is present for tests and standalone usage.
    fn init(&self) -> Result<()>;

    /// Register a repo path, or bump its `last_touched_at` + `touch_count`
    /// if already present. The path is canonicalized before insertion so
    /// callers can pass a relative or symlinked path.
    ///
    /// A path that does not exist on disk is still accepted — callers may
    /// want to upsert a cached path even when the repo has been moved —
    /// but the value is stored as-given after canonicalization attempts.
    fn upsert(&self, path: &Path) -> Result<()>;

    /// Update only `last_touched_at` and `touch_count` for an already-known
    /// path. Returns the number of rows touched (0 if the path is unknown).
    fn touch(&self, path: &Path) -> Result<usize>;

    /// Remove a path from the registry (e.g., the repo was deleted). Returns
    /// the number of rows removed.
    fn forget(&self, path: &Path) -> Result<usize>;

    /// List all known repos, ordered by `last_touched_at` descending (most
    /// recent first).
    fn list(&self) -> Result<Vec<KnownRepo>>;

    /// Count of known repos.
    fn count(&self) -> Result<usize>;
}

/// SQLite-backed `KnownRepoStore` sharing the process connection pool.
pub struct SqliteKnownRepoStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteKnownRepoStore {
    /// Open (or create) a store rooted at the given `cas_dir`. The database
    /// lives at `<cas_dir>/cas.db`. For the host registry callers pass
    /// `dirs::home_dir().join(".cas")`; tests pass a `TempDir` path.
    pub fn open(cas_dir: &Path) -> Result<Self> {
        let db_path = cas_dir.join("cas.db");
        let conn = crate::shared_db::shared_connection(&db_path)?;
        Ok(Self { conn })
    }

    fn parse_ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_else(|_| Utc::now())
    }

    fn row_to_known_repo(row: &rusqlite::Row) -> rusqlite::Result<KnownRepo> {
        let path_str: String = row.get(0)?;
        let first_seen: String = row.get(1)?;
        let last_touched: String = row.get(2)?;
        let touch_count: i64 = row.get(3)?;
        Ok(KnownRepo {
            path: PathBuf::from(path_str),
            first_seen_at: Self::parse_ts(&first_seen),
            last_touched_at: Self::parse_ts(&last_touched),
            touch_count: touch_count.max(0) as u64,
        })
    }

    /// Canonicalize a path for storage. Falls back to the original path if
    /// canonicalization fails (e.g., path does not exist yet).
    fn canonical_key(path: &Path) -> String {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        canonical.to_string_lossy().into_owned()
    }
}

impl KnownRepoStore for SqliteKnownRepoStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS known_repos (\
                path TEXT PRIMARY KEY,\
                first_seen_at TEXT NOT NULL,\
                last_touched_at TEXT NOT NULL,\
                touch_count INTEGER NOT NULL DEFAULT 1\
            );\
            CREATE INDEX IF NOT EXISTS idx_known_repos_last_touched \
            ON known_repos(last_touched_at DESC);",
        )?;
        Ok(())
    }

    fn upsert(&self, path: &Path) -> Result<()> {
        let key = Self::canonical_key(path);
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO known_repos (path, first_seen_at, last_touched_at, touch_count) \
             VALUES (?1, ?2, ?2, 1) \
             ON CONFLICT(path) DO UPDATE SET \
                last_touched_at = excluded.last_touched_at, \
                touch_count = touch_count + 1",
            params![key, now],
        )?;
        Ok(())
    }

    fn touch(&self, path: &Path) -> Result<usize> {
        let key = Self::canonical_key(path);
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE known_repos SET last_touched_at = ?1, touch_count = touch_count + 1 \
             WHERE path = ?2",
            params![now, key],
        )?;
        Ok(rows)
    }

    fn forget(&self, path: &Path) -> Result<usize> {
        let key = Self::canonical_key(path);
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM known_repos WHERE path = ?", params![key])?;
        Ok(rows)
    }

    fn list(&self) -> Result<Vec<KnownRepo>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT path, first_seen_at, last_touched_at, touch_count \
             FROM known_repos ORDER BY last_touched_at DESC",
        )?;
        let rows = stmt
            .query_map([], Self::row_to_known_repo)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();
        let n: i64 =
            conn.query_row("SELECT COUNT(*) FROM known_repos", [], |row| row.get(0))?;
        Ok(n.max(0) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn store() -> (TempDir, SqliteKnownRepoStore) {
        let temp = TempDir::new().unwrap();
        let store = SqliteKnownRepoStore::open(temp.path()).unwrap();
        store.init().unwrap();
        (temp, store)
    }

    #[test]
    fn upsert_inserts_new_path() {
        let (temp, store) = store();
        let repo = temp.path().join("myrepo");
        std::fs::create_dir_all(&repo).unwrap();

        store.upsert(&repo).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].touch_count, 1);
        // Stored as canonicalized absolute path.
        assert_eq!(list[0].path, repo.canonicalize().unwrap());
    }

    #[test]
    fn upsert_bumps_touch_count_on_conflict() {
        let (temp, store) = store();
        let repo = temp.path().join("myrepo");
        std::fs::create_dir_all(&repo).unwrap();

        store.upsert(&repo).unwrap();
        store.upsert(&repo).unwrap();
        store.upsert(&repo).unwrap();

        let list = store.list().unwrap();
        assert_eq!(list.len(), 1, "dedup on canonical path");
        assert_eq!(list[0].touch_count, 3);
        assert!(list[0].last_touched_at >= list[0].first_seen_at);
    }

    #[test]
    fn touch_updates_only_existing() {
        let (temp, store) = store();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        assert_eq!(store.touch(&repo).unwrap(), 0, "unknown path = 0 rows");
        store.upsert(&repo).unwrap();
        assert_eq!(store.touch(&repo).unwrap(), 1);

        let list = store.list().unwrap();
        assert_eq!(list[0].touch_count, 2);
    }

    #[test]
    fn forget_removes_path() {
        let (temp, store) = store();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();

        store.upsert(&repo).unwrap();
        assert_eq!(store.count().unwrap(), 1);

        assert_eq!(store.forget(&repo).unwrap(), 1);
        assert_eq!(store.count().unwrap(), 0);
        assert_eq!(store.forget(&repo).unwrap(), 0, "idempotent on missing");
    }

    #[test]
    fn list_ordered_by_last_touched_desc() {
        let (temp, store) = store();
        let a = temp.path().join("a");
        let b = temp.path().join("b");
        let c = temp.path().join("c");
        for p in [&a, &b, &c] {
            std::fs::create_dir_all(p).unwrap();
        }

        store.upsert(&a).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.upsert(&b).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));
        store.upsert(&c).unwrap();

        let list = store.list().unwrap();
        let names: Vec<_> = list
            .iter()
            .map(|r| r.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["c", "b", "a"]);
    }

    #[test]
    fn nonexistent_path_is_still_recorded() {
        let (temp, store) = store();
        // A path that will never exist on disk — we still want to accept it
        // so a repo that just moved can be recorded and flagged later.
        let ghost = temp.path().join("never-existed");

        store.upsert(&ghost).unwrap();
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        // canonicalize fails → falls back to path-as-given.
        assert_eq!(list[0].path, ghost);
    }

    #[test]
    fn symlink_upsert_canonicalizes_to_target() {
        use std::os::unix::fs::symlink;
        let (temp, store) = store();
        let real = temp.path().join("real");
        let link = temp.path().join("link");
        std::fs::create_dir_all(&real).unwrap();
        symlink(&real, &link).unwrap();

        store.upsert(&link).unwrap();
        store.upsert(&real).unwrap();

        // Both resolve to the same canonical path; should be one row, count=2.
        let list = store.list().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].touch_count, 2);
    }
}
