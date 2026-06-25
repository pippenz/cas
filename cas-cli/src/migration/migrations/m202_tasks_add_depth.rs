//! Migration: Add `depth` column to the `tasks` table (EPIC cas-1255).
//!
//! Fresh DBs already have this column via CREATE TABLE. Older DBs that pre-date
//! the column addition need it backfilled via ALTER TABLE. The detect query is
//! idempotent: returns 1 (already applied) when the column exists, 0 when missing.
//!
//! Existing rows get NULL, which the task store maps to `TaskDepth::Deep` on
//! read, so legacy tasks read as deep.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 202,
    name: "tasks_add_depth",
    subsystem: Subsystem::Tasks,
    description: "Add depth TEXT column to tasks table (per-task speed mode, EPIC cas-1255)",
    up: &["ALTER TABLE tasks ADD COLUMN depth TEXT"],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM pragma_table_info('tasks') WHERE name = 'depth') THEN 1 ELSE 0 END",
    ),
};

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    fn create_tasks_without_depth(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open'
            );",
        )
        .unwrap();
    }

    fn apply_up(conn: &Connection) {
        for sql in super::MIGRATION.up {
            conn.execute(sql, []).unwrap();
        }
    }

    fn has_depth_column(conn: &Connection) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'depth'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        count > 0
    }

    #[test]
    fn fresh_db_gets_depth_column() {
        let conn = Connection::open_in_memory().unwrap();
        create_tasks_without_depth(&conn);
        assert!(!has_depth_column(&conn));

        apply_up(&conn);

        assert!(has_depth_column(&conn), "depth column must exist after up");
    }

    #[test]
    fn detect_returns_0_when_column_missing_and_1_when_present() {
        let conn = Connection::open_in_memory().unwrap();
        create_tasks_without_depth(&conn);

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 0, "detect must return 0 when depth column is absent");

        apply_up(&conn);

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect must return 1 after depth column is added");
    }

    #[test]
    fn idempotent_when_depth_already_present() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                depth TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, title, status, depth) VALUES ('t1', 'fix bug', 'open', 'light')",
            [],
        )
        .unwrap();

        // detect must return 1 — migration runner will skip up
        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect must return 1 when depth column already exists");

        // Existing row must survive untouched
        let depth: Option<String> = conn
            .query_row("SELECT depth FROM tasks WHERE id = 't1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(depth.as_deref(), Some("light"));
    }
}
