use crate::Result;
use crate::agent_store::{AGENT_SCHEMA, SqliteAgentStore};
use crate::error::StoreError;
use crate::event_store::record_event_with_conn;
use crate::recording_store::capture_agent_event;
use crate::shared_db::ImmediateTx;
use cas_types::{Agent, AgentStatus, Event, EventEntityType, EventType, RecordingEventType};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};

impl SqliteAgentStore {
    pub(crate) fn agent_init(&self) -> Result<()> {
        let conn = self.lock_conn()?;
        conn.execute_batch(AGENT_SCHEMA)?;
        Ok(())
    }
    pub(crate) fn agent_register(&self, agent: &Agent) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.lock_conn()?;
            let metadata_json =
                serde_json::to_string(&agent.metadata).unwrap_or_else(|_| "{}".to_string());
            let existed = conn
                .query_row(
                    "SELECT 1 FROM agents WHERE id = ?1",
                    params![agent.id],
                    |_| Ok(()),
                )
                .optional()?
                .is_some();

            // Use INSERT ... ON CONFLICT for idempotent registration.
            // This allows SessionStart hook and MCP to both register without conflict.
            // On conflict (re-registration), we preserve startup_confirmed so that a
            // live agent that re-registers doesn't get falsely detected as failed-startup.
            conn.execute(
            "INSERT INTO agents (id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata, startup_confirmed)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, 0)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                agent_type = excluded.agent_type,
                role = excluded.role,
                status = excluded.status,
                pid = excluded.pid,
                ppid = excluded.ppid,
                cc_session_id = excluded.cc_session_id,
                parent_id = excluded.parent_id,
                machine_id = excluded.machine_id,
                last_heartbeat = excluded.last_heartbeat,
                active_tasks = excluded.active_tasks,
                metadata = excluded.metadata",
            params![
                agent.id,
                agent.name,
                agent.agent_type.to_string(),
                agent.role.to_string(),
                agent.status.to_string(),
                agent.pid,
                agent.ppid,
                agent.cc_session_id,
                agent.parent_id,
                agent.machine_id,
                agent.registered_at.to_rfc3339(),
                agent.last_heartbeat.to_rfc3339(),
                agent.active_tasks,
                metadata_json,
            ],
        )?;

            if !existed {
                // Record event for sidecar activity feed
                let event = Event::new(
                    EventType::AgentRegistered,
                    EventEntityType::Agent,
                    &agent.id,
                    format!("Agent registered: {}", agent.name),
                )
                .with_session(agent.cc_session_id.as_deref().unwrap_or(&agent.id));
                let _ = record_event_with_conn(&conn, &event); // Best-effort, don't fail on event recording

                // Capture event for recording playback
                let _ =
                    capture_agent_event(&conn, RecordingEventType::AgentJoined, &agent.id, None);
            }

            Ok(())
        }) // with_write_retry
    }
    pub(crate) fn agent_get(&self, id: &str) -> Result<Agent> {
        let conn = self.lock_conn()?;
        conn.query_row(
            "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata
             FROM agents WHERE id = ?",
            params![id],
            Self::agent_from_row,
        )
        .optional()?
        .ok_or_else(|| StoreError::NotFound(format!("Agent not found: {id}")))
    }
    pub(crate) fn agent_update(&self, agent: &Agent) -> Result<()> {
        let conn = self.lock_conn()?;
        let metadata_json =
            serde_json::to_string(&agent.metadata).unwrap_or_else(|_| "{}".to_string());

        let rows = conn.execute(
            "UPDATE agents SET name = ?1, agent_type = ?2, role = ?3, status = ?4, pid = ?5,
             ppid = ?6, cc_session_id = ?7, parent_id = ?8, machine_id = ?9, last_heartbeat = ?10,
             active_tasks = ?11, metadata = ?12
             WHERE id = ?13",
            params![
                agent.name,
                agent.agent_type.to_string(),
                agent.role.to_string(),
                agent.status.to_string(),
                agent.pid,
                agent.ppid,
                agent.cc_session_id,
                agent.parent_id,
                agent.machine_id,
                agent.last_heartbeat.to_rfc3339(),
                agent.active_tasks,
                metadata_json,
                agent.id,
            ],
        )?;

        if rows == 0 {
            return Err(StoreError::NotFound(format!(
                "Agent not found: {}",
                agent.id
            )));
        }
        Ok(())
    }
    pub(crate) fn agent_unregister(&self, id: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        let tx = ImmediateTx::new(&conn)?;

        // Get agent name before deleting (for event summary)
        let agent_name: Option<String> = tx
            .query_row("SELECT name FROM agents WHERE id = ?", params![id], |row| {
                row.get(0)
            })
            .optional()?;

        // Release all leases first (due to foreign key)
        tx.execute(
            "UPDATE task_leases SET status = 'released' WHERE agent_id = ?",
            params![id],
        )?;

        let rows = tx.execute("DELETE FROM agents WHERE id = ?", params![id])?;
        if rows == 0 {
            return Err(StoreError::NotFound(format!("Agent not found: {id}")));
        }

        // Record event for sidecar activity feed (use name if available, else id)
        let display_name = agent_name
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| id.to_string());
        let event = Event::new(
            EventType::AgentShutdown,
            EventEntityType::Agent,
            id,
            format!("Agent unregistered: {display_name}"),
        );
        let _ = record_event_with_conn(&tx, &event);

        // Capture event for recording playback
        let _ = capture_agent_event(&tx, RecordingEventType::AgentLeft, id, None);

        tx.commit()?;
        Ok(())
    }
    pub(crate) fn agent_list(&self, status: Option<AgentStatus>) -> Result<Vec<Agent>> {
        let conn = self.lock_conn()?;

        let (sql, params): (&str, Vec<String>) = match status {
            Some(s) => (
                "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
                 machine_id, registered_at, last_heartbeat, active_tasks, metadata
                 FROM agents WHERE status = ? ORDER BY registered_at DESC",
                vec![s.to_string()],
            ),
            None => (
                "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
                 machine_id, registered_at, last_heartbeat, active_tasks, metadata
                 FROM agents ORDER BY registered_at DESC",
                vec![],
            ),
        };

        let mut stmt = conn.prepare_cached(sql)?;
        let agents = if params.is_empty() {
            stmt.query_map([], Self::agent_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(params![params[0]], Self::agent_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };

        Ok(agents)
    }
    pub(crate) fn agent_list_stale(&self, timeout_secs: i64) -> Result<Vec<Agent>> {
        let conn = self.lock_conn()?;
        let cutoff = (Utc::now() - chrono::Duration::seconds(timeout_secs)).to_rfc3339();

        let mut stmt = conn.prepare_cached(
            "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata
             FROM agents
             WHERE status IN ('active', 'idle') AND last_heartbeat < ?
             ORDER BY last_heartbeat ASC",
        )?;

        let agents = stmt
            .query_map(params![cutoff], Self::agent_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(agents)
    }
    pub(crate) fn agent_heartbeat(&self, id: &str) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.lock_conn()?;
            let now = Utc::now().to_rfc3339();

            // Only heartbeat agents in live states (active/idle). Agents that have been
            // explicitly shut down or marked stale should not be revived by a heartbeat —
            // their daemon may still be running briefly after the process was killed.
            // Also confirm startup on first heartbeat (startup_confirmed = 1).
            let rows = conn.execute(
            "UPDATE agents SET last_heartbeat = ?, status = 'active', startup_confirmed = 1 WHERE id = ? AND status IN ('active', 'idle')",
            params![now, id],
        )?;

            if rows == 0 {
                // Use a single query to check existence and get status,
                // providing a specific error message without a second round-trip.
                // We already know the UPDATE didn't match, so the agent either
                // doesn't exist or is in a non-live state.
                let status: Option<String> = conn
                    .query_row(
                        "SELECT status FROM agents WHERE id = ?",
                        params![id],
                        |row| row.get(0),
                    )
                    .optional()?;
                match status {
                    Some(s) if s == "shutdown" || s == "stale" => {
                        return Err(StoreError::Other(format!(
                            "Agent {id} is {s} — heartbeat ignored"
                        )));
                    }
                    Some(s) => {
                        return Err(StoreError::Other(format!(
                            "Agent {id} has unexpected status '{s}'"
                        )));
                    }
                    None => {
                        return Err(StoreError::NotFound(format!("Agent not found: {id}")));
                    }
                }
            }
            Ok(())
        }) // with_write_retry
    }
    pub(crate) fn agent_mark_stale(&self, id: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        let tx = ImmediateTx::new(&conn)?;

        // Get all active leases for this agent before revoking
        let mut stmt = tx.prepare_cached(
            "SELECT task_id, epoch FROM task_leases WHERE agent_id = ? AND status = 'active'",
        )?;
        let leases_to_revoke: Vec<(String, i64)> = stmt
            .query_map(params![id], |row| {
                Ok((row.get(0)?, row.get::<_, i64>(1).unwrap_or(1)))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(stmt);

        // Mark agent as stale (was: dead)
        tx.execute(
            "UPDATE agents SET status = 'stale' WHERE id = ?",
            params![id],
        )?;

        // Revoke all active task leases
        tx.execute(
            "UPDATE task_leases SET status = 'revoked' WHERE agent_id = ? AND status = 'active'",
            params![id],
        )?;

        // Revoke all active worktree leases
        tx.execute(
            "UPDATE worktree_leases SET status = 'revoked' WHERE agent_id = ? AND status = 'active'",
            params![id],
        )?;

        // Log revoked events for each lease
        for (task_id, epoch) in &leases_to_revoke {
            Self::log_lease_event(
                &tx,
                task_id,
                id,
                "revoked",
                *epoch as u64,
                Some(r#"{"reason":"agent_stale"}"#),
                None,
            )?;
        }

        tx.commit()?;
        Ok(())
    }
    pub(crate) fn agent_revive(&self, id: &str) -> Result<()> {
        let conn = self.lock_conn()?;
        let now = Utc::now().to_rfc3339();

        // Revive agent: set status to active, update heartbeat, and confirm startup.
        // Only works if agent exists and is in stale/shutdown/dead state.
        // Setting startup_confirmed = 1 prevents the agent from being immediately
        // re-detected as failed-startup after revival.
        let rows = conn.execute(
            "UPDATE agents SET status = 'active', last_heartbeat = ?, startup_confirmed = 1
             WHERE id = ? AND status IN ('dead', 'shutdown', 'stale')",
            params![now, id],
        )?;

        if rows == 0 {
            return Err(StoreError::NotFound(format!(
                "Agent not found or already active: {id}"
            )));
        }

        Ok(())
    }
    pub(crate) fn agent_list_failed_startup(&self, timeout_secs: i64) -> Result<Vec<Agent>> {
        let conn = self.lock_conn()?;
        let cutoff = (Utc::now() - chrono::Duration::seconds(timeout_secs)).to_rfc3339();

        let mut stmt = conn.prepare_cached(
            "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata
             FROM agents
             WHERE status IN ('active', 'idle') AND startup_confirmed = 0 AND registered_at < ?
             ORDER BY registered_at ASC",
        )?;

        let agents = stmt
            .query_map(params![cutoff], Self::agent_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(agents)
    }

    pub(crate) fn agent_get_by_cc_pid(&self, cc_pid: u32) -> Result<Option<Agent>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata
             FROM agents WHERE ppid = ? AND status IN ('active', 'idle', 'stale', 'dead', 'shutdown')
             ORDER BY last_heartbeat DESC LIMIT 1",
        )?;

        stmt.query_row(params![cc_pid], Self::agent_from_row)
            .optional()
            .map_err(Into::into)
    }

    pub(crate) fn agent_get_by_pid(&self, pid: u32) -> Result<Option<Agent>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare_cached(
            "SELECT id, name, agent_type, role, status, pid, ppid, cc_session_id, parent_id,
             machine_id, registered_at, last_heartbeat, active_tasks, metadata
             FROM agents WHERE pid = ? AND status IN ('active', 'idle', 'stale', 'dead', 'shutdown')
             ORDER BY last_heartbeat DESC LIMIT 1",
        )?;

        stmt.query_row(params![pid], Self::agent_from_row)
            .optional()
            .map_err(Into::into)
    }
}
