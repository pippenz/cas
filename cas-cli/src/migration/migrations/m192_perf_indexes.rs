//! Migration: Add missing indexes for hot polling paths
//!
//! Eliminates full table scans on frequently-polled queries:
//! - list_pending_verification: partial index on tasks(pending_verification)
//! - list_pending_worktree_merge: partial index on tasks(pending_worktree_merge)
//! - list_ready: composite index on dependencies(from_id, dep_type) for correlated subquery
//! - agent_get_by_pid: index on agents(pid)
//! - friction/severity aggregations: generated column + composite index on events

use crate::migration::{Migration, Subsystem};

pub const MIGRATION: Migration = Migration {
    id: 192,
    name: "perf_indexes",
    subsystem: Subsystem::Tasks,
    description:
        "Add indexes for hot polling paths: pending_verification, pending_worktree_merge, list_ready, agent_get_by_pid, events severity",
    up: &[
        // Partial index for list_pending_verification — only rows where pending_verification = 1
        "CREATE INDEX IF NOT EXISTS idx_tasks_pending_verification
         ON tasks(pending_verification) WHERE pending_verification = 1",
        // Partial index for list_pending_worktree_merge — only rows where pending_worktree_merge = 1
        "CREATE INDEX IF NOT EXISTS idx_tasks_pending_worktree_merge
         ON tasks(pending_worktree_merge) WHERE pending_worktree_merge = 1",
        // Composite index for list_ready correlated subquery: WHERE d.from_id = t.id AND d.dep_type = 'blocks'
        "CREATE INDEX IF NOT EXISTS idx_deps_from_type
         ON dependencies(from_id, dep_type)",
        // Index for agent_get_by_pid: WHERE pid = ?
        "CREATE INDEX IF NOT EXISTS idx_agents_pid
         ON agents(pid)",
        // Generated column + composite index for severity aggregation queries (mirrors m189 pattern)
        "ALTER TABLE events ADD COLUMN severity_val REAL
         GENERATED ALWAYS AS (CAST(json_extract(metadata, '$.severity') AS REAL)) VIRTUAL",
        "CREATE INDEX IF NOT EXISTS idx_events_severity
         ON events(event_type, severity_val) WHERE severity_val IS NOT NULL",
    ],
    detect: Some(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_tasks_pending_verification'",
    ),
};
