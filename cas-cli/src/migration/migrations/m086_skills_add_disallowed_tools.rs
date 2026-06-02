//! Migration: skills_add_disallowed_tools

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 86,
    name: "skills_add_disallowed_tools",
    subsystem: Subsystem::Skills,
    description: "Add disallowed_tools column for Claude Code 'disallowed-tools' frontmatter (2.1.152+)",
    up: &["ALTER TABLE skills ADD COLUMN disallowed_tools TEXT NOT NULL DEFAULT '[]'"],
    detect: Some("SELECT COUNT(*) FROM pragma_table_info('skills') WHERE name = 'disallowed_tools'"),
};
