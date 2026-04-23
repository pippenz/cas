use crate::mcp::tools::service::imports::*;

/// Heartbeat age at which a worker is considered **stale** and becomes
/// eligible for the opportunistic prune in `factory_worker_status`.
///
/// A stale worker is dropped from the Active listing on the next status
/// call (see the `list_stale` + `mark_stale` loop). If the prune succeeds
/// — the overwhelmingly common path — the worker never reaches the
/// render-time liveness-label branch at all.
///
/// Bumped from 120s to 30s by cas-2749 so a crashed CC client is
/// detected within roughly one supervisor status poll. The 30s number is
/// load-bearing: callers in tests assert the exact value via
/// [`worker_stale_secs_is_pinned_at_30`] (cas-8240 AC anchor) so a drift
/// fix in one place that forgets to update the other cannot silently
/// regress the UX.
pub(crate) const WORKER_STALE_SECS: i64 = 30;

/// Heartbeat age at which a worker is escalated to **dead** in the
/// supervisor-facing render: hard `[DEAD]` label + transcript-path
/// surfacing so the supervisor can salvage the last in-flight tool call.
///
/// Two-band model (cas-8240): `WORKER_STALE_SECS` (30s) drives the
/// opportunistic prune and a lighter-weight `[stale]` indicator on any
/// worker that slipped past the prune (e.g. `mark_stale` hit a DB lock).
/// `WORKER_DEAD_SECS` (75s) gates the more expensive `[DEAD]` + transcript
/// emission so tokio scheduler jitter or a missed 30s daemon tick cannot
/// produce false-positive DEAD labels that train supervisors to distrust
/// the signal. Picked at 2.5× the stale threshold: gives the daemon one
/// full heartbeat interval of grace past the prune window before the
/// render escalates, which in practice means a worker has to have
/// missed at least two consecutive heartbeats before we surface it as
/// dead.
pub(crate) const WORKER_DEAD_SECS: i64 = 75;

impl CasService {
    pub(super) async fn factory_spawn_workers(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_spawn_queue_store, open_task_store};
        use cas_types::{TaskStatus, TaskType};

        // Check that there's an active EPIC before spawning workers
        let task_store = open_task_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open task store: {e}"),
            )
        })?;

        let open_epics: Vec<_> = task_store
            .list(None)
            .map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to list tasks: {e}"),
                )
            })?
            .into_iter()
            .filter(|t| t.task_type == TaskType::Epic && t.status != TaskStatus::Closed)
            .collect();

        if open_epics.is_empty() {
            return Err(Self::error(
                ErrorCode::INVALID_REQUEST,
                "No active EPIC found. Before spawning workers, create or assign an EPIC:\n\
                 1. Create EPIC: mcp__cas__task action=create task_type=epic title=\"...\" description=\"...\"\n\
                 2. Or assign existing EPIC: mcp__cas__task action=start id=<epic-id>\n\
                 3. Optionally gather requirements using the epic-spec skill\n\
                 4. Break into tasks using the epic-breakdown skill\n\
                 5. Then spawn workers to work on the tasks",
            ));
        }

        let queue = open_spawn_queue_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spawn queue: {e}"),
            )
        })?;

        let count = req.count.unwrap_or(1);
        let isolate = req.isolate.unwrap_or(false);
        let worker_names: Vec<String> = req
            .worker_names
            .map(|names| {
                names
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let request_id = queue
            .enqueue_spawn(count, &worker_names, isolate)
            .map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue spawn request: {e}"),
                )
            })?;

        let msg = if worker_names.is_empty() {
            format!("Queued spawn request for {count} worker(s) (request ID: {request_id})")
        } else {
            format!(
                "Queued spawn request for worker(s): {} (request ID: {})",
                worker_names.join(", "),
                request_id
            )
        };

        Ok(Self::success(msg))
    }

    pub(super) async fn factory_shutdown_workers(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_spawn_queue_store};
        use cas_types::{AgentRole, AgentStatus};

        let mut worker_names: Vec<String> = req
            .worker_names
            .map(|names| {
                names
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // When supervisor has no specific worker names requested, scope to owned workers
        // so a supervisor cannot shut down another supervisor's workers.
        if worker_names.is_empty() {
            if let Some(owned) = supervisor_owned_workers() {
                worker_names = owned.into_iter().collect();
            }
        }

        // Validate workers exist before queuing (synchronous validation)
        if !worker_names.is_empty() {
            let agent_store = open_agent_store(&self.inner.cas_root).map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to open agent store: {e}"),
                )
            })?;

            // Include both active and stale workers — stale workers are often
            // exactly what supervisors want to shut down.
            let mut known_agents = agent_store.list(Some(AgentStatus::Active)).map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to list agents: {e}"),
                )
            })?;
            if let Ok(stale) = agent_store.list(Some(AgentStatus::Stale)) {
                known_agents.extend(stale);
            }

            // Get worker names, scoped to this supervisor's workers when applicable
            let owned = supervisor_owned_workers();
            let known_workers: std::collections::HashSet<String> = known_agents
                .iter()
                .filter(|a| {
                    a.role == AgentRole::Worker
                        && owned.as_ref().is_none_or(|set| set.contains(&a.name))
                })
                .map(|a| a.name.clone())
                .collect();

            // Check each requested worker exists
            let mut not_found = Vec::new();
            for name in &worker_names {
                if !known_workers.contains(name) {
                    not_found.push(name.clone());
                }
            }

            if !not_found.is_empty() {
                return Err(Self::error(
                    ErrorCode::INVALID_PARAMS,
                    format!(
                        "Worker(s) not found: {}. Known workers: {}",
                        not_found.join(", "),
                        if known_workers.is_empty() {
                            "(none)".to_string()
                        } else {
                            known_workers.into_iter().collect::<Vec<_>>().join(", ")
                        }
                    ),
                ));
            }
        }

        // Validation passed, queue the shutdown
        let queue = open_spawn_queue_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spawn queue: {e}"),
            )
        })?;

        let count = req.count;
        let force = req.force.unwrap_or(false);
        let request_id = queue
            .enqueue_shutdown(count, &worker_names, force)
            .map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue shutdown request: {e}"),
                )
            })?;

        let msg = if !worker_names.is_empty() {
            format!(
                "Queued shutdown request for worker(s): {} (request ID: {})",
                worker_names.join(", "),
                request_id
            )
        } else if let Some(c) = count {
            if c == 0 {
                format!("Queued shutdown request for ALL workers (request ID: {request_id})")
            } else {
                format!("Queued shutdown request for {c} worker(s) (request ID: {request_id})")
            }
        } else {
            format!("Queued shutdown request for ALL workers (request ID: {request_id})")
        };

        Ok(Self::success(msg))
    }

    pub(super) async fn factory_worker_status(
        &self,
        _req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_agent_store;
        use cas_types::{AgentRole, AgentStatus};

        let store = open_agent_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open agent store: {e}"),
            )
        })?;

        // Opportunistically prune stale agents so status output stays actionable.
        // Worker threshold tightened from 120s → 30s per cas-2749 so a dead CC
        // client is detected within one supervisor poll. Paired with the
        // daemon-side PID liveness gate in mcp::daemon::send_agent_heartbeat,
        // a crashed worker stops heartbeating within the 30s daemon tick and
        // transitions to "dead" in the next status call. Supervisors/directors
        // are long-lived and less chatty and are filtered out of the prune by
        // the role check below; they remain visible until their own
        // daemon-level cleanup eventually removes them.
        //
        // See the module-level `WORKER_STALE_SECS` and `WORKER_DEAD_SECS`
        // constants (cas-8240) for the two-band model that separates the
        // prune + `[stale]` indicator (30s) from the hard `[DEAD]` + transcript
        // surface (75s).
        let worker_stale_threshold_secs: i64 = WORKER_STALE_SECS;
        let mut stale_pruned = 0usize;
        if let Ok(stale_agents) = store.list_stale(worker_stale_threshold_secs) {
            for agent in stale_agents {
                if agent.role == AgentRole::Supervisor || agent.role == AgentRole::Director {
                    continue;
                }
                if store.mark_stale(&agent.id).is_ok() {
                    stale_pruned += 1;
                }
            }
        }

        let agents = store.list(Some(AgentStatus::Active)).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to list agents: {e}"),
            )
        })?;

        if agents.is_empty() {
            return Ok(Self::success(
                "No active agents registered.\n\nNote: Factory TUI must be running for agents to be registered.",
            ));
        }

        let owned = supervisor_owned_workers();
        let mut output = String::from("Worker Status\n=============\n\n");

        let workers: Vec<_> = agents
            .iter()
            .filter(|a| {
                a.role == AgentRole::Worker
                    && owned.as_ref().is_none_or(|set| set.contains(&a.name))
            })
            .collect();
        let self_name = std::env::var("CAS_AGENT_NAME").ok();
        let supervisors: Vec<_> = agents
            .iter()
            .filter(|a| {
                (a.role == AgentRole::Supervisor || a.role == AgentRole::Director)
                    && if owned.is_some() {
                        // When scoped, only show this supervisor (not others)
                        self_name.as_ref() == Some(&a.name)
                    } else {
                        true
                    }
            })
            .collect();

        if !supervisors.is_empty() {
            output.push_str("Supervisors:\n");
            for agent in supervisors {
                let elapsed = (chrono::Utc::now() - agent.last_heartbeat).num_seconds();
                let since = format!("{elapsed}s ago");
                output.push_str(&format!("  • {} (heartbeat: {})\n", &agent.name, since));
            }
            output.push('\n');
        }

        if workers.is_empty() {
            output.push_str("Workers: None active\n");
        } else {
            output.push_str(&format!("Workers ({}):\n", workers.len()));
            for agent in workers {
                let elapsed = (chrono::Utc::now() - agent.last_heartbeat).num_seconds();
                let since = format!("{elapsed}s ago");
                // cas-8240 two-band model — see `liveness_label_for`.
                let liveness_label = liveness_label_for(elapsed);
                let clone_path = agent.metadata.get("clone_path").cloned();
                let clone_info = clone_path
                    .as_ref()
                    .map(|p| format!("\n    Clone: {p}"))
                    .unwrap_or_default();
                // Surface transcript path only for hard-dead workers so
                // supervisor can salvage whatever was in-flight when the
                // CC client died (cas-2749 AC: transcript-path-surfacing
                // on crash). The `[stale]` tier does NOT emit the
                // transcript path — a worker lagging past 30s under
                // scheduler jitter does not need its transcript surfaced
                // yet, and emitting it there would produce the
                // false-positive noise cas-8240 is fixing.
                let transcript_info = if elapsed >= WORKER_DEAD_SECS {
                    let transcript = clone_path
                        .as_deref()
                        .map(|p| derive_transcript_path(p, &agent.id))
                        .unwrap_or_else(|| {
                            format!(
                                "~/.claude/projects/<cwd>/{}.jsonl (clone path unknown)",
                                agent.id
                            )
                        });
                    format!("\n    Transcript: {transcript}")
                } else {
                    String::new()
                };
                output.push_str(&format!(
                    "  • {} (heartbeat: {}){}{}{}\n",
                    &agent.name, since, liveness_label, clone_info, transcript_info
                ));
            }
        }

        if stale_pruned > 0 {
            output.push_str(&format!(
                "\nFiltered stale agent record(s): {stale_pruned} (>{worker_stale_threshold_secs}s heartbeat age)\n"
            ));
        }

        Ok(Self::success(output))
    }

    pub(super) async fn factory_worker_activity(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use cas_store::{EventStore, SqliteEventStore};
        use cas_types::EventType;

        let event_store = SqliteEventStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open event store: {e}"),
            )
        })?;

        // Filter by worker name if specified, otherwise scope to this supervisor's workers
        let worker_filter = req.worker_names.as_ref();
        let owned = supervisor_owned_workers();

        // Get recent worker activity events
        let events = event_store.list_recent(50).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to list events: {e}"),
            )
        })?;

        // Filter to worker activity events
        let worker_events: Vec<_> = events
            .into_iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    EventType::WorkerSubagentSpawned
                        | EventType::WorkerSubagentCompleted
                        | EventType::WorkerFileEdited
                        | EventType::WorkerGitCommit
                        | EventType::WorkerVerificationBlocked
                        | EventType::VerificationStarted
                        | EventType::VerificationAdded
                )
            })
            .filter(|e| {
                let name_matches = |name: &str| {
                    e.session_id
                        .as_ref()
                        .map(|s| s.contains(name))
                        .unwrap_or(false)
                        || e.entity_id.contains(name)
                };
                if let Some(filter) = worker_filter {
                    name_matches(filter.as_str())
                } else if let Some(set) = &owned {
                    set.iter().any(|w| name_matches(w.as_str()))
                } else {
                    true
                }
            })
            .take(20)
            .collect();

        if worker_events.is_empty() {
            return Ok(Self::success(
                "No recent worker activity.\n\nWorker activity is tracked when workers edit files, run subagents, or commit code.",
            ));
        }

        let mut output = String::from("Worker Activity\n===============\n\n");
        for event in worker_events {
            let ago = format_relative_time(event.created_at);
            let session_short = event
                .session_id
                .as_ref()
                .map(|s| &s[..8.min(s.len())])
                .unwrap_or("unknown");
            output.push_str(&format!(
                "• {} - {} ({})\n",
                session_short, event.summary, ago
            ));
        }

        Ok(Self::success(output))
    }

    pub(super) async fn factory_clear_context(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_prompt_queue_store;

        let target = req.target.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "target required for clear_context",
            )
        })?;

        // Validate target is an owned worker when supervisor scoping applies
        if target != "all_workers" && target != "supervisor" {
            if let Some(owned) = supervisor_owned_workers() {
                if !owned.contains(&target) {
                    return Err(Self::error(
                        ErrorCode::INVALID_PARAMS,
                        format!(
                            "Worker '{}' not owned by this supervisor. Owned: {}",
                            target,
                            owned.into_iter().collect::<Vec<_>>().join(", ")
                        ),
                    ));
                }
            }
        }

        let queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open message queue: {e}"),
            )
        })?;

        // Use the MCP caller's agent ID as the source
        let source = self
            .inner
            .get_agent_id()
            .unwrap_or_else(|_| "unknown".to_string());

        // Enqueue /clear directly without XML wrapping - this is a raw command
        let factory_session = std::env::var("CAS_FACTORY_SESSION").ok();
        if let Some(ref session) = factory_session {
            queue
                .enqueue_with_session(&source, &target, "/clear", session)
                .map_err(|e| {
                    Self::error(
                        ErrorCode::INTERNAL_ERROR,
                        format!("Failed to queue clear command: {e}"),
                    )
                })?;
        } else {
            queue.enqueue(&source, &target, "/clear").map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to queue clear command: {e}"),
                )
            })?;
        }

        let msg = if target == "all_workers" {
            "Queued /clear for all workers".to_string()
        } else {
            format!("Queued /clear for {target}")
        };

        Ok(Self::success(msg))
    }

    pub(super) async fn factory_my_context(
        &self,
        _req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_task_store};
        use cas_types::AgentRole;

        // Get current agent's info
        let agent_id = self.inner.get_agent_id().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to get agent ID: {e}"),
            )
        })?;

        let agent_store = open_agent_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open agent store: {e}"),
            )
        })?;

        let agent = agent_store.get(&agent_id).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to get agent: {e}"),
            )
        })?;

        let mut output = String::from("My Factory Context\n==================\n\n");

        // Agent info
        let role_str = match agent.role {
            AgentRole::Worker => "Worker",
            AgentRole::Supervisor => "Supervisor",
            AgentRole::Director => "Director",
            AgentRole::Standard => "Standard Agent",
        };
        output.push_str(&format!("**Name**: {}\n", agent.name));
        output.push_str(&format!("**Role**: {role_str}\n"));
        output.push_str(&format!("**ID**: {}\n\n", agent.id));

        // Clone path (from environment)
        if let Ok(cwd) = std::env::var("CAS_CLONE_PATH") {
            output.push_str(&format!("**Clone Path**: {cwd}\n"));
        } else if let Ok(cwd) = std::env::current_dir() {
            output.push_str(&format!("**Working Directory**: {}\n", cwd.display()));
        }

        // Current task(s)
        let leases = agent_store.list_agent_leases(&agent_id).unwrap_or_default();
        if leases.is_empty() {
            output.push_str("\n**Current Task**: None (idle)\n");
        } else {
            output.push_str("\n**Claimed Tasks**:\n");
            if let Ok(task_store) = open_task_store(&self.inner.cas_root) {
                for lease in &leases {
                    if let Ok(task) = task_store.get(&lease.task_id) {
                        output.push_str(&format!("  - {} {}\n", task.id, task.title));
                    } else {
                        output.push_str(&format!("  - {}\n", lease.task_id));
                    }
                }
            } else {
                for lease in &leases {
                    output.push_str(&format!("  - {}\n", lease.task_id));
                }
            }
        }

        // Git branch info
        if let Ok(branch_output) = std::process::Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
        {
            if branch_output.status.success() {
                let branch = String::from_utf8_lossy(&branch_output.stdout)
                    .trim()
                    .to_string();
                output.push_str(&format!("\n**Git Branch**: {branch}\n"));
            }
        }

        Ok(Self::success(output))
    }

    pub(super) async fn factory_sync_all_workers(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_task_store};
        use cas_types::{AgentRole, AgentStatus, TaskStatus, TaskType};
        use std::path::Path;

        let store = open_agent_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open agent store: {e}"),
            )
        })?;

        let owned = supervisor_owned_workers();
        let mut workers: Vec<_> = store
            .list(Some(AgentStatus::Active))
            .map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to list agents: {e}"),
                )
            })?
            .into_iter()
            .filter(|a| {
                a.role == AgentRole::Worker
                    && owned.as_ref().is_none_or(|set| set.contains(&a.name))
            })
            .collect();

        if workers.is_empty() {
            return Ok(Self::success("No active workers found."));
        }

        if let Some(filter) = req.worker_names.as_ref() {
            let names: std::collections::HashSet<String> = filter
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            workers.retain(|w| names.contains(&w.name));
        }

        if workers.is_empty() {
            return Ok(Self::success(
                "No matching active workers found for requested worker_names filter.",
            ));
        }

        let sync_ref = if let Some(branch) = req.branch.clone().filter(|b| !b.trim().is_empty()) {
            branch
        } else {
            let task_store = open_task_store(&self.inner.cas_root).map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to open task store: {e}"),
                )
            })?;

            let mut epic_branch = None;
            if let Ok(tasks) = task_store.list(None) {
                if let Some(epic) = tasks
                    .iter()
                    .find(|t| t.task_type == TaskType::Epic && t.status == TaskStatus::InProgress)
                {
                    epic_branch = epic.branch.clone();
                }
                if epic_branch.is_none() {
                    epic_branch = tasks
                        .iter()
                        .find(|t| t.task_type == TaskType::Epic && t.status == TaskStatus::Open)
                        .and_then(|t| t.branch.clone());
                }
            }
            epic_branch.unwrap_or_else(|| {
                // Use local main branch, not origin/main. In factory mode the
                // supervisor merges worker branches into the local main branch,
                // so workers should rebase onto it directly.
                use crate::worktree::GitOperations;
                GitOperations::detect_repo_root(&self.inner.cas_root)
                    .ok()
                    .map(GitOperations::new)
                    .map(|git| git.detect_default_branch())
                    .unwrap_or_else(|| "main".to_string())
            })
        };

        let mut synced = Vec::new();
        let mut skipped = Vec::new();
        let mut failed = Vec::new();

        for worker in workers {
            let clone_path = match worker.metadata.get("clone_path") {
                Some(p) => p.clone(),
                None => {
                    skipped.push(format!("{} (missing clone_path metadata)", worker.name));
                    continue;
                }
            };
            let path = Path::new(&clone_path);
            if !path.exists() {
                skipped.push(format!(
                    "{} (clone path not found: {})",
                    worker.name, clone_path
                ));
                continue;
            }

            match sync_worker_clone(path, &sync_ref) {
                Ok(details) => synced.push(format!("{} ({})", worker.name, details)),
                Err(err) => failed.push(format!("{} ({})", worker.name, err)),
            }
        }

        let mut out =
            format!("Worker Sync Report\n==================\n\nSync target: {sync_ref}\n");
        if !synced.is_empty() {
            out.push_str("\nSynced:\n");
            for item in synced {
                out.push_str(&format!("  - {item}\n"));
            }
        }
        if !skipped.is_empty() {
            out.push_str("\nSkipped:\n");
            for item in skipped {
                out.push_str(&format!("  - {item}\n"));
            }
        }
        if !failed.is_empty() {
            out.push_str("\nFailed:\n");
            for item in failed {
                out.push_str(&format!("  - {item}\n"));
            }
        }

        Ok(Self::success(out))
    }

    pub(super) async fn factory_gc_report(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_prompt_queue_store, open_worktree_store};
        use cas_types::WorktreeStatus;
        use std::path::Path;

        let stale_after = req.older_than_secs.unwrap_or(120);
        let agent_store = open_agent_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open agent store: {e}"),
            )
        })?;
        let stale_agents = agent_store.list_stale(stale_after).unwrap_or_default();

        let prompt_queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open prompt queue: {e}"),
            )
        })?;
        let pending_prompts = prompt_queue.pending_count().unwrap_or(0);

        let worktree_store = open_worktree_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open worktree store: {e}"),
            )
        })?;
        let active_worktrees = worktree_store
            .list_by_status(WorktreeStatus::Active)
            .unwrap_or_default();
        let orphan_worktrees: Vec<_> = active_worktrees
            .iter()
            .filter(|wt| !Path::new(&wt.path).exists())
            .collect();

        let mut out = String::from("Factory GC Report\n=================\n");
        out.push_str(&format!(
            "\nStale agent threshold: {}s\nStale agents: {}\nPending prompts: {}\nActive worktrees: {}\nOrphan worktrees: {}\n",
            stale_after,
            stale_agents.len(),
            pending_prompts,
            active_worktrees.len(),
            orphan_worktrees.len()
        ));

        if !stale_agents.is_empty() {
            out.push_str("\nStale agents:\n");
            for a in &stale_agents {
                out.push_str(&format!("  - {} ({})\n", a.name, a.id));
            }
        }
        if !orphan_worktrees.is_empty() {
            out.push_str("\nOrphan worktrees:\n");
            for wt in orphan_worktrees {
                out.push_str(&format!("  - {} ({})\n", wt.id, wt.path.display()));
            }
        }

        // Task cas-a9ab: surface uncommitted files in the main worktree as
        // "likely prior-factory WIP". Informational only — we never auto-delete.
        if let Some(summary) =
            crate::hooks::handlers::session_hygiene::wip_candidates(&self.inner.cas_root)
        {
            out.push_str(&format!(
                "\nMain worktree: {}\n",
                summary.worktree.display()
            ));
            if summary.is_clean() {
                out.push_str("Prior-factory WIP candidates: none (worktree clean)\n");
            } else {
                out.push_str(&format!(
                    "Prior-factory WIP candidates: {} ({} untracked, {} modified)\n",
                    summary.entries.len(),
                    summary.untracked_count(),
                    summary.modified_count(),
                ));
                for entry in &summary.entries {
                    out.push_str(&format!(
                        "  [{}] {} {}\n",
                        entry.label(),
                        entry.status,
                        entry.path,
                    ));
                }
                out.push_str(
                    "\nNote: these are not auto-deleted. Inspect, then commit/salvage/discard.\n",
                );
            }
        }

        Ok(Self::success(out))
    }

    pub(super) async fn factory_gc_cleanup(
        &self,
        req: FactoryRequest,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::{open_agent_store, open_prompt_queue_store, open_worktree_store};
        use cas_types::{AgentRole, WorktreeStatus};
        use std::path::Path;

        let stale_after = req.older_than_secs.unwrap_or(120);
        let agent_store = open_agent_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open agent store: {e}"),
            )
        })?;
        let stale_agents = agent_store.list_stale(stale_after).unwrap_or_default();
        let mut stale_marked = 0usize;
        for agent in stale_agents {
            // Don't let workers prune supervisors/directors
            if agent.role == AgentRole::Supervisor || agent.role == AgentRole::Director {
                continue;
            }
            if agent_store.mark_stale(&agent.id).is_ok() {
                stale_marked += 1;
            }
        }

        let worktree_store = open_worktree_store(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open worktree store: {e}"),
            )
        })?;
        let active_worktrees = worktree_store
            .list_by_status(WorktreeStatus::Active)
            .unwrap_or_default();
        let mut orphan_marked_removed = 0usize;
        for mut wt in active_worktrees {
            if !Path::new(&wt.path).exists() {
                wt.mark_removed();
                if worktree_store.update(&wt).is_ok() {
                    orphan_marked_removed += 1;
                }
            }
        }

        // Clear prompt queue only when explicitly forced.
        let mut cleared_prompts = 0usize;
        if req.force.unwrap_or(false) {
            let prompt_queue = open_prompt_queue_store(&self.inner.cas_root).map_err(|e| {
                Self::error(
                    ErrorCode::INTERNAL_ERROR,
                    format!("Failed to open prompt queue: {e}"),
                )
            })?;
            cleared_prompts = prompt_queue.clear().unwrap_or(0);
        }

        Ok(Self::success(format!(
            "Factory GC cleanup complete.\n\nStale agents marked: {stale_marked}\nOrphan worktrees marked removed: {orphan_marked_removed}\nPrompt queue entries cleared: {cleared_prompts}"
        )))
    }
}

/// Returns the set of worker names this supervisor owns, derived from the `CAS_FACTORY_WORKER_NAMES`
/// environment variable. Returns `None` when not running as a supervisor or when the variable is
/// absent, meaning no scoping should be applied.
fn supervisor_owned_workers() -> Option<std::collections::HashSet<String>> {
    let role = std::env::var("CAS_AGENT_ROLE").unwrap_or_default();
    if !role.eq_ignore_ascii_case("supervisor") {
        return None;
    }
    let csv = std::env::var("CAS_FACTORY_WORKER_NAMES").ok()?;
    if csv.trim().is_empty() {
        return None;
    }
    Some(
        csv.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .collect(),
    )
}

fn run_git(path: &std::path::Path, args: &[&str]) -> std::result::Result<String, String> {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(path)
        .output()
        .map_err(|e| format!("git {} failed to start: {}", args.join(" "), e))?;

    if !output.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn sync_worker_clone(
    path: &std::path::Path,
    sync_ref: &str,
) -> std::result::Result<String, String> {
    let status = run_git(path, &["status", "--porcelain"])?;
    let mut stashed = false;

    if !status.trim().is_empty() {
        let stash_msg = format!(
            "cas-factory-auto-sync {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
        );
        let stash_out = run_git(
            path,
            &["stash", "push", "--include-untracked", "-m", &stash_msg],
        )?;
        if !stash_out.contains("No local changes") {
            stashed = true;
        }
    }

    let _ = run_git(path, &["fetch", "origin"]);

    if let Err(rebase_err) = run_git(path, &["rebase", sync_ref]) {
        let _ = run_git(path, &["rebase", "--abort"]);
        if stashed {
            let _ = run_git(path, &["stash", "pop"]);
        }
        return Err(format!("rebase failed: {rebase_err}"));
    }

    if stashed {
        run_git(path, &["stash", "pop"])
            .map_err(|e| format!("sync applied but stash pop failed: {e}"))?;
    }

    Ok(if stashed {
        "stashed + rebased + restored".to_string()
    } else {
        "rebased cleanly".to_string()
    })
}

/// Format a timestamp as relative time (e.g., "2s ago", "5m ago")
fn format_relative_time(dt: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let diff = now.signed_duration_since(dt);

    if diff.num_seconds() < 0 {
        return "just now".to_string();
    }

    if diff.num_seconds() < 60 {
        return format!("{}s ago", diff.num_seconds());
    }

    if diff.num_minutes() < 60 {
        return format!("{}m ago", diff.num_minutes());
    }

    if diff.num_hours() < 24 {
        return format!("{}h ago", diff.num_hours());
    }

    format!("{}d ago", diff.num_days())
}

/// cas-8240 two-band liveness label for `factory_worker_status`:
///
/// * `elapsed >= WORKER_DEAD_SECS` → `" [DEAD]"` (hard escalation —
///   caller also surfaces the transcript path for salvage).
/// * `WORKER_STALE_SECS <= elapsed < WORKER_DEAD_SECS` → `" [stale]"`
///   (grace-window indicator — the worker slipped past the prune
///   without being `mark_stale`'d, but it's too early to declare it
///   dead).
/// * Otherwise → `""` (no label).
///
/// Leading space is intentional: the caller concatenates the returned
/// slice directly after the `heartbeat: <Xs ago>` segment, and an empty
/// string avoids a trailing space when the worker is fresh. Returning
/// `&'static str` keeps this allocation-free.
fn liveness_label_for(elapsed_secs: i64) -> &'static str {
    if elapsed_secs >= WORKER_DEAD_SECS {
        " [DEAD]"
    } else if elapsed_secs >= WORKER_STALE_SECS {
        " [stale]"
    } else {
        ""
    }
}

/// Derive the Claude Code transcript path for an agent from its worktree clone
/// path and session id.
///
/// Claude Code persists each session's JSONL transcript under
/// `~/.claude/projects/<escaped-cwd>/<session-id>.jsonl`, where `<escaped-cwd>`
/// is the absolute cwd with `/`, `.`, and `_` collapsed to `-`. Surfacing this
/// path in `worker_status` lets a supervisor open the last in-flight tool call
/// after a worker dies without manually reconstructing the escape (cas-2749).
fn derive_transcript_path(clone_path: &str, session_id: &str) -> String {
    // Claude Code's `cwd` escape observed in the wild: both `/` and `.` are
    // collapsed to `-`. Example: `/home/a/.cas/worktrees/x` becomes
    // `-home-a--cas-worktrees-x` (the `-.` pair produces `--`). Underscores
    // and other characters are preserved.
    let escaped: String = clone_path
        .chars()
        .map(|c| match c {
            '/' | '.' => '-',
            other => other,
        })
        .collect();
    // `~` is literal here so the path remains portable across users; the
    // supervisor resolves it in their shell.
    format!("~/.claude/projects/{escaped}/{session_id}.jsonl")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_transcript_path_matches_claude_code_escape() {
        // Observed in the field: worker crisp-badger-65 worktree yields
        // `-home-pippenz-Petrastella-cas-src--cas-worktrees-crisp-badger-65`.
        let clone = "/home/pippenz/Petrastella/cas-src/.cas/worktrees/crisp-badger-65";
        let session = "064e7b23-331d-4dae-9c6a-721cbbe9c024";
        let got = derive_transcript_path(clone, session);
        assert_eq!(
            got,
            "~/.claude/projects/-home-pippenz-Petrastella-cas-src--cas-worktrees-crisp-badger-65/\
             064e7b23-331d-4dae-9c6a-721cbbe9c024.jsonl"
        );
    }

    #[test]
    fn derive_transcript_path_escapes_dots_preserves_underscores() {
        // Dots collapse to '-'; underscores are preserved (observed from
        // `~/.claude/projects/` layout in field workers).
        let got = derive_transcript_path("/tmp/my_proj.sub", "abc");
        assert_eq!(got, "~/.claude/projects/-tmp-my_proj-sub/abc.jsonl");
    }

    // --- cas-8240: two-band stale/dead threshold constants ------------------

    /// AC anchor: `WORKER_STALE_SECS` is pinned at 30. The supervisor-facing
    /// footer embeds this value as a literal ("30s heartbeat age") and the
    /// daemon heartbeat tick is tuned against it, so a silent change here
    /// would desync the prune window from the UX text.
    #[test]
    fn worker_stale_secs_is_pinned_at_30() {
        assert_eq!(WORKER_STALE_SECS, 30);
    }

    /// AC anchor: `WORKER_DEAD_SECS` is pinned at 75. The two-band model
    /// requires DEAD to lag STALE by roughly one grace window so scheduler
    /// jitter and missed ticks do not produce false-positive [DEAD] labels.
    /// Bumping this silently would regress the cas-8240 fix.
    #[test]
    fn worker_dead_secs_is_pinned_at_75() {
        assert_eq!(WORKER_DEAD_SECS, 75);
    }

    /// Invariant: the dead threshold must strictly exceed the stale
    /// threshold. Otherwise the two-band render collapses into one band
    /// and we reintroduce the false-positive DEAD labeling cas-8240 fixes.
    #[test]
    fn worker_dead_secs_exceeds_stale_secs() {
        assert!(
            WORKER_DEAD_SECS > WORKER_STALE_SECS,
            "WORKER_DEAD_SECS ({WORKER_DEAD_SECS}) must exceed WORKER_STALE_SECS ({WORKER_STALE_SECS}) — the two-band model collapses otherwise"
        );
    }

    // --- cas-8240: liveness_label_for branch matrix -------------------------

    #[test]
    fn liveness_label_fresh_worker_is_empty() {
        assert_eq!(liveness_label_for(0), "");
        assert_eq!(liveness_label_for(WORKER_STALE_SECS - 1), "");
    }

    #[test]
    fn liveness_label_grace_window_is_stale() {
        // Exactly at STALE → [stale]; just below DEAD → still [stale].
        assert_eq!(liveness_label_for(WORKER_STALE_SECS), " [stale]");
        assert_eq!(liveness_label_for(WORKER_DEAD_SECS - 1), " [stale]");
    }

    #[test]
    fn liveness_label_past_dead_is_hard_dead() {
        // Exactly at DEAD → [DEAD]; well past → still [DEAD].
        assert_eq!(liveness_label_for(WORKER_DEAD_SECS), " [DEAD]");
        assert_eq!(liveness_label_for(WORKER_DEAD_SECS * 10), " [DEAD]");
    }

    #[test]
    fn liveness_label_distinguishes_stale_from_dead() {
        // The cas-8240 core behavior: stale and DEAD are distinct bands.
        // A mutation that collapsed the stale branch into " [DEAD]"
        // would fail here.
        let stale = liveness_label_for(WORKER_STALE_SECS);
        let dead = liveness_label_for(WORKER_DEAD_SECS);
        assert_ne!(stale, dead, "stale and DEAD bands must render distinct labels");
        assert!(stale.contains("stale"));
        assert!(dead.contains("DEAD"));
    }
}
