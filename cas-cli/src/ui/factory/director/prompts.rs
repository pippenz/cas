//! Auto-prompting system for the Director
//!
//! Generates prompts based on detected CAS state changes and injects them
//! into the appropriate agent's terminal.

use crate::config::AutoPromptConfig;
use crate::ui::factory::director::data::DirectorData;
use crate::ui::factory::director::events::DirectorEvent;
use cas_mux::SupervisorCli;
use cas_types::TaskStatus;

/// Count tasks that are actually dispatchable to an idle worker.
///
/// `DirectorData::ready_tasks` conflates `Open` and `Blocked` (see
/// `crates/cas-factory/src/director.rs`). Blocked tasks cannot be started, and
/// Closed tasks never appear in `ready_tasks` at all, but this count decides
/// whether the `WorkerIdle` / `AgentRegistered` prompts should offer an assign
/// command. Count only `Open` and only tasks without an assignee already set.
/// See cas-177f.
fn dispatchable_ready_count(data: &DirectorData) -> usize {
    data.ready_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Open && t.assignee.is_none())
        .count()
}

fn live_worker_session_id(data: &DirectorData, worker_name: &str) -> Option<String> {
    data.agents
        .iter()
        .find(|agent| agent.name == worker_name)
        .map(|agent| agent.id.clone())
        .or_else(|| {
            data.agent_id_to_name
                .iter()
                .find_map(|(id, name)| (name == worker_name).then(|| id.clone()))
        })
}

/// A prompt to be injected into an agent's terminal
#[derive(Debug, Clone)]
pub struct Prompt {
    /// Target agent name (worker name or "supervisor")
    pub target: String,
    /// Prompt text to inject
    pub text: String,
}

/// Wrap a message with response instructions
///
/// Appends instructions telling the agent how to respond using the MCP message tool.
/// The command prefix differs by harness:
/// - Claude: `mcp__cas__`
/// - Codex: `mcp__cs__`
///
/// # Arguments
/// * `message` - The original message text
/// * `respond_to` - The target agent name for responses (e.g., "supervisor", "swift-fox")
/// * `receiver_cli` - CLI harness for the agent receiving this message
///
/// # Returns
/// The message with response instructions appended at the end
pub fn with_response_instructions(
    message: &str,
    respond_to: &str,
    receiver_cli: SupervisorCli,
) -> String {
    let prefix = receiver_cli.capabilities().tool_prefix;
    format!(
        "{message}\n\n---\nTo respond to this message, use: `{prefix}coordination action=message target={respond_to} message=\"...\"`"
    )
}

/// Generate a prompt for a detected event
///
/// Returns `Some(Prompt)` if a prompt should be sent for this event,
/// or `None` if no prompt is needed or if the event type is disabled in config.
pub fn generate_prompt(
    event: &DirectorEvent,
    data: &DirectorData,
    supervisor_name: &str,
    config: &AutoPromptConfig,
    supervisor_cli: SupervisorCli,
    worker_cli: SupervisorCli,
) -> Option<Prompt> {
    // Check global enable flag first
    if !config.enabled {
        return None;
    }
    let supervisor_prefix = supervisor_cli.capabilities().tool_prefix;
    let worker_prefix = worker_cli.capabilities().tool_prefix;

    match event {
        DirectorEvent::TaskAssigned {
            task_id,
            task_title,
            worker,
        } => {
            if !config.on_task_assigned {
                return None;
            }

            let text = format!(
                "You have been assigned a new task:\n\
                 Task ID: {task_id}\n\
                 Title: {task_title}\n\n\
                 View full details: {worker_prefix}task action=show id={task_id}\n\
                 Start working: {worker_prefix}task action=start id={task_id}\n\
                 Then send an ACK to supervisor with your execution plan.\n\
                 While working, post progress notes with {worker_prefix}task action=notes.\n\
                 If blocked, set status=blocked and explain the blocker."
            );

            Some(Prompt {
                target: worker.clone(),
                text: with_response_instructions(&text, supervisor_name, worker_cli),
            })
        }

        DirectorEvent::TaskCompleted {
            task_id,
            task_title,
            worker,
        } => {
            if !config.on_task_completed {
                return None;
            }

            // cas-6aaf: check current task state before emitting guidance.
            //
            // `TaskCompleted` fires when a task disappears from `in_progress_tasks`,
            // which happens when it transitions to `Closed`. However, lease churn
            // can also cause a task to temporarily regress to `Open` (lease expired
            // → status reset to Open). We check the current snapshot to distinguish
            // the two cases and avoid emitting "please close" guidance for a task
            // the worker has already closed.
            //
            // State resolution:
            //   - task absent from ready+in_progress → closed (expected path)
            //   - task in ready_tasks as Open       → lease expired, still needs close
            //   - task in in_progress_tasks         → still being worked (edge case)
            let in_ready = data
                .ready_tasks
                .iter()
                .any(|t| t.id == *task_id && t.status == cas_types::TaskStatus::Open);
            let in_progress = data.in_progress_tasks.iter().any(|t| t.id == *task_id);

            let text = if in_ready {
                // Task regressed to Open (lease expired) — worker needs to close it.
                format!(
                    "Worker {worker} was working on task {task_id} ({task_title}) but \
                     it is now Open (lease may have expired).\n\n\
                     Next steps:\n\
                     - Ask the worker to close: {worker_prefix}task action=close id={task_id}\n\
                     - If they have uncommitted work, they should commit first, then close\n\
                     - If close triggers verification, the worker handles it (not you)\n\n\
                     Remember: workers close their own tasks, supervisors close epics."
                )
            } else if in_progress {
                // Still in progress — stale event, nothing to do.
                return None;
            } else {
                // Task is already closed (the normal path after a successful close).
                // Do NOT instruct the supervisor to ask the worker to close it again.
                format!(
                    "Worker {worker} has closed task {task_id} ({task_title}).\n\n\
                     Next steps:\n\
                     - Assign another task to this worker, OR\n\
                     - If all subtasks are done, verify and close the epic\n\n\
                     Remember: workers close their own tasks, supervisors close epics."
                )
            };

            Some(Prompt {
                target: supervisor_name.to_string(),
                text: with_response_instructions(&text, worker, supervisor_cli),
            })
        }

        DirectorEvent::TaskBlocked {
            task_id,
            task_title,
            worker,
        } => {
            if !config.on_task_blocked {
                return None;
            }

            let text = format!(
                "Worker {worker} is blocked on task {task_id} ({task_title}).\n\
                 They may need assistance or the blocker needs to be resolved."
            );

            Some(Prompt {
                target: supervisor_name.to_string(),
                text: with_response_instructions(&text, worker, supervisor_cli),
            })
        }

        DirectorEvent::WorkerIdle { worker } => {
            if !config.on_worker_idle {
                return None;
            }

            // Guard (cas-c790): supervisor / team-lead is never an idle worker.
            // The event detector filters this at the source (is_worker_agent_name),
            // but defense-in-depth here catches any path that bypasses the upstream
            // gate (e.g. supervisor name in worker_names on resume/reconnect — the
            // recurrence described in cas-c790 / cas-b67d).
            if worker == supervisor_name {
                return None;
            }

            // Defense-in-depth for stale queued events: only emit an idle nudge
            // when the current authoritative snapshot still contains this worker.
            // If the worker was shut down, crashed, or belonged to another session,
            // a stale WorkerIdle event must not tell the supervisor to assign into
            // the void.
            // Liveness gate only — the assignee interpolation below uses the
            // display name (`worker`), not this session ID. `task mine` matches
            // on display name, and `task update assignee=<session-id>` gets
            // silently normalized back to the display name (update.rs:176-186,
            // cas-dbbb). Advertising the session id here just adds a spurious
            // normalization warning on every assignment.
            let Some(_worker_session_id) = live_worker_session_id(data, worker) else {
                return None;
            };

            // Guard (cas-889d / cas-dbbb): suppress idle nudge if the worker already
            // has an active in_progress task OR an assigned-but-not-yet-started Open
            // task in the current snapshot. Checking in_progress_tasks alone misses
            // the window between `task.update assignee=<name>` (status stays Open)
            // and the worker calling `task start` (status becomes InProgress) — the
            // director would incorrectly re-fire WorkerIdle during that gap.
            //
            // Blocked tasks are EXCLUDED: `ready_tasks` contains both Open and Blocked
            // tasks, but a worker with only a Blocked task is genuinely stalled and may
            // still need an idle nudge. Including Blocked tasks here would suppress
            // WorkerIdle indefinitely for stalled workers.
            //
            // Checking by both display-name assignee (canonical DB path) and session-ID
            // assignee (legacy assignment path via agent_id_to_name) makes this robust
            // to either convention.
            let worker_is_busy =
                data.in_progress_tasks
                    .iter()
                    .chain(
                        data.ready_tasks
                            .iter()
                            .filter(|t| t.status == TaskStatus::Open),
                    )
                    .any(|t| {
                        t.assignee.as_deref() == Some(worker.as_str())
                            || data.agent_id_to_name.iter().any(|(id, name)| {
                                name == worker && t.assignee.as_deref() == Some(id)
                            })
                    });
            if worker_is_busy {
                return None;
            }

            // Count only truly-dispatchable tasks (Open + unassigned). See
            // `dispatchable_ready_count` for why `ready_tasks.len()` is wrong.
            let ready_count = dispatchable_ready_count(data);

            let text = if ready_count > 0 {
                // D-3 (cas-405f): do NOT embed the snapshot count here.
                //
                // `ready_count` comes from the director's epic-filtered snapshot
                // (app::filter_director_agents_to_current_session), which tracks
                // only tasks visible to the current epic scope. The live global
                // `task action=ready` often shows more — confirmed mismatches of
                // "said 1, actual 10" and "said 14, actual 25" were traced to this
                // gap. Advertising a stale number causes the supervisor to
                // under-assign or over-assign, so we remove the specific count and
                // direct them to the live command instead.
                //
                format!(
                    "Worker {worker} is idle with no assigned tasks.\n\
                     Ready tasks exist — check live: `{supervisor_prefix}task action=ready`\n\
                     Assign work: {supervisor_prefix}task action=update id=<task-id> assignee={worker}"
                )
            } else {
                // Do NOT suggest "closing the epic" here — the task snapshot may
                // be stale (cas-b67d D-3): the director refresh window is 2s, and
                // recently-created tasks may not yet be visible. Obeying "close the
                // epic" advice from a stale snapshot would orphan live open work.
                // Direct the supervisor to verify with a live query instead.
                format!(
                    "Worker {worker} is idle with no assigned tasks.\n\
                     No dispatchable tasks in current snapshot — verify with \
                     `{supervisor_prefix}task action=ready` before acting.\n\
                     If genuinely idle, assign new work or stand down this worker."
                )
            };

            Some(Prompt {
                target: supervisor_name.to_string(),
                text: with_response_instructions(&text, worker, supervisor_cli),
            })
        }

        DirectorEvent::AgentRegistered {
            agent_id,
            agent_name,
        } => {
            if !config.on_worker_ready {
                return None;
            }

            // Don't notify about supervisor registering
            if agent_name == supervisor_name {
                return None;
            }

            // Guard (cas-889d / cas-dbbb): suppress registration nudge if the
            // newly-registered worker already has an active in_progress task OR an
            // assigned-but-not-yet-started Open task (reconnect after session restart,
            // or assignment during the registration window). Check both ID-keyed and
            // name-keyed assignees for the same reason as WorkerIdle above.
            //
            // Blocked tasks are EXCLUDED (see WorkerIdle guard comment above).
            let worker_already_busy = data
                .in_progress_tasks
                .iter()
                .chain(
                    data.ready_tasks
                        .iter()
                        .filter(|t| t.status == TaskStatus::Open),
                )
                .any(|t| {
                    t.assignee.as_deref() == Some(agent_id.as_str())
                        || t.assignee.as_deref() == Some(agent_name.as_str())
                });
            if worker_already_busy {
                return None;
            }

            let ready_count = dispatchable_ready_count(data);
            let text = if ready_count > 0 {
                format!(
                    "Worker {agent_name} is ready and waiting for tasks.\n\
                     Ready tasks exist — check live: `{supervisor_prefix}task action=ready`\n\
                     Assign work: {supervisor_prefix}task action=update id=<task-id> assignee={agent_name}"
                )
            } else {
                format!(
                    "Worker {agent_name} is ready and waiting for tasks.\n\
                     No dispatchable tasks in current snapshot — verify with \
                     `{supervisor_prefix}task action=ready` before acting."
                )
            };

            Some(Prompt {
                target: supervisor_name.to_string(),
                text: with_response_instructions(&text, agent_name, supervisor_cli),
            })
        }

        DirectorEvent::EpicStarted { .. } => {
            // No prompt needed - supervisor already knows since they started the epic
            None
        }

        DirectorEvent::EpicCompleted { .. } => {
            // No prompt needed - supervisor already knows since they orchestrated the epic
            // completion (closed tasks, merged branches, shut down workers)
            None
        }

        DirectorEvent::EpicAllSubtasksClosed {
            epic_id,
            epic_title,
        } => {
            if !config.on_epic_completed {
                return None;
            }

            let text = format!(
                "All subtasks of epic '{epic_title}' ({epic_id}) are now closed.\n\n\
                 Next steps:\n\
                 - Verify the integrated result\n\
                 - Close the epic: {supervisor_prefix}task action=close id={epic_id} reason=\"All subtasks complete\"\n\
                 - Shut down idle workers if no more work"
            );

            Some(Prompt {
                target: supervisor_name.to_string(),
                text,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ui::factory::director::data::{AgentSummary, TaskSummary};
    use crate::ui::factory::director::prompts::*;
    use cas_mux::SupervisorCli;
    use cas_types::{AgentStatus, Priority, TaskStatus, TaskType};
    use std::collections::HashMap;

    fn make_data(ready_count: usize) -> DirectorData {
        let ready_tasks: Vec<TaskSummary> = (0..ready_count)
            .map(|i| TaskSummary {
                id: format!("task-{i}"),
                title: format!("Ready Task {i}"),
                status: TaskStatus::Open,
                priority: Priority::MEDIUM,
                assignee: None,
                task_type: TaskType::Task,
                epic: None,
                branch: None,
                updated_at: None,
            })
            .collect();

        DirectorData {
            ready_tasks,
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![AgentSummary {
                id: "sess-id-abc123".to_string(),
                name: "swift-fox".to_string(),
                status: AgentStatus::Active,
                current_task: None,
                latest_activity: None,
                last_heartbeat: Some(chrono::Utc::now()),
                pending_messages: 0,
            }],
            activity: vec![],
            agent_id_to_name: [("sess-id-abc123".to_string(), "swift-fox".to_string())]
                .into_iter()
                .collect(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        }
    }

    fn default_config() -> AutoPromptConfig {
        AutoPromptConfig::default()
    }

    fn codex() -> SupervisorCli {
        SupervisorCli::Codex
    }

    fn claude() -> SupervisorCli {
        SupervisorCli::Claude
    }

    #[test]
    fn test_task_assigned_prompt() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "swift-fox");
        assert!(prompt.text.contains("task-123"));
        assert!(prompt.text.contains("Implement feature X"));
        assert!(prompt.text.contains("mcp__cs__task action=start"));
        // Response instructions should be appended
        assert!(prompt.text.contains("To respond to this message, use:"));
        assert!(prompt.text.contains("target=supervisor"));
    }

    /// cas-6aaf: TaskCompleted with task already closed (the normal path).
    /// The prompt must NOT instruct the supervisor to ask the worker to close
    /// the task — it was already closed when the event fired.
    #[test]
    fn test_task_completed_prompt_already_closed() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        // Task not present in any active set = already closed.
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("swift-fox"));
        assert!(prompt.text.contains("task-123"));
        // Must say "closed" not "completed" — reflects actual final state.
        assert!(
            prompt.text.contains("closed"),
            "cas-6aaf: TaskCompleted prompt must say 'closed' (task is already closed): {}",
            prompt.text
        );
        // Must NOT instruct supervisor to close an already-closed task.
        assert!(
            !prompt.text.to_lowercase().contains("task action=close"),
            "cas-6aaf: TaskCompleted must not emit close instruction for already-closed task: {}",
            prompt.text
        );
        // Should clarify verification ownership.
        assert!(prompt.text.contains("workers close their own tasks"));
        assert!(prompt.text.contains("supervisors close epics"));
        // Response instructions should point to the worker.
        assert!(prompt.text.contains("To respond to this message, use:"));
        assert!(prompt.text.contains("target=swift-fox"));
    }

    /// cas-6aaf: TaskCompleted when task regressed to Open (lease expired).
    /// The supervisor SHOULD be asked to have the worker close it — the task
    /// is still open and needs attention.
    #[test]
    fn test_task_completed_prompt_lease_expired_still_open() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        // Task is in ready_tasks as Open — lease expired, not yet closed.
        let task = TaskSummary {
            id: "task-123".to_string(),
            title: "Implement feature X".to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: Some("swift-fox".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let data = DirectorData {
            ready_tasks: vec![task],
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        // When lease expired and task regressed to Open, supervisor should ask worker to close.
        assert!(
            prompt.text.to_lowercase().contains("task action=close"),
            "cas-6aaf: TaskCompleted for lease-expired Open task must include close instruction: {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("task-123"),
            "Prompt must identify the task: {}",
            prompt.text
        );
    }

    /// cas-6aaf: TaskCompleted when task is still InProgress returns None
    /// (stale event, nothing actionable).
    #[test]
    fn test_task_completed_prompt_still_in_progress_suppressed() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        // Task is still in in_progress — stale event.
        let task = TaskSummary {
            id: "task-123".to_string(),
            title: "Implement feature X".to_string(),
            status: TaskStatus::InProgress,
            priority: Priority::MEDIUM,
            assignee: Some("swift-fox".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let data = DirectorData {
            ready_tasks: vec![],
            in_progress_tasks: vec![task],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt.is_none(),
            "cas-6aaf: TaskCompleted must be suppressed when task is still in_progress: {:?}",
            prompt.map(|p| p.text)
        );
    }

    #[test]
    fn test_worker_idle_with_ready_tasks() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let data = make_data(3); // 3 ready tasks in snapshot
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("swift-fox"));
        assert!(prompt.text.contains("idle"));
        // D-3 (cas-405f): the specific count is intentionally NOT included — the
        // snapshot count diverges from the live global `task action=ready` result
        // because the director filters tasks to the current epic scope. We verify
        // that the prompt directs the supervisor to the live command instead.
        assert!(
            !prompt.text.contains("3 ready tasks"),
            "Prompt must not embed stale snapshot count (D-3): {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("task action=ready"),
            "Prompt must direct supervisor to live task action=ready (D-3): {}",
            prompt.text
        );
    }

    #[test]
    fn test_worker_idle_no_ready_tasks() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0); // No ready tasks
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        let lower = prompt.text.to_lowercase();
        assert!(
            lower.contains("no ready tasks") || lower.contains("no dispatchable"),
            "Expected 'no ready tasks' or 'no dispatchable' in: {}",
            prompt.text
        );
    }

    /// Regression for cas-b67d D-3: the zero-ready-task nudge must NOT instruct
    /// the supervisor to close the epic. The director snapshot may be stale; the
    /// epic may have open children that just aren't visible in this refresh cycle.
    /// Obeying "close the epic" advice would orphan live work.
    #[test]
    fn test_worker_idle_no_close_epic_advice() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0); // No ready tasks in snapshot
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        // Must never suggest closing the epic — the snapshot may be stale and
        // the epic might have live open children not visible in this refresh.
        assert!(
            !prompt.text.to_lowercase().contains("closing the epic")
                && !prompt.text.to_lowercase().contains("close the epic"),
            "WorkerIdle nudge must not advise closing the epic (stale-snapshot risk): {:?}",
            prompt.text
        );
    }

    #[test]
    fn test_worker_idle_suppressed_when_worker_absent_from_live_snapshot() {
        let event = DirectorEvent::WorkerIdle {
            worker: "stale-worker".to_string(),
        };
        let data = make_data(2);
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "WorkerIdle must not emit for a worker absent from current DirectorData: {:?}",
            prompt.map(|p| p.text)
        );
    }

    #[test]
    fn test_epic_completed_no_prompt() {
        let event = DirectorEvent::EpicCompleted {
            epic_id: "epic-456".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "EpicCompleted should not generate a prompt"
        );
    }

    #[test]
    fn test_epic_all_subtasks_closed_has_no_branch_or_main_instructions() {
        let event = DirectorEvent::EpicAllSubtasksClosed {
            epic_id: "epic-456".to_string(),
            epic_title: "Test Epic".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();
        let lower = prompt.text.to_lowercase();

        assert!(
            !lower.contains("cherry-pick") && !lower.contains("main"),
            "Epic completion prompt must not prescribe branch/merge/main instructions: {}",
            prompt.text
        );
        assert!(prompt.text.contains("task action=close id=epic-456"));
    }

    #[test]
    fn test_worker_ready_prompt() {
        let event = DirectorEvent::AgentRegistered {
            agent_id: "agent-123".to_string(),
            agent_name: "calm-owl".to_string(),
        };
        let data = make_data(3);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("calm-owl"));
        assert!(prompt.text.contains("ready"));
        assert!(!prompt.text.contains("3 ready tasks"));
        assert!(prompt.text.contains("task action=ready"));
        assert!(prompt.text.contains("assignee=calm-owl"));
    }

    #[test]
    fn test_worker_ready_no_tasks() {
        let event = DirectorEvent::AgentRegistered {
            agent_id: "agent-123".to_string(),
            agent_name: "calm-owl".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("calm-owl"));
        assert!(prompt.text.contains("ready"));
        assert!(prompt.text.contains("No dispatchable tasks"));
        assert!(prompt.text.contains("task action=ready"));
    }

    #[test]
    fn test_worker_ready_disabled() {
        let event = DirectorEvent::AgentRegistered {
            agent_id: "agent-123".to_string(),
            agent_name: "calm-owl".to_string(),
        };
        let data = make_data(0);
        let config = AutoPromptConfig {
            on_worker_ready: false,
            ..default_config()
        };

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(prompt.is_none());
    }

    #[test]
    fn test_supervisor_registered_no_prompt() {
        // Supervisor registering should not notify itself
        let event = DirectorEvent::AgentRegistered {
            agent_id: "agent-sup".to_string(),
            agent_name: "supervisor".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(prompt.is_none());
    }

    #[test]
    fn test_config_disabled_globally() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0);
        let config = AutoPromptConfig {
            enabled: false,
            ..default_config()
        };

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(prompt.is_none());
    }

    #[test]
    fn test_config_task_assigned_disabled() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0);
        let config = AutoPromptConfig {
            on_task_assigned: false,
            ..default_config()
        };

        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(prompt.is_none());
    }

    #[test]
    fn test_with_response_instructions() {
        let message = "Hello worker, please do X";
        let wrapped = with_response_instructions(message, "supervisor", codex());

        // Original message should be preserved
        assert!(wrapped.starts_with(message));
        // Response instructions should be at the end
        assert!(wrapped.contains("To respond to this message, use:"));
        assert!(wrapped.contains("mcp__cs__coordination action=message"));
        assert!(wrapped.contains("target=supervisor"));
    }

    #[test]
    fn test_claude_prefix_for_worker_and_supervisor() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, claude(), claude()).unwrap();
        assert!(prompt.text.contains("mcp__cas__task action=start"));
        assert!(
            prompt
                .text
                .contains("mcp__cas__coordination action=message")
        );
    }

    // ── cas-889d regression tests ─────────────────────────────────────────────

    /// Build a DirectorData with one in-progress task assigned to `assignee`.
    fn make_data_with_in_progress(assignee: &str) -> DirectorData {
        let task = TaskSummary {
            id: "task-active".to_string(),
            title: "Active Task".to_string(),
            status: TaskStatus::InProgress,
            priority: Priority::MEDIUM,
            assignee: Some(assignee.to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        DirectorData {
            ready_tasks: vec![],
            in_progress_tasks: vec![task],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        }
    }

    /// WorkerIdle assignment guidance must use the worker's display name, not
    /// the session ID. `task mine` matches on display name, and
    /// `task update assignee=<session-id>` gets silently normalized back to
    /// the display name (update.rs:176-186, cas-dbbb) — so the session ID
    /// form just produces a spurious warning. The live-session-ID lookup
    /// (`live_worker_session_id`) still gates whether a prompt fires at all
    /// (cas-c790 defense-in-depth), it just isn't interpolated into the
    /// assignee field.
    #[test]
    fn test_worker_idle_assignee_uses_display_name() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };

        let data = make_data(2);

        let config = default_config();
        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert!(
            prompt.text.contains("assignee=swift-fox"),
            "WorkerIdle must use the display name in assignee field, got: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("assignee=sess-id-abc123"),
            "WorkerIdle must not use the session ID in assignee field, got: {}",
            prompt.text
        );
    }

    /// cas-889d: WorkerIdle must return None when the worker already has an
    /// in-progress task (ID-keyed assignee path). Prevents spurious idle nudges
    /// that race with actual work.
    #[test]
    fn test_889d_worker_idle_suppressed_when_busy_by_session_id() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };

        // in_progress task assigned by session ID; agent_id_to_name maps it.
        let mut data = make_data_with_in_progress("sess-id-abc123");
        data.agent_id_to_name
            .insert("sess-id-abc123".to_string(), "swift-fox".to_string());

        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "cas-889d: WorkerIdle must be suppressed when worker has active task (ID key), got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    /// cas-889d: WorkerIdle must return None when the in-progress task uses the
    /// display-name as assignee (legacy manual assignment path).
    #[test]
    fn test_889d_worker_idle_suppressed_when_busy_by_display_name() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };

        // in_progress task assigned by display name (legacy manual path).
        let data = make_data_with_in_progress("swift-fox");
        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "cas-889d: WorkerIdle must be suppressed when worker has active task (name key), got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    /// AgentRegistered assignment guidance must use the registered display
    /// name, not the session ID — same rationale as WorkerIdle above
    /// (cas-dbbb: `task mine` matches display name; session-id assignees get
    /// silently normalized back to it, so advertising the session id here
    /// just adds a spurious warning).
    #[test]
    fn test_agent_registered_assignee_uses_display_name() {
        let event = DirectorEvent::AgentRegistered {
            agent_id: "sess-id-abc123".to_string(),
            agent_name: "calm-owl".to_string(),
        };
        let data = make_data(2);
        let config = default_config();
        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert!(
            prompt.text.contains("assignee=calm-owl"),
            "AgentRegistered must use display name in assignee field, got: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("assignee=sess-id-abc123"),
            "AgentRegistered must not use the session ID in assignee field, got: {}",
            prompt.text
        );
    }

    /// cas-889d: AgentRegistered must return None when the worker already has an
    /// active in-progress task (reconnect after session restart).
    #[test]
    fn test_889d_agent_registered_suppressed_when_busy() {
        let event = DirectorEvent::AgentRegistered {
            agent_id: "sess-id-abc123".to_string(),
            agent_name: "calm-owl".to_string(),
        };

        // Busy by session ID.
        let data = make_data_with_in_progress("sess-id-abc123");
        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "cas-889d: AgentRegistered must be suppressed when worker already has active task, got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    /// cas-dbbb: AgentRegistered and WorkerIdle must be suppressed when the worker
    /// has an assigned Open (not yet InProgress) task. Without this, the director
    /// fires idle/registration nudges in the window between `task update assignee=X`
    /// (task stays Open) and the worker calling `task start` (task becomes InProgress).
    #[test]
    fn test_dbbb_idle_suppressed_when_worker_has_assigned_ready_task() {
        // ready_tasks (Open) with worker as the assignee — simulates the post-assign,
        // pre-start window.
        let task = TaskSummary {
            id: "task-assigned".to_string(),
            title: "Assigned Task".to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: Some("swift-fox".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let data = DirectorData {
            ready_tasks: vec![task],
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };
        let config = default_config();

        // WorkerIdle must be suppressed.
        let idle_event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let prompt = generate_prompt(&idle_event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt.is_none(),
            "cas-dbbb: WorkerIdle must be suppressed when worker has an assigned Open task \
             (post-assign, pre-start window): got {:?}",
            prompt.map(|p| p.text)
        );

        // AgentRegistered must also be suppressed.
        let reg_event = DirectorEvent::AgentRegistered {
            agent_id: "sess-id-xyz".to_string(),
            agent_name: "swift-fox".to_string(),
        };
        let prompt2 = generate_prompt(&reg_event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt2.is_none(),
            "cas-dbbb: AgentRegistered must be suppressed when worker has an assigned Open task: \
             got {:?}",
            prompt2.map(|p| p.text)
        );
    }

    /// cas-dbbb P2: WorkerIdle must NOT be suppressed when the worker's only task
    /// is Blocked. A Blocked task means the worker is genuinely stalled; the
    /// supervisor still needs an idle nudge so they can resolve the blocker or
    /// assign new work. Including Blocked tasks in the busy-guard would suppress
    /// the nudge indefinitely.
    #[test]
    fn test_dbbb_idle_not_suppressed_when_worker_only_has_blocked_task() {
        let blocked_task = TaskSummary {
            id: "task-blocked".to_string(),
            title: "Blocked Task".to_string(),
            status: TaskStatus::Blocked,
            priority: Priority::MEDIUM,
            assignee: Some("swift-fox".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let data = DirectorData {
            // Blocked task is in ready_tasks (Open|Blocked both land here).
            ready_tasks: vec![blocked_task],
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![AgentSummary {
                id: "sess-id-abc123".to_string(),
                name: "swift-fox".to_string(),
                status: AgentStatus::Active,
                current_task: None,
                latest_activity: None,
                last_heartbeat: Some(chrono::Utc::now()),
                pending_messages: 0,
            }],
            activity: vec![],
            agent_id_to_name: [("sess-id-abc123".to_string(), "swift-fox".to_string())]
                .into_iter()
                .collect(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };

        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt.is_some(),
            "cas-dbbb P2: WorkerIdle must NOT be suppressed when worker has only a Blocked task \
             (blocked ≠ busy). Got: None"
        );
    }

    /// cas-dbbb P2: WorkerIdle must be suppressed when the worker has a session-ID
    /// assignee on an Open task in ready_tasks, with agent_id_to_name mapping the
    /// session ID to the worker's display name. This covers the chain()
    /// + session-ID path added in cas-dbbb.
    #[test]
    fn test_dbbb_idle_suppressed_via_session_id_in_ready_open_task() {
        let open_task = TaskSummary {
            id: "task-open-session-id".to_string(),
            title: "Session-ID assigned Open task".to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: Some("sess-id-abc123".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let mut data = DirectorData {
            ready_tasks: vec![open_task],
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };
        // The reverse-lookup maps session ID → display name.
        data.agent_id_to_name
            .insert("sess-id-abc123".to_string(), "swift-fox".to_string());

        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt.is_none(),
            "cas-dbbb P2: WorkerIdle must be suppressed when worker has a session-ID assigned \
             Open task in ready_tasks (agent_id_to_name reverse-lookup path). Got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    /// cas-dbbb P2: AgentRegistered must be suppressed when the worker's session ID
    /// matches an assignee on an Open task in ready_tasks. This verifies the
    /// chain() + agent_id path added in cas-dbbb.
    #[test]
    fn test_dbbb_agent_registered_suppressed_via_session_id_in_ready_open_task() {
        let open_task = TaskSummary {
            id: "task-reg-session-id".to_string(),
            title: "Session-ID assigned for registration test".to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            // Assignee is the session UUID (agent_id), not the display name.
            assignee: Some("sess-id-abc123".to_string()),
            task_type: TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        };
        let data = DirectorData {
            ready_tasks: vec![open_task],
            in_progress_tasks: vec![],
            epic_tasks: vec![],
            agents: vec![],
            activity: vec![],
            agent_id_to_name: HashMap::new(),
            changes: vec![],
            git_loaded: true,
            reminders: vec![],
            epic_closed_counts: HashMap::new(),
        };

        let event = DirectorEvent::AgentRegistered {
            agent_id: "sess-id-abc123".to_string(),
            agent_name: "calm-owl".to_string(),
        };
        let config = default_config();
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());
        assert!(
            prompt.is_none(),
            "cas-dbbb P2: AgentRegistered must be suppressed when session ID (agent_id) is the \
             assignee of an Open task in ready_tasks. Got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    // ── cas-c790 regression tests ─────────────────────────────────────────────

    /// cas-c790: WorkerIdle must return None when the "worker" is actually the
    /// supervisor / team-lead. This is defense-in-depth at the prompt layer — the
    /// event detector already filters via is_worker_agent_name, but that gate can
    /// be bypassed when the supervisor's name ends up in worker_names on
    /// resume/reconnect paths (the recurrence pattern described in cas-c790).
    #[test]
    fn test_c790_worker_idle_never_fires_for_supervisor() {
        // The worker name in the event is the supervisor's name.
        let event = DirectorEvent::WorkerIdle {
            worker: "supervisor".to_string(),
        };
        let data = make_data(5); // 5 ready tasks — the worst-case scenario
        let config = default_config();

        // Pass "supervisor" as supervisor_name — the prompt must return None.
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_none(),
            "cas-c790: WorkerIdle for the supervisor must return None regardless of ready count. \
             Got: {:?}",
            prompt.map(|p| p.text)
        );
    }

    /// cas-c790: WorkerIdle for a legitimate worker must still fire (not
    /// accidentally suppressed by the supervisor-name guard).
    #[test]
    fn test_c790_worker_idle_still_fires_for_real_workers() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        // No in_progress tasks (so the busy guard doesn't suppress).
        let data = make_data(1);
        let config = default_config();

        // "supervisor" is distinct from "swift-fox" — nudge must fire.
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), codex());

        assert!(
            prompt.is_some(),
            "cas-c790: WorkerIdle for a legitimate worker must still produce a prompt. \
             Got: None"
        );
    }

    // ── cas-efc4: Heterogeneous Claude+Codex smoke regression tests ───────────
    //
    // Verifies that `generate_prompt` routes MCP tool prefixes correctly when the
    // supervisor and worker use different CLI harnesses (AC3 + AC5).  All
    // homogeneous tests above use codex()+codex() or claude()+claude(); these
    // tests specifically exercise the mixed-harness surfaces identified in the
    // cas-efc4 scope: director assignment hints (cas-dbbb), harness-aware tool
    // aliases in prompts (cas-8aaf at the prompt layer), and stale-guidance
    // suppression for idle/completed events (cas-6aaf).

    /// cas-efc4 AC3 / cas-dbbb: TaskAssigned to a Codex worker from a Claude
    /// supervisor.  The prompt is sent TO the worker, so it must use the
    /// worker's MCP prefix (`mcp__cs__`).  The response instruction appended at
    /// the end must also use the Codex prefix so the worker can reply.
    #[test]
    fn test_efc4_task_assigned_codex_worker_claude_supervisor_uses_worker_prefix() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "cas-efc4-t1".to_string(),
            task_title: "Smoke test task".to_string(),
            worker: "codex-worker".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        // Claude supervisor, Codex worker
        let prompt = generate_prompt(&event, &data, "supervisor", &config, claude(), codex())
            .expect("TaskAssigned must produce a prompt");

        assert_eq!(
            prompt.target, "codex-worker",
            "cas-efc4 AC3: prompt must target the Codex worker"
        );
        assert!(
            prompt.text.contains("mcp__cs__task action=show"),
            "cas-efc4 AC3: show command must use Codex prefix mcp__cs__: {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("mcp__cs__task action=start"),
            "cas-efc4 AC3: start command must use Codex prefix mcp__cs__: {}",
            prompt.text
        );
        // Response instruction: Codex worker replies to Claude supervisor using
        // its own coordination tool.
        assert!(
            prompt.text.contains("mcp__cs__coordination action=message"),
            "cas-efc4 AC3: response instruction must use Codex coordination tool: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("mcp__cas__task action=start"),
            "cas-efc4 AC3: must NOT leak Claude prefix into Codex worker prompt: {}",
            prompt.text
        );
    }

    /// cas-efc4 AC3 (other direction): TaskAssigned to a Claude worker from a
    /// Codex supervisor.  Worker tools must be `mcp__cas__`, NOT `mcp__cs__`.
    #[test]
    fn test_efc4_task_assigned_claude_worker_codex_supervisor_uses_cas_prefix() {
        let event = DirectorEvent::TaskAssigned {
            task_id: "cas-efc4-t2".to_string(),
            task_title: "Another smoke task".to_string(),
            worker: "claude-worker".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        // Codex supervisor, Claude worker
        let prompt = generate_prompt(&event, &data, "supervisor", &config, codex(), claude())
            .expect("TaskAssigned must produce a prompt");

        assert_eq!(
            prompt.target, "claude-worker",
            "cas-efc4 AC3: prompt must target the Claude worker"
        );
        assert!(
            prompt.text.contains("mcp__cas__task action=start"),
            "cas-efc4 AC3: start command must use Claude prefix mcp__cas__: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("mcp__cs__task action=start"),
            "cas-efc4 AC3: must NOT use Codex prefix for Claude worker: {}",
            prompt.text
        );
        assert!(
            prompt
                .text
                .contains("mcp__cas__coordination action=message"),
            "cas-efc4 AC3: response instruction must use Claude coordination tool: {}",
            prompt.text
        );
    }

    /// cas-efc4 AC5 / cas-8aaf (prompt layer): TaskCompleted for a Codex worker
    /// reported to a Claude supervisor.
    ///
    /// cas-6aaf added state-aware routing for TaskCompleted:
    ///   - Task already closed (not in ready/in_progress) → "Worker has closed" path,
    ///     NO close instruction in body.  Regression guard: supervisor must NOT be
    ///     told to re-close a task the worker already closed.
    ///   - Task regressed to Open (lease expired) → "ask worker to close" path,
    ///     close instruction uses the worker's prefix (mcp__cs__task for Codex).
    ///
    /// The response-instruction footer always uses the supervisor's own prefix
    /// (mcp__cas__coordination for Claude supervisor) because it tells the
    /// RECIPIENT how to reply — the recipient always uses their own tools.
    ///
    /// Two sub-tests cover both branches.

    /// cas-efc4 AC5 normal (closed) path: TaskCompleted when task is already
    /// closed must NOT emit a close instruction. Verifies cas-6aaf stale-guidance
    /// suppression in the heterogeneous case (Claude sup + Codex worker).
    #[test]
    fn test_efc4_task_completed_already_closed_no_stale_close_instruction() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "cas-efc4-t3".to_string(),
            task_title: "Done task".to_string(),
            worker: "codex-worker".to_string(),
        };
        // Task absent from both ready_tasks and in_progress_tasks → "already closed"
        let data = make_data(0);
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, claude(), codex())
            .expect("TaskCompleted (closed path) must produce a prompt");

        assert_eq!(
            prompt.target, "supervisor",
            "cas-efc4 AC5: TaskCompleted prompt goes to supervisor"
        );
        // cas-6aaf: stale-guidance suppression — no "please close" for already-closed task
        assert!(
            !prompt.text.contains("action=close"),
            "cas-efc4 / cas-6aaf: already-closed path must NOT emit a close instruction: {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("closed"),
            "cas-efc4: prompt must confirm the task is already closed: {}",
            prompt.text
        );
        // Response instruction: supervisor (Claude) uses its own coordination tool
        assert!(
            prompt
                .text
                .contains("mcp__cas__coordination action=message"),
            "cas-efc4 AC5: response instruction must use Claude supervisor prefix: {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("target=codex-worker"),
            "cas-efc4 AC5: response instruction must address the Codex worker: {}",
            prompt.text
        );
    }

    /// cas-efc4 AC5 regressed-to-Open path: TaskCompleted when the task regressed
    /// to Open (lease expired) must emit a close instruction using the WORKER's
    /// prefix (mcp__cs__task for a Codex worker). Verifies heterogeneous prefix
    /// routing for the recovery branch.
    #[test]
    fn test_efc4_task_completed_regressed_open_close_uses_worker_prefix() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "cas-efc4-t3".to_string(),
            task_title: "Done task".to_string(),
            worker: "codex-worker".to_string(),
        };
        // Put the task into ready_tasks as Open to trigger the "regressed" branch.
        let mut data = make_data(0);
        data.ready_tasks.push(TaskSummary {
            id: "cas-efc4-t3".to_string(),
            title: "Done task".to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: None,
            task_type: cas_types::TaskType::Task,
            epic: None,
            branch: None,
            updated_at: None,
        });
        let config = default_config();

        let prompt = generate_prompt(&event, &data, "supervisor", &config, claude(), codex())
            .expect("TaskCompleted (regressed) must produce a prompt");

        assert_eq!(
            prompt.target, "supervisor",
            "cas-efc4 AC5: TaskCompleted (regressed) prompt goes to supervisor"
        );
        // Close instruction uses the worker's prefix (Codex → mcp__cs__)
        assert!(
            prompt.text.contains("mcp__cs__task action=close"),
            "cas-efc4 AC5: close instruction must use Codex worker prefix mcp__cs__: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("mcp__cas__task action=close"),
            "cas-efc4 AC5: close instruction must NOT use Claude prefix for Codex worker: {}",
            prompt.text
        );
        // Response instruction: supervisor (Claude) uses its own coordination tool
        assert!(
            prompt
                .text
                .contains("mcp__cas__coordination action=message"),
            "cas-efc4 AC5: response instruction must use Claude supervisor prefix: {}",
            prompt.text
        );
    }

    /// cas-efc4 AC5 / cas-dbbb: WorkerIdle for a Codex worker with a Claude
    /// supervisor.
    ///
    /// Prefix routing in the heterogeneous case:
    /// - Body commands address the SUPERVISOR's actions (assigning tasks, checking
    ///   ready queue) → `supervisor_prefix` = `mcp__cas__` (Claude).
    /// - Response instruction tells the SUPERVISOR how to reply → `supervisor_cli`
    ///   = Claude → `mcp__cas__coordination`.
    /// - assignee= uses the worker's display name (cas-dbbb); the live session
    ///   ID lookup still gates whether the prompt fires at all.
    #[test]
    fn test_efc4_worker_idle_codex_worker_claude_supervisor_prefixes() {
        let event = DirectorEvent::WorkerIdle {
            worker: "codex-worker".to_string(),
        };
        // 2 ready tasks so the "ready tasks exist" branch fires (non-empty assign cmd).
        let mut data = make_data(2);
        data.agents = vec![AgentSummary {
            id: "sess-id-codex-worker".to_string(),
            name: "codex-worker".to_string(),
            status: AgentStatus::Active,
            current_task: None,
            latest_activity: None,
            last_heartbeat: Some(chrono::Utc::now()),
            pending_messages: 0,
        }];
        data.agent_id_to_name = [(
            "sess-id-codex-worker".to_string(),
            "codex-worker".to_string(),
        )]
        .into_iter()
        .collect();
        let config = default_config();

        // Claude supervisor, Codex worker
        let prompt = generate_prompt(&event, &data, "supervisor", &config, claude(), codex())
            .expect("WorkerIdle must produce a prompt");

        assert_eq!(
            prompt.target, "supervisor",
            "cas-efc4 AC5: WorkerIdle prompt goes to the supervisor"
        );
        // Assign command uses supervisor's prefix (Claude supervisor acts)
        assert!(
            prompt.text.contains("mcp__cas__task action=update"),
            "cas-efc4 AC5: assign command must use Claude supervisor prefix: {}",
            prompt.text
        );
        // Ready-check uses supervisor's prefix
        assert!(
            prompt.text.contains("mcp__cas__task action=ready"),
            "cas-efc4 AC5: ready-check must use Claude supervisor prefix: {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("assignee=codex-worker"),
            "cas-efc4: assignee must use worker display name: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("assignee=sess-id-codex-worker"),
            "cas-efc4: assignee must not use the worker session ID: {}",
            prompt.text
        );
        // Response instruction: supervisor (Claude) uses its own tool to reply
        assert!(
            prompt
                .text
                .contains("mcp__cas__coordination action=message"),
            "cas-efc4 AC5: response instruction (to supervisor) must use Claude coordination prefix: {}",
            prompt.text
        );
        assert!(
            !prompt.text.contains("mcp__cs__task action=update"),
            "cas-efc4 AC5: body assign command must NOT use Codex prefix (supervisor acts): {}",
            prompt.text
        );
        assert!(
            prompt.text.contains("target=codex-worker"),
            "cas-efc4 AC5: response instruction must address the Codex worker: {}",
            prompt.text
        );
    }
}
