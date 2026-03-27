//! Migration: Add index on entries.team_id for team-scoped queries

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 183,
    name: "entries_idx_team_id",
    subsystem: Subsystem::Entries,
    description: "Add index on entries.team_id for team-scoped filtering",
    up: &[
        "CREATE INDEX IF NOT EXISTS idx_entries_team_id ON entries(team_id)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_entries_team_id'",
    ),
};
