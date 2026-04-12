//! Migration: Move spawn_queue force/isolate columns from hand-rolled init() to migration framework
//!
//! These columns were previously added by inline pragmas in spawn_queue_store.rs init().
//! The inline schema already includes them, so this migration only exists for old DBs that
//! were created before the columns were added to the inline schema.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 193,
    name: "spawn_queue_force_isolate",
    subsystem: Subsystem::Agents,
    description: "Ensure spawn_queue has force and isolate columns (moved from hand-rolled init)",
    up: &[
        "CREATE TABLE IF NOT EXISTS spawn_queue (id INTEGER PRIMARY KEY AUTOINCREMENT, task_type TEXT NOT NULL, task_id TEXT, priority INTEGER NOT NULL DEFAULT 2, payload TEXT, status TEXT NOT NULL DEFAULT 'pending', created_at TEXT NOT NULL, processed_at TEXT, force INTEGER NOT NULL DEFAULT 0, isolate INTEGER NOT NULL DEFAULT 0)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM pragma_table_info('spawn_queue') WHERE name = 'force'",
    ),
};
