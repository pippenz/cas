//! Migration: Add `task_id` column to `spawn_queue`.
//!
//! Lets `spawn_workers` pre-assign a task to the spawned worker at
//! spawn-request-consumption time, eliminating the "first-claim stall" race
//! where a spawn-time assignment message could sit in `prompt_queue` before
//! the worker was registered (cas-6913). The nullable column preserves
//! legacy rows: `NULL` means "no pre-assignment requested" (the pre-cas-6913
//! behavior).

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 206,
    name: "spawn_queue_add_task_id",
    subsystem: Subsystem::Agents,
    description: "Add task_id TEXT column to spawn_queue for spawn-time task pre-assignment (cas-6913)",
    up: &["ALTER TABLE spawn_queue ADD COLUMN task_id TEXT"],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM pragma_table_info('spawn_queue') WHERE name = 'task_id') THEN 1 ELSE 0 END",
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

    #[test]
    fn migration_adds_task_id_column() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "CREATE TABLE spawn_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action TEXT NOT NULL,
                count INTEGER,
                worker_names TEXT,
                force INTEGER NOT NULL DEFAULT 0,
                isolate INTEGER NOT NULL DEFAULT 0,
                worker_spec TEXT,
                factory_session TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                processed_at TEXT
            );",
        )
        .unwrap();

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 0, "detect should return 0 on pre-migration schema");

        for sql in super::MIGRATION.up {
            conn.execute(sql, []).unwrap();
        }

        let cols = spawn_queue_columns(&conn);
        assert!(
            cols.contains(&"task_id".to_string()),
            "task_id column should exist after migration"
        );

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect should return 1 after migration");
    }

    #[test]
    fn idempotent_detect_on_fresh_schema() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "CREATE TABLE spawn_queue (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                action TEXT NOT NULL,
                count INTEGER,
                worker_names TEXT,
                force INTEGER NOT NULL DEFAULT 0,
                isolate INTEGER NOT NULL DEFAULT 0,
                worker_spec TEXT,
                factory_session TEXT,
                task_id TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                processed_at TEXT
            );",
        )
        .unwrap();

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect must return 1 when column already exists");
    }
}
