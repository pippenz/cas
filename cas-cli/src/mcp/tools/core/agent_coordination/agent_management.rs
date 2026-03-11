use crate::mcp::tools::core::imports::*;

impl CasCore {
    fn parse_csv_env(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect()
    }

    pub async fn cas_agent_whoami(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        let agent = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Agent not found: {e}")),
            data: None,
        })?;

        let leases = store.list_agent_leases(&req.id).unwrap_or_default();

        let mut output = format!(
            "Agent: {}\n\
             Name: {}\n\
             Type: {}\n\
             Status: {}\n\
             Active Tasks: {}\n\
             Registered: {}\n\
             Last Heartbeat: {}",
            agent.id,
            agent.name,
            agent.agent_type,
            agent.status,
            agent.active_tasks,
            agent.registered_at.format("%Y-%m-%d %H:%M:%S"),
            agent.last_heartbeat.format("%Y-%m-%d %H:%M:%S"),
        );

        if !leases.is_empty() {
            output.push_str("\n\nClaimed Tasks:\n");
            for lease in leases {
                output.push_str(&format!(
                    "  - {} (expires in {}s)\n",
                    lease.task_id,
                    lease.remaining_secs()
                ));
            }
        }

        Ok(Self::success(output))
    }

    /// Send heartbeat to keep agent alive
    pub async fn cas_agent_heartbeat(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        store.heartbeat(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Failed to update heartbeat: {e}")),
            data: None,
        })?;

        // Also renew any active worktree leases for this agent
        let worktree_leases = store
            .list_agent_worktree_leases(&req.id)
            .unwrap_or_default();
        let mut renewed_count = 0;
        for lease in worktree_leases {
            if store
                .renew_worktree_lease(&lease.worktree_id, &req.id, 600)
                .is_ok()
            {
                renewed_count += 1;
            }
        }

        let msg = if renewed_count > 0 {
            format!(
                "Heartbeat updated for agent: {} (renewed {} worktree lease(s))",
                req.id, renewed_count
            )
        } else {
            format!("Heartbeat updated for agent: {}", req.id)
        };
        Ok(Self::success(msg))
    }

    /// List all registered agents
    pub async fn cas_agent_list(
        &self,
        Parameters(req): Parameters<LimitRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        let all_agents = store.list(None).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to list agents: {e}")),
            data: None,
        })?;

        if all_agents.is_empty() {
            return Ok(Self::success("No agents registered"));
        }

        // In factory supervisor sessions, prefer the canonical per-session roster over global
        // historical agents to avoid stale cross-session confusion.
        let is_supervisor = std::env::var("CAS_AGENT_ROLE")
            .map(|r| r.eq_ignore_ascii_case("supervisor"))
            .unwrap_or(false);
        let canonical_workers = std::env::var("CAS_FACTORY_WORKER_NAMES")
            .ok()
            .map(|value| Self::parse_csv_env(&value))
            .unwrap_or_default();
        let supervisor_name = std::env::var("CAS_AGENT_NAME").ok();
        let supervisor_session_id = std::env::var("CAS_SESSION_ID").ok();

        let mut using_session_scope = false;
        let agents = if is_supervisor && !canonical_workers.is_empty() {
            let worker_set: std::collections::BTreeSet<&str> =
                canonical_workers.iter().map(String::as_str).collect();
            let session_agents: Vec<_> = all_agents
                .iter()
                .filter(|agent| {
                    worker_set.contains(agent.name.as_str())
                        || supervisor_name
                            .as_ref()
                            .map(|name| name == &agent.name)
                            .unwrap_or(false)
                        || supervisor_session_id
                            .as_ref()
                            .map(|id| id == &agent.id)
                            .unwrap_or(false)
                })
                .cloned()
                .collect();
            if session_agents.is_empty() {
                all_agents.clone()
            } else {
                using_session_scope = true;
                session_agents
            }
        } else {
            all_agents.clone()
        };

        let limit = req.limit.unwrap_or(50);
        let mut output = if using_session_scope {
            format!(
                "Agents (session scope: {} shown, {} total in store):\n\n",
                agents.len(),
                all_agents.len()
            )
        } else {
            format!("Agents ({} total):\n\n", agents.len())
        };

        for agent in agents.iter().take(limit) {
            output.push_str(&format!(
                "[{}] {} - {} ({}/{}) - {} tasks\n",
                agent.status,
                agent.id,
                agent.name,
                agent.agent_type,
                agent.role,
                agent.active_tasks
            ));
        }

        Ok(Self::success(output))
    }

    /// Unregister an agent
    pub async fn cas_agent_unregister(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        // Verify agent exists
        let _agent = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Agent not found: {e}")),
            data: None,
        })?;

        // Unregister (releases all leases and removes agent)
        store.unregister(&req.id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to unregister agent: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!("Unregistered agent: {}", req.id)))
    }

    /// Mark stale agents as dead and reclaim their leases
    pub async fn cas_agent_cleanup(
        &self,
        Parameters(req): Parameters<AgentCleanupRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        let threshold_secs = req.stale_threshold_secs.unwrap_or(120);

        // Find stale agents
        let stale_agents = store.list_stale(threshold_secs).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to list stale agents: {e}")),
            data: None,
        })?;

        let mut marked_stale = 0;
        for agent in &stale_agents {
            if store.mark_stale(&agent.id).is_ok() {
                marked_stale += 1;
            }
        }

        // Reclaim expired leases
        let reclaimed = store.reclaim_expired_leases().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to reclaim leases: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Cleanup complete:\n\
             - Stale agents marked: {marked_stale}\n\
             - Expired leases reclaimed: {reclaimed}"
        )))
    }

    /// Get lease history for a task
    pub async fn cas_lease_history(
        &self,
        Parameters(req): Parameters<LeaseHistoryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_agent_store()?;

        let history = store
            .get_lease_history(&req.task_id, req.limit)
            .map_err(|e| McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to get lease history: {e}")),
                data: None,
            })?;

        if history.is_empty() {
            return Ok(Self::success(format!(
                "No lease history for task: {}",
                req.task_id
            )));
        }

        let mut output = format!(
            "Lease history for {} ({} events):\n\n",
            req.task_id,
            history.len()
        );

        for event in &history {
            output.push_str(&format!(
                "[{}] {} by {} (epoch {}){}\n",
                event.timestamp.format("%Y-%m-%d %H:%M:%S"),
                event.event_type,
                event.agent_id,
                event.epoch,
                event
                    .previous_agent_id
                    .as_ref()
                    .map(|p| format!(" (from {p})"))
                    .unwrap_or_default()
            ));
        }

        Ok(Self::success(output))
    }
}
