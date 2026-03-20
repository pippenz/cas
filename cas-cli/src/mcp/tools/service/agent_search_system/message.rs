use crate::mcp::tools::service::imports::*;

impl CasService {
    pub(in crate::mcp::tools::service) async fn message_send(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_prompt_queue_store;

        let target = req.target.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "target required (agent name, 'supervisor', or 'all_workers')",
            )
        })?;
        let message = req
            .message
            .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "message required"))?;
        let summary = req.summary.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "summary required (a short one-line description of the message)",
            )
        })?;

        let source = self
            .inner
            .get_agent_id()
            .unwrap_or_else(|_| "unknown".to_string());
        // When agent ID lookup fails but CAS_AGENT_NAME is set (factory mode),
        // resolve display_name from the env var so messages show the correct sender.
        let env_agent_name = std::env::var("CAS_AGENT_NAME").ok();
        let agent_from_store = {
            use crate::store::open_agent_store;
            open_agent_store(&self.inner.cas_root)
                .ok()
                .and_then(|store| store.get(&source).ok())
        };
        let role = std::env::var("CAS_AGENT_ROLE")
            .ok()
            .or_else(|| agent_from_store.as_ref().map(|a| a.role.to_string()))
            .unwrap_or_else(|| "primary".to_string());

        let resolve_supervisor_name = || -> Option<String> {
            if let Ok(name) = std::env::var("CAS_SUPERVISOR_NAME") {
                if !name.trim().is_empty() {
                    return Some(name);
                }
            }
            use crate::store::open_agent_store;
            use cas_types::{AgentRole, AgentStatus};
            open_agent_store(&self.inner.cas_root)
                .ok()
                .and_then(|store| store.list(None).ok())
                .and_then(|agents| {
                    agents
                        .into_iter()
                        .find(|a| {
                            a.role == AgentRole::Supervisor
                                && (a.status == AgentStatus::Active
                                    || a.status == AgentStatus::Idle)
                        })
                        .map(|a| a.name)
                })
        };

        let resolved_target = if role == "worker" {
            if target == "supervisor" {
                resolve_supervisor_name().ok_or_else(|| {
                    Self::error(ErrorCode::INVALID_REQUEST,
                        "Cannot resolve 'supervisor' - no CAS_SUPERVISOR_NAME and no active supervisor agent found.")
                })?
            } else if target == "all_workers" {
                return Err(Self::error(
                    ErrorCode::INVALID_REQUEST,
                    "Workers cannot broadcast to all_workers",
                ));
            } else {
                let supervisor_name = resolve_supervisor_name();
                if supervisor_name.as_deref() != Some(&target) {
                    return Err(Self::error(
                        ErrorCode::INVALID_REQUEST,
                        format!(
                            "Workers can only message their supervisor. Use target='supervisor' or '{}'",
                            supervisor_name.unwrap_or_else(|| "<supervisor>".to_string())
                        ),
                    ));
                }
                target
            }
        } else {
            target
        };

        if role != "worker" && (resolved_target == "owner" || resolved_target.starts_with("inbox:"))
        {
            use crate::store::{
                NotificationPriority, open_agent_store, open_supervisor_queue_store,
            };
            use cas_types::AgentRole;
            use rusqlite::Connection;
            use std::collections::HashSet;

            let display_name = open_agent_store(&self.inner.cas_root)
                .ok()
                .and_then(|store| store.get(&source).ok())
                .map(|agent| {
                    if agent.role == AgentRole::Supervisor {
                        "supervisor".to_string()
                    } else {
                        agent.name
                    }
                })
                .unwrap_or_else(|| source.clone());

            let inbox_id = if resolved_target == "owner" {
                "owner".to_string()
            } else {
                resolved_target
                    .strip_prefix("inbox:")
                    .unwrap_or("owner")
                    .to_string()
            };

            let engaged = (|| -> std::result::Result<bool, rusqlite::Error> {
                let agent_name = std::env::var("CAS_AGENT_NAME").unwrap_or_default();
                if agent_name.is_empty() {
                    return Ok(false);
                }

                let manager = crate::ui::factory::SessionManager::new();
                let sessions = manager
                    .list_sessions()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?;

                let session = sessions
                    .into_iter()
                    .find(|session| session.metadata.supervisor.name == agent_name);

                let Some(session) = session else {
                    return Ok(false);
                };

                let mut targets: HashSet<String> = HashSet::new();
                targets.insert(session.metadata.supervisor.name.clone());
                targets.insert("all_workers".to_string());
                for worker in &session.metadata.workers {
                    targets.insert(worker.name.clone());
                }

                if targets.is_empty() {
                    return Ok(false);
                }

                let db_path = self.inner.cas_root.join("cas.db");
                let conn = Connection::open(&db_path)?;

                let mut target_vec: Vec<String> = targets.into_iter().collect();
                target_vec.sort();
                let placeholders = std::iter::repeat_n("?", target_vec.len())
                    .collect::<Vec<_>>()
                    .join(", ");

                let sql = format!(
                    "SELECT 1 FROM prompt_queue WHERE source = ? AND target IN ({placeholders}) LIMIT 1"
                );
                let mut stmt = conn.prepare(&sql)?;

                let mut params: Vec<Box<dyn rusqlite::ToSql>> =
                    Vec::with_capacity(1 + target_vec.len());
                params.push(Box::new("openclaw".to_string()));
                for target in target_vec {
                    params.push(Box::new(target));
                }

                let mut rows = stmt.query(rusqlite::params_from_iter(
                    params.iter().map(|param| param.as_ref()),
                ))?;
                Ok(rows.next()?.is_some())
            })()
            .unwrap_or(false);

            if !engaged {
                return Err(Self::error(
                    ErrorCode::INVALID_REQUEST,
                    "External inbox is not engaged for this session yet. Owner must message this factory session first (via OpenClaw) before agents can reply to 'owner'.",
                ));
            }

            let queue = open_supervisor_queue_store(&self.inner.cas_root).map_err(|error| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to open supervisor queue: {error}"),
                )
            })?;

            let payload = serde_json::json!({
                "schema_version": 1,
                "type": "message",
                "from": display_name,
                "message": message,
            })
            .to_string();

            let notification_id = queue
                .notify(&inbox_id, "message", &payload, NotificationPriority::Normal)
                .map_err(|error| {
                    Self::error(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to queue external message: {error}"),
                    )
                })?;

            return Ok(Self::success(format!(
                "External message queued\n\nID: {}\nInbox: {}\nFrom: {} ({})\nMessage: {}",
                notification_id,
                inbox_id,
                display_name,
                role,
                truncate_str(&message, 100)
            )));
        }

        let queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open message queue: {error}"),
            )
        })?;

        let display_name = {
            use crate::store::open_agent_store;
            use cas_types::AgentRole;
            let agent_store = open_agent_store(&self.inner.cas_root).ok();
            agent_store
                .and_then(|store| store.get(&source).ok())
                .map(|agent| {
                    if agent.role == AgentRole::Supervisor {
                        "supervisor".to_string()
                    } else {
                        agent.name
                    }
                })
                .or_else(|| env_agent_name.clone())
                .unwrap_or_else(|| source.clone())
        };

        let factory_session = std::env::var("CAS_FACTORY_SESSION").ok();
        let priority = req.priority.as_deref().map(|p| match p {
            "critical" | "0" => cas_store::NotificationPriority::Critical,
            "high" | "1" => cas_store::NotificationPriority::High,
            _ => cas_store::NotificationPriority::Normal,
        });
        let message_id = queue
            .enqueue_full(
                &display_name,
                &resolved_target,
                &message,
                factory_session.as_deref(),
                Some(summary.as_str()),
                priority,
            )
            .map_err(|error| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue message: {error}"),
                )
            })?;

        // Notify daemon that prompt queue has new data (best-effort)
        if let Err(e) = cas_factory::notify_daemon(&self.inner.cas_root) {
            tracing::debug!(
                "Prompt queue notification failed (daemon may not be running): {}",
                e
            );
        }

        Ok(Self::success(format!(
            "Message queued\n\nID: {}\nFrom: {} ({})\nTo: {}\nMessage: {}",
            message_id,
            display_name,
            role,
            resolved_target,
            truncate_str(&message, 100)
        )))
    }
}
