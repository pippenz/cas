//! Migration: Add index on rules.team_id for team-scoped queries

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 184,
    name: "rules_idx_team_id",
    subsystem: Subsystem::Rules,
    description: "Add index on rules.team_id for team-scoped filtering",
    up: &[
        "CREATE INDEX IF NOT EXISTS idx_rules_team_id ON rules(team_id)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_rules_team_id'",
    ),
};
