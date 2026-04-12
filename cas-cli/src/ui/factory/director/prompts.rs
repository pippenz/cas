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
/// Closed tasks never appear in `ready_tasks` at all, but this count is used
/// in the `WorkerIdle` / `AgentRegistered` prompts to tell the supervisor how
/// many tasks are available — if we reported the raw length the supervisor
/// would be told "there are N ready tasks" and then find nothing to assign
/// when N of them are actually blocked. Count only `Open` and only tasks
/// without an assignee already set. See cas-177f.
fn dispatchable_ready_count(data: &DirectorData) -> usize {
    data.ready_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Open && t.assignee.is_none())
        .count()
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

            let text = format!(
                "Worker {worker} has completed task {task_id} ({task_title}).\n\n\
                 Next steps:\n\
                 - Tell the worker to close their own task: {worker_prefix}task action=close id={task_id}\n\
                 - If close triggers verification, the worker handles it (not you)\n\
                 - Then assign another task to this worker, OR\n\
                 - If all subtasks are done, YOU verify and close the epic\n\n\
                 Remember: workers close their own tasks, supervisors close epics."
            );

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

            // Count only truly-dispatchable tasks (Open + unassigned). See
            // `dispatchable_ready_count` for why `ready_tasks.len()` is wrong.
            let ready_count = dispatchable_ready_count(data);

            let text = if ready_count > 0 {
                format!(
                    "Worker {worker} is idle with no assigned tasks.\n\
                     There are {ready_count} ready tasks available.\n\
                     Assign work: {supervisor_prefix}task action=update id=<task-id> assignee={worker}"
                )
            } else {
                format!(
                    "Worker {worker} is idle with no assigned tasks.\n\
                     No ready tasks are available. Consider creating new tasks or closing the epic."
                )
            };

            Some(Prompt {
                target: supervisor_name.to_string(),
                text: with_response_instructions(&text, worker, supervisor_cli),
            })
        }

        DirectorEvent::AgentRegistered { agent_name, .. } => {
            if !config.on_worker_ready {
                return None;
            }

            // Don't notify about supervisor registering
            if agent_name == supervisor_name {
                return None;
            }

            let ready_count = dispatchable_ready_count(data);

            let text = if ready_count > 0 {
                format!(
                    "Worker {agent_name} is ready and waiting for tasks.\n\
                     There are {ready_count} ready tasks available.\n\
                     Assign work: {supervisor_prefix}task action=update id=<task-id> assignee={agent_name}"
                )
            } else {
                format!(
                    "Worker {agent_name} is ready and waiting for tasks.\n\
                     No ready tasks are available yet."
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
                "🎉 All subtasks of epic '{epic_title}' ({epic_id}) are now closed!\n\n\
                 Next steps:\n\
                 - Cherry-pick worker commits to main\n\
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
    use crate::ui::factory::director::data::TaskSummary;
    use crate::ui::factory::director::prompts::*;
    use cas_mux::SupervisorCli;
    use cas_types::{Priority, TaskStatus, TaskType};
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
            })
            .collect();

        DirectorData {
            ready_tasks,
            in_progress_tasks: vec![],
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

    #[test]
    fn test_task_completed_prompt() {
        let event = DirectorEvent::TaskCompleted {
            task_id: "task-123".to_string(),
            task_title: "Implement feature X".to_string(),
            worker: "swift-fox".to_string(),
        };
        let data = make_data(0);
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("swift-fox"));
        assert!(prompt.text.contains("task-123"));
        assert!(prompt.text.contains("completed"));
        // Should clarify verification ownership
        assert!(prompt.text.contains("workers close their own tasks"));
        assert!(prompt.text.contains("supervisors close epics"));
        // Response instructions should point to the worker
        assert!(prompt.text.contains("To respond to this message, use:"));
        assert!(prompt.text.contains("target=swift-fox"));
    }

    #[test]
    fn test_worker_idle_with_ready_tasks() {
        let event = DirectorEvent::WorkerIdle {
            worker: "swift-fox".to_string(),
        };
        let data = make_data(3); // 3 ready tasks
        let config = default_config();

        let prompt =
            generate_prompt(&event, &data, "supervisor", &config, codex(), codex()).unwrap();

        assert_eq!(prompt.target, "supervisor");
        assert!(prompt.text.contains("swift-fox"));
        assert!(prompt.text.contains("idle"));
        assert!(prompt.text.contains("3 ready tasks"));
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
        assert!(prompt.text.contains("No ready tasks"));
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
        assert!(prompt.text.contains("3 ready tasks"));
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
        assert!(prompt.text.contains("No ready tasks"));
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
}
