//! Orphan recovery when a worker vanishes mid-task (cas-2e81).
//!
//! `mark_stale` / lease reclaim revoke leases but historically left the task
//! row as `InProgress` with no supervisor signal. This module parks eligible
//! tasks Open with an audit note, records a `WorkerDied` event, and queues a
//! critical `worker_died` notification for active supervisors.

use std::path::Path;
use std::sync::Arc;

use cas_types::{
    Agent, AgentRole, AgentStatus, Event, EventEntityType, EventType, TaskStatus,
};
use chrono::Utc;

use crate::store::{
    AgentStore, NotificationPriority, TaskStore, open_event_store, open_supervisor_queue_store,
    open_task_store,
};

/// Summary of a single recovery pass.
#[derive(Debug, Default, Clone)]
pub struct OrphanRecoverySummary {
    pub recovered_task_ids: Vec<String>,
    pub held_task_ids: Vec<String>,
}

/// Statuses that must NOT be auto-parked (cas-6e4c PSR + merge-gate work).
fn is_protected_status(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Closed
            | TaskStatus::Open
            | TaskStatus::PendingSupervisorReview
            | TaskStatus::AwaitingMerge
    )
}

/// Park orphaned working tasks for a dead agent and emit supervisor signals.
///
/// `held_task_ids` should be the leases observed *before* `mark_stale` /
/// reclaim (those calls clear active leases). Also recovers InProgress/
/// Blocked tasks still assigned to the agent by name or id.
pub fn recover_worker_vanished(
    cas_root: &Path,
    agent_store: &dyn AgentStore,
    agent: &Agent,
    held_task_ids: &[String],
    reason: &str,
) -> OrphanRecoverySummary {
    let mut summary = OrphanRecoverySummary {
        held_task_ids: held_task_ids.to_vec(),
        ..Default::default()
    };

    let task_store = match open_task_store(cas_root) {
        Ok(s) => s,
        Err(_) => {
            emit_worker_died_signals(cas_root, agent_store, agent, &summary, reason);
            return summary;
        }
    };

    let mut candidate_ids: Vec<String> = held_task_ids.to_vec();

    // Also pick up tasks still assigned to this worker whose lease may already
    // have been reclaimed earlier without recovery.
    if let Ok(in_progress) = task_store.list(Some(TaskStatus::InProgress)) {
        for t in in_progress {
            if task_assigned_to_agent(&t.assignee, agent) {
                candidate_ids.push(t.id);
            }
        }
    }
    if let Ok(blocked) = task_store.list(Some(TaskStatus::Blocked)) {
        for t in blocked {
            if task_assigned_to_agent(&t.assignee, agent) {
                candidate_ids.push(t.id);
            }
        }
    }
    candidate_ids.sort();
    candidate_ids.dedup();
    summary.held_task_ids = candidate_ids.clone();

    for task_id in &candidate_ids {
        if park_orphaned_task(&task_store, task_id, agent, reason) {
            summary.recovered_task_ids.push(task_id.clone());
        }
    }

    emit_worker_died_signals(cas_root, agent_store, agent, &summary, reason);
    summary
}

/// Recover tasks whose leases just expired, but only when the holder is dead
/// or heartbeat-stale (worker vanished) — not when a live agent simply failed
/// to renew mid-turn.
pub fn recover_expired_leases_for_dead_holders(
    cas_root: &Path,
    agent_store: &dyn AgentStore,
    expired: &[(String, String)], // (task_id, agent_id)
    stale_threshold_secs: i64,
) -> Vec<OrphanRecoverySummary> {
    use std::collections::HashMap;

    let mut by_agent: HashMap<String, Vec<String>> = HashMap::new();
    for (task_id, agent_id) in expired {
        by_agent
            .entry(agent_id.clone())
            .or_default()
            .push(task_id.clone());
    }

    let mut out = Vec::new();
    for (agent_id, task_ids) in by_agent {
        let agent = match agent_store.get(&agent_id) {
            Ok(a) => a,
            Err(_) => continue,
        };
        if holder_is_alive(&agent, stale_threshold_secs) {
            continue;
        }
        let summary = recover_worker_vanished(
            cas_root,
            agent_store,
            &agent,
            &task_ids,
            "lease expired while holder gone",
        );
        out.push(summary);
    }
    out
}

fn holder_is_alive(agent: &Agent, stale_threshold_secs: i64) -> bool {
    if !matches!(agent.status, AgentStatus::Active | AgentStatus::Idle) {
        return false;
    }
    let elapsed = (Utc::now() - agent.last_heartbeat).num_seconds();
    elapsed <= stale_threshold_secs
}

fn task_assigned_to_agent(assignee: &Option<String>, agent: &Agent) -> bool {
    match assignee {
        Some(a) => a == &agent.name || a == &agent.id,
        None => false,
    }
}

/// Returns true if the task was parked to Open.
fn park_orphaned_task(
    task_store: &Arc<dyn TaskStore>,
    task_id: &str,
    agent: &Agent,
    reason: &str,
) -> bool {
    let mut task = match task_store.get(task_id) {
        Ok(t) => t,
        Err(_) => return false,
    };
    if is_protected_status(task.status) {
        return false;
    }

    let prior_status = task.status;
    let prior_assignee = task.assignee.clone();
    task.status = TaskStatus::Open;
    task.assignee = None;
    let ts = Utc::now().format("%Y-%m-%d %H:%M");
    let audit = format!(
        "[{ts}] ⚠ orphan recovery: worker vanished mid-task — parked {prior_status:?}→Open \
         (prior assignee: {}, worker: {} / {}, reason: {reason}). \
         Use `task action=reset` only if still stuck; task is now claimable.",
        prior_assignee.as_deref().unwrap_or("<none>"),
        agent.name,
        &agent.id[..8.min(agent.id.len())],
    );
    task.notes = if task.notes.is_empty() {
        audit
    } else {
        format!("{}\n\n{}", task.notes, audit)
    };
    task.updated_at = Utc::now();
    task_store.update(&task).is_ok()
}

fn emit_worker_died_signals(
    cas_root: &Path,
    agent_store: &dyn AgentStore,
    agent: &Agent,
    summary: &OrphanRecoverySummary,
    reason: &str,
) {
    let held = summary.held_task_ids.join(",");
    let recovered = summary.recovered_task_ids.join(",");
    let payload = serde_json::json!({
        "worker_id": agent.id,
        "worker_name": agent.name,
        "held_tasks": summary.held_task_ids,
        "recovered_tasks": summary.recovered_task_ids,
        "reason": reason,
        "last_heartbeat": agent.last_heartbeat.to_rfc3339(),
        "factory_session": agent.factory_session,
    });

    // Activity feed event.
    if let Ok(event_store) = open_event_store(cas_root) {
        let summary_text = if summary.held_task_ids.is_empty() {
            format!(
                "Worker {} died ({reason}); no held tasks",
                agent.name
            )
        } else {
            format!(
                "Worker {} died mid-task ({reason}); held=[{held}]; recovered=[{recovered}]",
                agent.name
            )
        };
        let event = Event::new(
            EventType::WorkerDied,
            EventEntityType::Agent,
            agent.id.clone(),
            summary_text,
        )
        .with_metadata(payload.clone())
        .with_session(agent.id.clone());
        let _ = event_store.record(&event);
    }

    // Supervisor queue — critical priority.
    let supervisors = match agent_store.list(None) {
        Ok(agents) => agents
            .into_iter()
            .filter(|a| {
                matches!(a.role, AgentRole::Supervisor | AgentRole::Director)
                    && matches!(a.status, AgentStatus::Active | AgentStatus::Idle)
                    && a.visible_to_factory_session(agent.factory_session.as_deref())
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };

    if let Ok(queue) = open_supervisor_queue_store(cas_root) {
        let payload_str = payload.to_string();
        for sup in &supervisors {
            let _ = queue.notify(
                &sup.id,
                "worker_died",
                &payload_str,
                NotificationPriority::Critical,
            );
        }
        // If no live supervisor rows, still notify parent_id when set.
        if supervisors.is_empty() {
            if let Some(ref parent) = agent.parent_id {
                let _ = queue.notify(
                    parent,
                    "worker_died",
                    &payload_str,
                    NotificationPriority::Critical,
                );
            }
        }
    }
}

/// Format the "Recently died while leased" section for worker_status.
///
/// Pulls recent WorkerDied events (last `window_secs`) and any still-stale
/// workers that held leases at death. Returns empty string when nothing to show.
pub fn format_recently_died_while_leased(
    cas_root: &Path,
    agent_store: &dyn AgentStore,
    factory_session: Option<&str>,
    window_secs: i64,
) -> String {
    let cutoff = Utc::now() - chrono::Duration::seconds(window_secs);
    let mut lines: Vec<String> = Vec::new();

    // Prefer structured WorkerDied events.
    if let Ok(event_store) = open_event_store(cas_root) {
        if let Ok(events) = event_store.list_since(cutoff, 100) {
            for ev in events {
                if ev.event_type != EventType::WorkerDied {
                    continue;
                }
                let meta = ev.metadata.as_ref();
                let name = meta
                    .and_then(|m| m.get("worker_name"))
                    .and_then(|v| v.as_str())
                    .unwrap_or(ev.entity_id.as_str());
                let held = meta
                    .and_then(|m| m.get("held_tasks"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                if held.is_empty() {
                    // AC: died-while-leased — skip pure idle deaths.
                    continue;
                }
                let recovered = meta
                    .and_then(|m| m.get("recovered_tasks"))
                    .and_then(|v| v.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let hb = meta
                    .and_then(|m| m.get("last_heartbeat"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let elapsed = chrono::DateTime::parse_from_rfc3339(hb)
                    .map(|dt| (Utc::now() - dt.with_timezone(&Utc)).num_seconds())
                    .unwrap_or(-1);
                let since = if elapsed >= 0 {
                    format!("{elapsed}s ago")
                } else {
                    hb.to_string()
                };
                let rec_note = if recovered.is_empty() {
                    String::new()
                } else {
                    format!(" → Open (orphaned: {recovered})")
                };
                lines.push(format!(
                    "  • {name} (last heartbeat: {since}) [stale]\n    held: {held}{rec_note}"
                ));
            }
        }
    }

    // Also surface currently-stale workers that still hold InProgress assignee
    // (recovery not yet run) — defensive second signal.
    if let Ok(stale) = agent_store.list(Some(AgentStatus::Stale)) {
        if let Ok(task_store) = open_task_store(cas_root) {
            for agent in stale {
                if !agent.visible_to_factory_session(factory_session) {
                    continue;
                }
                if agent.role != AgentRole::Worker {
                    continue;
                }
                // Skip if already listed via WorkerDied event for this agent.
                if lines.iter().any(|l| l.contains(&agent.name)) {
                    continue;
                }
                let mut held = Vec::new();
                if let Ok(tasks) = task_store.list(Some(TaskStatus::InProgress)) {
                    for t in tasks {
                        if task_assigned_to_agent(&t.assignee, &agent) {
                            held.push(t.id);
                        }
                    }
                }
                if held.is_empty() {
                    continue;
                }
                let elapsed = (Utc::now() - agent.last_heartbeat).num_seconds();
                lines.push(format!(
                    "  • {} (last heartbeat: {elapsed}s ago) [stale]\n    held: {} (still InProgress — run agent_cleanup)",
                    agent.name,
                    held.join(", ")
                ));
            }
        }
    }

    if lines.is_empty() {
        return String::new();
    }
    // Dedupe by worker name line prefix.
    lines.sort();
    lines.dedup();
    format!(
        "\nRecently died while leased ({}):\n{}\n",
        lines.len(),
        lines.join("\n")
    )
}
