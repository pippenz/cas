//! Migration: Create id_sequences table for O(1) ID generation
//!
//! Replaces per-insert MAX(LIKE) scans with an atomic sequence counter.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 188,
    name: "id_sequences_create_table",
    subsystem: Subsystem::Entries,
    description: "Create id_sequences table for O(1) ID generation",
    up: &[
        "CREATE TABLE IF NOT EXISTS id_sequences (
            name TEXT PRIMARY KEY,
            next_val INTEGER NOT NULL DEFAULT 1
        )",
        // Seed from existing data so sequences continue from the current max
        "INSERT OR IGNORE INTO id_sequences (name, next_val)
         SELECT 'rule', COALESCE(MAX(CAST(SUBSTR(id, 6) AS INTEGER)), 0) + 1
         FROM rules WHERE id LIKE 'rule-%'",
        "INSERT OR IGNORE INTO id_sequences (name, next_val)
         SELECT 'entity', COALESCE(MAX(CAST(SUBSTR(id, 5) AS INTEGER)), 0) + 1
         FROM entities WHERE id LIKE 'ent-%'",
        "INSERT OR IGNORE INTO id_sequences (name, next_val)
         SELECT 'relationship', COALESCE(MAX(CAST(SUBSTR(id, 5) AS INTEGER)), 0) + 1
         FROM relationships WHERE id LIKE 'rel-%'",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='id_sequences'",
    ),
};
