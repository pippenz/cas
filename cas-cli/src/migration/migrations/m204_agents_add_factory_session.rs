//! Migration: Add `factory_session` column to `agents`.
//!
//! Factory directors and MCP tools must only see agents that belong to their
//! own factory session. The nullable column preserves legacy/non-factory rows:
//! `NULL` means no factory owner is known.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 204,
    name: "agents_add_factory_session",
    subsystem: Subsystem::Agents,
    description: "Add factory_session TEXT column to agents for per-factory visibility isolation (cas-7baa)",
    up: &[
        "ALTER TABLE agents ADD COLUMN factory_session TEXT",
        "CREATE INDEX IF NOT EXISTS idx_agents_factory_session ON agents(factory_session)",
    ],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM pragma_table_info('agents') WHERE name = 'factory_session') THEN 1 ELSE 0 END",
    ),
};

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    fn agent_columns(conn: &Connection) -> Vec<String> {
        let mut stmt = conn
            .prepare("SELECT name FROM pragma_table_info('agents') ORDER BY cid")
            .unwrap();
        stmt.query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    #[test]
    fn migration_adds_factory_session_column() {
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                agent_type TEXT NOT NULL DEFAULT 'primary',
                role TEXT NOT NULL DEFAULT 'standard',
                status TEXT NOT NULL DEFAULT 'active',
                pid INTEGER,
                ppid INTEGER,
                cc_session_id TEXT,
                parent_id TEXT,
                machine_id TEXT,
                registered_at TEXT NOT NULL,
                last_heartbeat TEXT NOT NULL,
                active_tasks INTEGER NOT NULL DEFAULT 0,
                metadata TEXT NOT NULL DEFAULT '{}',
                startup_confirmed INTEGER NOT NULL DEFAULT 0,
                pid_starttime INTEGER
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

        let cols = agent_columns(&conn);
        assert!(
            cols.contains(&"factory_session".to_string()),
            "factory_session column should exist after migration"
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
            "CREATE TABLE agents (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                factory_session TEXT
            );",
        )
        .unwrap();

        let result: i64 = conn
            .query_row(super::MIGRATION.detect.unwrap(), [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            result, 1,
            "detect should return 1 when column already exists"
        );
    }
}
