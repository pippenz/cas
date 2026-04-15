//! Migration: Move spawn_queue table creation from hand-rolled init() to migration framework
//!
//! The spawn_queue table was previously created by `SpawnQueueStore::init()` with an inline
//! CREATE TABLE IF NOT EXISTS. Migrations now run before hand-rolled init, so this migration
//! must create the table with the correct schema (matching SPAWN_QUEUE_SCHEMA in
//! `crates/cas-store/src/spawn_queue_store.rs`). If hand-rolled init ran first historically
//! (legacy DBs), the detect clause finds the `force` column and skips this migration.
//!
//! NOTE: An earlier revision of this migration used a wrong schema (task_type/task_id/...),
//! which broke fresh `cas init` on DBs created between that revision and this fix.
//! `m194_spawn_queue_repair_broken_schema` repairs those DBs in place.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 193,
    name: "spawn_queue_force_isolate",
    subsystem: Subsystem::Agents,
    description: "Create spawn_queue table (moved from hand-rolled init)",
    up: &[
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
        "SELECT COUNT(*) FROM pragma_table_info('spawn_queue') WHERE name = 'force'",
    ),
};
