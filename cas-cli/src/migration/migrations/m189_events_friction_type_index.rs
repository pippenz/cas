//! Migration: Add generated column + index for friction_type json_extract queries
//!
//! friction_summary and friction_by_type use json_extract(metadata, '$.friction_type')
//! in WHERE clauses, which is unindexable. Adding a generated column with an index
//! allows SQLite to use index scans instead of full table scans.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 189,
    name: "events_friction_type_index",
    subsystem: Subsystem::Events,
    description: "Add generated column + composite index for friction_type queries on events",
    up: &[
        "ALTER TABLE events ADD COLUMN friction_type TEXT
         GENERATED ALWAYS AS (json_extract(metadata, '$.friction_type')) VIRTUAL",
        "CREATE INDEX IF NOT EXISTS idx_events_friction_type
         ON events (event_type, friction_type) WHERE friction_type IS NOT NULL",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM pragma_table_xinfo('events') WHERE name = 'friction_type'",
    ),
};
