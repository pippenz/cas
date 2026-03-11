//! Shared helpers for agent status display across factory panels.
//!
//! Consolidates heartbeat detection, status icon/color resolution,
//! agent counting, and task lookup logic used by factory_radar,
//! mission_workers, and mission_epic.

use cas_types::{AgentStatus, TaskStatus};
use chrono::Utc;
use ratatui::prelude::Color;

use super::data::DirectorData;
use crate::ui::theme::{Icons, Palette};

/// Agents with no heartbeat for this many seconds are considered disconnected.
pub const HEARTBEAT_TIMEOUT_SECS: i64 = 300;

/// Returns the number of seconds since the agent's last heartbeat,
/// or `i64::MAX` if no heartbeat has been recorded.
pub fn heartbeat_age_secs(agent: &cas_factory::AgentSummary) -> i64 {
    agent
        .last_heartbeat
        .map(|hb| Utc::now().signed_duration_since(hb).num_seconds())
        .unwrap_or(i64::MAX)
}

/// Whether the agent should be treated as disconnected.
pub fn is_disconnected(agent: &cas_factory::AgentSummary) -> bool {
    heartbeat_age_secs(agent) > HEARTBEAT_TIMEOUT_SECS
}

/// Resolve the status icon string and color for an agent.
///
/// Takes heartbeat into account: disconnected agents always show the dead icon
/// regardless of their reported status.
pub fn agent_status_icon(
    agent: &cas_factory::AgentSummary,
    palette: &Palette,
) -> (&'static str, Color) {
    if is_disconnected(agent) {
        ("\u{2298}", palette.agent_dead) // ⊘
    } else {
        match agent.status {
            AgentStatus::Active => (Icons::CIRCLE_FILLED, palette.agent_active),
            AgentStatus::Idle => (Icons::CIRCLE_HALF, palette.agent_idle),
            _ => (Icons::CIRCLE_EMPTY, palette.agent_dead),
        }
    }
}

/// Resolve agent status icon using simple Unicode characters (for compact views).
pub fn agent_status_icon_simple(
    agent: &cas_factory::AgentSummary,
    palette: &Palette,
) -> (&'static str, Color) {
    match agent.status {
        AgentStatus::Active => ("\u{25cf}", palette.agent_active), // ●
        AgentStatus::Idle => ("\u{25cb}", palette.agent_idle),     // ○
        _ => ("\u{2298}", palette.agent_dead),                     // ⊘
    }
}

/// Heartbeat-aware agent status counts.
#[derive(Debug, Default)]
pub struct AgentStatusCounts {
    pub active: usize,
    pub idle: usize,
    pub dead: usize,
}

/// Count agents by effective status, treating disconnected agents as dead.
pub fn count_agent_statuses(agents: &[cas_factory::AgentSummary]) -> AgentStatusCounts {
    let mut counts = AgentStatusCounts::default();
    for agent in agents {
        if is_disconnected(agent) {
            counts.dead += 1;
        } else {
            match agent.status {
                AgentStatus::Active => counts.active += 1,
                AgentStatus::Idle => counts.idle += 1,
                _ => counts.dead += 1,
            }
        }
    }
    counts
}

/// Find the task currently assigned to an agent.
///
/// Searches in_progress_tasks first, then ready_tasks.
pub fn find_agent_task<'a>(
    agent: &cas_factory::AgentSummary,
    data: &'a DirectorData,
) -> Option<&'a cas_factory::TaskSummary> {
    let task_id = agent.current_task.as_ref()?;
    data.in_progress_tasks
        .iter()
        .chain(data.ready_tasks.iter())
        .find(|t| &t.id == task_id)
}

/// Map a task status to a display icon.
pub fn task_status_icon(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::InProgress => Icons::SPINNER_STATIC,
        TaskStatus::Blocked => Icons::BLOCKED,
        _ => "",
    }
}
