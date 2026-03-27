//! Migration: Add index on tasks.team_id for team-scoped queries

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 186,
    name: "tasks_idx_team_id",
    subsystem: Subsystem::Tasks,
    description: "Add index on tasks.team_id for team-scoped filtering",
    up: &[
        "CREATE INDEX IF NOT EXISTS idx_tasks_team_id ON tasks(team_id)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_tasks_team_id'",
    ),
};
