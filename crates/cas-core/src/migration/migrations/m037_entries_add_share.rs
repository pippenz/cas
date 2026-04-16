//! Migration: entries_add_share
//!
//! Adds `share` column for the per-entry team-promotion override
//! introduced in T5 (cas-07d7). Values are the lowercase serde forms of
//! `ShareScope` — "private" or "team". `NULL` means the T1 auto-rule
//! applies (project-scope, non-Preference entries dual-enqueue when a
//! team is configured). See `docs/requests/team-memories-filter-policy.md`.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 37,
    name: "entries_add_share",
    subsystem: Subsystem::Entries,
    description: "Add share column to entries for T5 `cas memory share`",
    up: &["ALTER TABLE entries ADD COLUMN share TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('entries') WHERE name = 'share'"),
};
