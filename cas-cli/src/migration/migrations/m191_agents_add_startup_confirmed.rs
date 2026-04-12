//! Migration: agents_add_startup_confirmed

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 191,
    name: "agents_add_startup_confirmed",
    subsystem: Subsystem::Agents,
    description: "Add startup_confirmed column to distinguish 'registered but never started' from 'running' agents",
    up: &["ALTER TABLE agents ADD COLUMN startup_confirmed INTEGER NOT NULL DEFAULT 0"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('agents') WHERE name = 'startup_confirmed'"),
};
