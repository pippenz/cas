//! Migration: Add `share` column to the `tasks` table.
//!
//! Fresh DBs already have this column via CREATE TABLE. Older DBs that pre-date
//! the column addition need it backfilled via ALTER TABLE. The detect query is
//! idempotent: returns 1 (already applied) when the column exists, 0 when missing.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 198,
    name: "tasks_add_share",
    subsystem: Subsystem::Tasks,
    description: "Add share TEXT column to tasks table for older DBs that predate the column",
    up: &["ALTER TABLE tasks ADD COLUMN share TEXT"],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM pragma_table_info('tasks') WHERE name = 'share') THEN 1 ELSE 0 END",
    ),
};

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    fn create_tasks_without_share(conn: &Connection) {
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

    fn has_share_column(conn: &Connection) -> bool {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'share'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        count > 0
    }

    #[test]
    fn fresh_db_gets_share_column() {
        let conn = Connection::open_in_memory().unwrap();
        create_tasks_without_share(&conn);
        assert!(!has_share_column(&conn));

        apply_up(&conn);

        assert!(has_share_column(&conn), "share column must exist after up");
    }

    #[test]
    fn detect_returns_0_when_column_missing_and_1_when_present() {
        let conn = Connection::open_in_memory().unwrap();
        create_tasks_without_share(&conn);

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 0, "detect must return 0 when share column is absent");

        apply_up(&conn);

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect must return 1 after share column is added");
    }

    #[test]
    fn idempotent_when_share_already_present() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tasks (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'open',
                share TEXT
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tasks (id, title, status, share) VALUES ('t1', 'fix bug', 'open', 'team')",
            [],
        )
        .unwrap();

        // detect must return 1 — migration runner will skip up
        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(result, 1, "detect must return 1 when share column already exists");

        // Existing row must survive untouched
        let share: Option<String> = conn
            .query_row("SELECT share FROM tasks WHERE id = 't1'", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(share.as_deref(), Some("team"));
    }
}
