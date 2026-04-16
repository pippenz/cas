//! Migration: tasks_add_share — T5 scope consistency.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 121,
    name: "tasks_add_share",
    subsystem: Subsystem::Tasks,
    description: "Add share column to tasks (T5 scope consistency)",
    up: &["ALTER TABLE tasks ADD COLUMN share TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'share'"),
};
