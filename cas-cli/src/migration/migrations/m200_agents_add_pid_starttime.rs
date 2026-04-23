//! Migration: agents_add_pid_starttime
//!
//! EPIC cas-9508 / cas-b157: promote the PID-reuse fingerprint from
//! `Agent.metadata[PID_STARTTIME_KEY]` to a first-class typed field.
//!
//! The metadata key is still written as a shadow entry (and read as
//! fallback in the liveness gate) so agents registered on a pre-cas-b157
//! binary and revived mid-flight keep working across the upgrade. The
//! shadow write can be dropped in a future release once fleet rollout
//! is confirmed.

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 200,
    name: "agents_add_pid_starttime",
    subsystem: Subsystem::Agents,
    description:
        "Add pid_starttime column (Linux /proc/<pid>/stat field 22) to agents for typed PID-reuse fingerprint (cas-b157)",
    up: &[
        "ALTER TABLE agents ADD COLUMN pid_starttime INTEGER",
        // No backfill: legacy rows keep NULL here. The liveness gate
        // reads metadata[PID_STARTTIME_KEY] as fallback, which is where
        // cas-ea46 stashed the value originally.
    ],
    detect: Some(
        "SELECT COUNT(*) FROM pragma_table_info('agents') WHERE name = 'pid_starttime'",
    ),
};
