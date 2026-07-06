//! Migration: Create `idx_agents_factory_session` on `agents(factory_session)`.
//!
//! Split out of the baseline `AGENT_SCHEMA` (2026-07-06 hotfix): the baseline
//! executes at every store open, BEFORE migrations, so an index on the
//! m204-added column aborted store init on every pre-m204 database and
//! bricked `cas serve`. As a migration it is ordered after m204, so the
//! column exists on every path:
//! - old DB: m204 adds the column (and, historically, this index), then this
//!   migration's detect sees the index and no-ops;
//! - fresh DB: the baseline table already has the column, m204's detect
//!   skips it, and this migration creates the index.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 205,
    name: "agents_factory_session_index",
    subsystem: Subsystem::Agents,
    description: "Create idx_agents_factory_session (moved out of baseline AGENT_SCHEMA — indexing a migration-added column from the baseline broke store init on pre-m204 databases)",
    up: &["CREATE INDEX IF NOT EXISTS idx_agents_factory_session ON agents(factory_session)"],
    detect: Some(
        "SELECT CASE WHEN EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'index' AND name = 'idx_agents_factory_session') THEN 1 ELSE 0 END",
    ),
};

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    /// Pre-m204 agents table shape (no factory_session column).
    const OLD_AGENTS_TABLE: &str = "CREATE TABLE agents (
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
    );";

    fn index_exists(conn: &Connection) -> bool {
        conn.query_row(super::MIGRATION.detect.unwrap(), [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap()
            == 1
    }

    /// THE regression guard for the 2026-07-06 serve brick: the baseline
    /// AGENT_SCHEMA must apply cleanly over a pre-m204 database. The broken
    /// build indexed factory_session from the baseline, which aborted store
    /// init before migrations could run.
    #[test]
    fn baseline_agent_schema_applies_over_pre_m204_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(OLD_AGENTS_TABLE).unwrap();

        conn.execute_batch(cas_store::AGENT_SCHEMA)
            .expect("baseline AGENT_SCHEMA must apply over a pre-m204 agents table");
    }

    #[test]
    fn index_created_on_fresh_schema_where_m204_detect_skips() {
        let conn = Connection::open_in_memory().unwrap();
        // Fresh DB: baseline creates the table WITH the column, m204's
        // column-existence detect returns 1 (skip), so this migration must
        // create the index.
        conn.execute_batch(cas_store::AGENT_SCHEMA).unwrap();
        assert!(!index_exists(&conn), "baseline must not carry the index");

        for sql in super::MIGRATION.up {
            conn.execute(sql, []).unwrap();
        }
        assert!(index_exists(&conn), "m205 must create the index");
    }

    #[test]
    fn detect_skips_when_m204_already_created_the_index() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(OLD_AGENTS_TABLE).unwrap();
        // Old DB path: m204 ran (column + index).
        conn.execute("ALTER TABLE agents ADD COLUMN factory_session TEXT", [])
            .unwrap();
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_agents_factory_session ON agents(factory_session)",
            [],
        )
        .unwrap();

        assert!(index_exists(&conn), "detect must return 1 → migration skipped");
        // up is IF NOT EXISTS, so even a forced run is harmless.
        for sql in super::MIGRATION.up {
            conn.execute(sql, []).unwrap();
        }
    }
}
