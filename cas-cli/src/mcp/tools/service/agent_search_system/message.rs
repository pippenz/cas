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
        let message = req.message.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "message required — full message body goes in `message`. \
                 Example: mcp__cas__coordination action=message target=supervisor \
                 summary=\"task blocked\" message=\"cas-abc1 needs ...\"",
            )
        })?;
        let summary = req.summary.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "summary required — a short one-line preview shown in the UI. \
                 Example: summary=\"task blocked on verification\" (required alongside `message`).",
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

        // cas-6913: "Message queued" reads as delivery confirmation, but a
        // message addressed to a not-yet-registered worker name (the common
        // spawn-then-immediately-assign sequence) sits in the queue until
        // that name shows up in the agent store — the supervisor has no
        // signal this happened. Check registration state up front so the
        // response can say so honestly. `all_workers` is a broadcast, not a
        // single-target claim, so it's always reported as delivered framing.
        let target_is_registered = resolved_target == "all_workers"
            || resolved_target == "supervisor"
            || {
                use crate::store::open_agent_store;
                open_agent_store(&self.inner.cas_root)
                    .ok()
                    .and_then(|store| store.list(None).ok())
                    .map(|agents| {
                        agents
                            .iter()
                            .any(|a| a.name.eq_ignore_ascii_case(&resolved_target))
                    })
                    .unwrap_or(false)
            };

        let factory_session = std::env::var("CAS_FACTORY_SESSION").ok();
        let urgent = req.urgent.unwrap_or(false);
        // Urgent messages break the target's in-flight turn, so they must jump
        // the queue ahead of any backlog: force Critical priority when urgent
        // and no explicit priority was given.
        let priority = req.priority.as_deref().map(|p| match p {
            "critical" | "0" => cas_store::NotificationPriority::Critical,
            "high" | "1" => cas_store::NotificationPriority::High,
            _ => cas_store::NotificationPriority::Normal,
        });
        let priority = if urgent && priority.is_none() {
            Some(cas_store::NotificationPriority::Critical)
        } else {
            priority
        };

        // cas-b269 review: persist halt_task_work BEFORE enqueue so delivery
        // cannot race ahead of the durable stop flag. Only authorized
        // supervisor/director sources may set halt; all_workers expands to
        // every worker; never halt the supervisor. Fail closed if halt
        // cannot be persisted for an authorized urgent stop.
        {
            use crate::mcp::tools::core::task::lifecycle::stale_close_guard::{
                halt_targets_for_urgent, may_source_set_halt, should_persist_urgent_halt,
                HALT_TASK_WORK_META,
            };
            use crate::store::open_agent_store;
            use cas_types::AgentRole;

            if let Ok(agent_store) = open_agent_store(&self.inner.cas_root) {
                let agents = agent_store.list(None).unwrap_or_default();
                let worker_names: Vec<String> = agents
                    .iter()
                    .filter(|a| a.role == AgentRole::Worker)
                    .map(|a| a.name.clone())
                    .collect();

                if should_persist_urgent_halt(
                    urgent,
                    &display_name,
                    &role,
                    &resolved_target,
                    &worker_names,
                ) {
                    let targets = halt_targets_for_urgent(&resolved_target, &worker_names);
                    for target_name in &targets {
                        let Some(mut agent) = agents
                            .iter()
                            .find(|a| a.name.eq_ignore_ascii_case(target_name))
                            .cloned()
                        else {
                            continue;
                        };
                        // Never halt a supervisor agent even if misnamed in the list.
                        if agent.role == AgentRole::Supervisor {
                            continue;
                        }
                        agent
                            .metadata
                            .insert(HALT_TASK_WORK_META.to_string(), "1".to_string());
                        agent_store.update(&agent).map_err(|e| {
                            Self::error(
                                ErrorCode::INTERNAL_ERROR,
                                format!(
                                    "Failed to persist halt_task_work for {target_name} \
                                     before urgent enqueue (cas-b269): {e}"
                                ),
                            )
                        })?;
                    }
                } else if urgent
                    && !may_source_set_halt(&display_name, &role)
                    && resolved_target.eq_ignore_ascii_case("supervisor")
                {
                    // Worker→supervisor urgent: explicit no-op on halt (policy).
                    tracing::debug!(
                        source = %display_name,
                        "cas-b269: ignoring halt for unauthorized source or supervisor target"
                    );
                }
            }
        }

        // cas-f9e8 telemetry: measure the wall-clock spent inside the DB
        // insert and log it alongside the caller-visible message id, so a
        // future investigator can bisect whether stalls live in send-side
        // persistence, daemon wake, daemon poll, or downstream inject. Logged
        // at debug so normal sessions stay quiet; enable via
        // `RUST_LOG=cas::coordination=debug`.
        let enqueue_started = std::time::Instant::now();
        let message_id = queue
            .enqueue_urgent(
                &display_name,
                &resolved_target,
                &message,
                factory_session.as_deref(),
                Some(summary.as_str()),
                priority,
                urgent,
            )
            .map_err(|error| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue message: {error}"),
                )
            })?;

        let persist_latency_ms = enqueue_started.elapsed().as_secs_f64() * 1000.0;
        tracing::debug!(
            target: "cas::coordination",
            stage = "enqueue",
            channel = "prompt_queue",
            message_id,
            source = %display_name,
            target_agent = %resolved_target,
            priority = ?priority,
            persist_ms = persist_latency_ms,
            "prompt_queue message enqueued"
        );

        // Notify daemon that prompt queue has new data (best-effort)
        let notify_started = std::time::Instant::now();
        let notify_outcome = cas_factory::notify_daemon(&self.inner.cas_root);
        let notify_latency_ms = notify_started.elapsed().as_secs_f64() * 1000.0;
        match notify_outcome {
            Ok(()) => {
                tracing::debug!(
                    target: "cas::coordination",
                    stage = "notify",
                    channel = "prompt_queue",
                    message_id,
                    notify_ms = notify_latency_ms,
                    "daemon wakeup signal sent"
                );
            }
            Err(ref e) => {
                // Kept as debug because this is expected when the daemon is
                // not running (e.g. `cas serve` standalone sessions).
                tracing::debug!(
                    target: "cas::coordination",
                    stage = "notify",
                    channel = "prompt_queue",
                    message_id,
                    notify_ms = notify_latency_ms,
                    error = %e,
                    "daemon wakeup signal failed (daemon may not be running)"
                );
            }
        }

        // cas-6913: honest delivery-status line. Urgent takes priority in the
        // wording since it describes the delivery MECHANISM (interrupt) —
        // but an urgent message to an unregistered target still can't
        // interrupt a turn that doesn't exist yet, so the registration
        // caveat wins even for urgent sends.
        let delivery_status = if !target_is_registered {
            "Delivery: queued — target not yet registered, will deliver on registration\n"
        } else if urgent {
            "Delivery: interrupt-and-redirect (breaks the target's in-flight turn, then injects)\n"
        } else {
            "Delivery: queued for next poll (target is registered)\n"
        };

        Ok(Self::success(format!(
            "{} queued\n\nID: {}\nFrom: {} ({})\nTo: {}\n{}Message: {}",
            if urgent { "URGENT message" } else { "Message" },
            message_id,
            display_name,
            role,
            resolved_target,
            delivery_status,
            truncate_str(&message, 100)
        )))
    }

    pub(in crate::mcp::tools::service) async fn message_ack(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_prompt_queue_store;

        let notification_id = req.notification_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "notification_id required for message_ack (the prompt queue message ID)",
            )
        })?;

        let queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open prompt queue: {error}"),
            )
        })?;

        queue.ack(notification_id).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to acknowledge message: {error}"),
            )
        })?;

        Ok(Self::success(format!(
            "Message {notification_id} acknowledged (delivery confirmed)"
        )))
    }

    pub(in crate::mcp::tools::service) async fn message_status_query(
        &self,
        req: AgentRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_prompt_queue_store;

        let notification_id = req.notification_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "notification_id required for message_status (the prompt queue message ID)",
            )
        })?;

        let queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open prompt queue: {error}"),
            )
        })?;

        let status = queue.message_status(notification_id).map_err(|error| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to query message status: {error}"),
            )
        })?;

        match status {
            Some(s) => Ok(Self::success(format!(
                "Message {notification_id} status: {s}"
            ))),
            None => Ok(Self::success(format!(
                "Message {notification_id} not found"
            ))),
        }
    }
}
