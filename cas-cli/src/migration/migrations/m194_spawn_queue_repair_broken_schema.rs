//! Migration: Repair broken spawn_queue schemas left behind by an earlier bug in m193.
//!
//! An earlier revision of `m193_spawn_queue_force_isolate` created `spawn_queue` with a
//! completely wrong schema (task_type/task_id/priority/payload/status) on fresh `cas init`.
//! The hand-rolled `SpawnQueueStore::init()` then became a no-op because of
//! `CREATE TABLE IF NOT EXISTS`. Subsequent factory `spawn_workers` calls failed with
//! "no such column: action".
//!
//! This migration detects the broken schema by checking for the `task_type` column (which
//! does not exist in the correct schema), drops the broken table, and recreates it with
//! the correct schema. Broken-schema DBs by definition cannot hold any queue rows the
//! application can read, so the drop is safe.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 194,
    name: "spawn_queue_repair_broken_schema",
    subsystem: Subsystem::Agents,
    description: "Drop and recreate spawn_queue if it was created with the broken m193 schema",
    up: &[
        "DROP TABLE IF EXISTS spawn_queue",
        "CREATE TABLE IF NOT EXISTS spawn_queue (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            action TEXT NOT NULL,
            count INTEGER,
            worker_names TEXT,
            force INTEGER NOT NULL DEFAULT 0,
            isolate INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            processed_at TEXT
        )",
        "CREATE INDEX IF NOT EXISTS idx_spawn_queue_pending ON spawn_queue(action) WHERE processed_at IS NULL",
    ],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM pragma_table_info('spawn_queue') WHERE name = 'task_type') THEN 0 ELSE 1 END",
    ),
};

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    fn spawn_queue_columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('spawn_queue') ORDER BY cid")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn apply_up(conn: &Connection) {
        for sql in super::MIGRATION.up {
            conn.execute(sql, []).unwrap();
        }
    }

    #[test]
    fn fresh_init_produces_correct_schema() {
        let conn = Connection::open_in_memory().unwrap();
        apply_up(&conn);

        let cols = spawn_queue_columns(&conn);
        assert_eq!(
            cols,
            vec![
                "id",
                "action",
                "count",
                "worker_names",
                "force",
                "isolate",
                "created_at",
                "processed_at"
            ]
        );

        // Confirm INSERT with expected columns works
        conn.execute(
            "INSERT INTO spawn_queue (action, count, worker_names) VALUES ('spawn', 1, 'a,b')",
            [],
        )
        .unwrap();
    }

    #[test]
    fn repair_drops_and_recreates_broken_schema() {
        let conn = Connection::open_in_memory().unwrap();

        // Simulate a DB created with the original broken m193 schema.
        conn.execute_batch(
            "CREATE TABLE spawn_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                task_type TEXT NOT NULL,
                task_id TEXT,
                priority INTEGER NOT NULL DEFAULT 2,
                payload TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                created_at TEXT NOT NULL,
                processed_at TEXT,
                force INTEGER NOT NULL DEFAULT 0,
                isolate INTEGER NOT NULL DEFAULT 0
            );",
        )
        .unwrap();

        // Detect should fire (broken → returns 0 meaning "not yet applied / needs repair").
        let detect_result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            detect_result, 0,
            "detect must return 0 on broken schema so migration runs"
        );

        apply_up(&conn);

        let cols = spawn_queue_columns(&conn);
        assert!(
            !cols.contains(&"task_type".to_string()),
            "task_type column should be gone after repair"
        );
        assert!(cols.contains(&"action".to_string()));
        assert!(cols.contains(&"worker_names".to_string()));

        // After repair, detect should report applied (returns 1 = "already applied").
        let detect_result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            detect_result, 1,
            "detect must return 1 after repair so migration is skipped next time"
        );
    }

    #[test]
    fn idempotent_on_correct_schema() {
        let conn = Connection::open_in_memory().unwrap();

        // Simulate a DB that already has the correct schema (legacy or fresh-after-fix).
        conn.execute_batch(
            "CREATE TABLE spawn_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action TEXT NOT NULL,
                count INTEGER,
                worker_names TEXT,
                force INTEGER NOT NULL DEFAULT 0,
                isolate INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                processed_at TEXT
            );",
        )
        .unwrap();

        // Insert a row that should survive — the repair must not drop correct tables.
        conn.execute(
            "INSERT INTO spawn_queue (action, count) VALUES ('spawn', 2)",
            [],
        )
        .unwrap();

        // Detect should report already-applied (returns 1).
        let detect_result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            detect_result, 1,
            "detect must return 1 on already-correct schema so migration is skipped"
        );

        // Row should still be there (migration runner wouldn't fire up, but confirm data is intact).
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM spawn_queue", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
