//! Migration: agents_add_startup_confirmed

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 191,
    name: "agents_add_startup_confirmed",
    subsystem: Subsystem::Agents,
    description: "Add startup_confirmed column to distinguish 'registered but never started' from 'running' agents",
    up: &[
        "ALTER TABLE agents ADD COLUMN startup_confirmed INTEGER NOT NULL DEFAULT 0",
        // Backfill: all existing agents are already running, so mark them confirmed
        // to avoid a thundering-herd mass-eviction on first maintenance cycle post-migration.
        "UPDATE agents SET startup_confirmed = 1 WHERE status IN ('active', 'idle')",
    ],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('agents') WHERE name = 'startup_confirmed'"),
};
