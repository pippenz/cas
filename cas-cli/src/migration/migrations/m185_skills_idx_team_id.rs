//! Migration: Add index on skills.team_id for team-scoped queries

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 185,
    name: "skills_idx_team_id",
    subsystem: Subsystem::Skills,
    description: "Add index on skills.team_id for team-scoped filtering",
    up: &[
        "CREATE INDEX IF NOT EXISTS idx_skills_team_id ON skills(team_id)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_skills_team_id'",
    ),
};
