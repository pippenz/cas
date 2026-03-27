//! Migration: Add index on tasks.assignee for assignee-filtered queries

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 187,
    name: "tasks_idx_assignee",
    subsystem: Subsystem::Tasks,
    description: "Add index on tasks.assignee for assignee-filtered queries",
    up: &[
        "CREATE INDEX IF NOT EXISTS idx_tasks_assignee ON tasks(assignee)",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_tasks_assignee'",
    ),
};
