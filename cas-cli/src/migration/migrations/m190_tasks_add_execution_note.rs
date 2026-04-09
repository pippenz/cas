//! Migration: tasks_add_execution_note
//!
//! Adds a nullable `execution_note` column to the tasks table. This field
//! records the execution methodology chosen for a task — one of
//! `test-first`, `characterization-first`, or `additive-only`. The enum is
//! validated at the MCP tool layer rather than via a SQL CHECK constraint so
//! that new values can be added without a schema migration. See cas-7fc1.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 190,
    name: "tasks_add_execution_note",
    subsystem: Subsystem::Tasks,
    description: "Add nullable execution_note column to tasks for methodology tracking",
    up: &["ALTER TABLE tasks ADD COLUMN execution_note TEXT"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('tasks') WHERE name = 'execution_note'"),
};
