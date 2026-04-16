//! Migration: skills_add_share — T5 scope consistency.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 82,
    name: "skills_add_share",
    subsystem: Subsystem::Skills,
    description: "Add share column to skills (T5 scope consistency)",
    up: &["ALTER TABLE skills ADD COLUMN share TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('skills') WHERE name = 'share'"),
};
