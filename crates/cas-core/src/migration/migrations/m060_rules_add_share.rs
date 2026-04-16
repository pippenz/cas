//! Migration: rules_add_share
//!
//! Adds `share` column for per-rule team-promotion override (T5).
//! Dormant in the current release — no CLI writes rules.share yet —
//! but shipping the column now keeps the four share-aware tables
//! (entries, rules, skills, tasks) structurally aligned so a future
//! `cas rule share` surface can wire CRUD without another migration.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 60,
    name: "rules_add_share",
    subsystem: Subsystem::Rules,
    description: "Add share column to rules (T5 scope consistency)",
    up: &["ALTER TABLE rules ADD COLUMN share TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('rules') WHERE name = 'share'"),
};
