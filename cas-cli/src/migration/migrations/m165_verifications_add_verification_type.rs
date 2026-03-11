//! Migration: Add verification_type column to verifications table
//!
//! This column distinguishes between task-level verification (individual subtask)
//! and epic-level verification (merged code on master).

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 165,
    name: "verifications_add_verification_type",
    subsystem: Subsystem::Verification,
    description: "Add verification_type column to verifications for epic vs task verification",
    up: &["ALTER TABLE verifications ADD COLUMN verification_type TEXT NOT NULL DEFAULT 'task'"],
    detect: Some(
        "SELECT COUNT(*) FROM pragma_table_info('verifications') WHERE name = 'verification_type'",
    ),
};
