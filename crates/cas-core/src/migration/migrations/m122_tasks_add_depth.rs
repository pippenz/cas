//! Migration: tasks_add_depth — per-task execution depth (EPIC cas-1255).
//!
//! Adds the `depth` column to `tasks`. Existing rows get NULL, which the
//! store maps to `TaskDepth::Deep` on read, so legacy tasks read as deep.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 122,
    name: "tasks_add_depth",
    subsystem: Subsystem::Tasks,
    description: "Add depth column to tasks (per-task speed mode, EPIC cas-1255)",
    up: &["ALTER TABLE tasks ADD COLUMN depth TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'depth'"),
};
