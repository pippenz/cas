//! Centralized task lifecycle → owning-supervisor push (cas-062d).
//!
//! Single transition-to-event seam so start / blocked / ready / close-rejected /
//! awaiting-merge / closed cannot drift. Events are durable in
//! `supervisor_queue` (idempotent by transition identity) and also enqueued to
//! `prompt_queue` for factory-session delivery to the supervisor pane.

use chrono::Utc;
use serde_json::json;

use cas_store::{
    AgentStore, NotificationPriority, NotifyIdempotentResult, PromptQueueStore,
    SupervisorQueueStore,
};
use cas_types::{AgentRole, TaskStatus};

use crate::mcp::server::CasCore;
use crate::store::{open_prompt_queue_store, open_supervisor_queue_store};

/// Named lifecycle transitions that must push to the owning supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTransition {
    Started,
    Blocked,
    ReadyReopened,
    CloseRejected,
    AwaitingMerge,
    Closed,
}

impl LifecycleTransition {
    pub fn as_event_type(self) -> &'static str {
        match self {
            Self::Started => "task_started",
            Self::Blocked => "task_blocked",
            Self::ReadyReopened => "task_ready",
            Self::CloseRejected => "task_close_rejected",
            Self::AwaitingMerge => "task_awaiting_merge",
            Self::Closed => "task_closed",
        }
    }

    pub fn priority(self) -> NotificationPriority {
        match self {
            Self::CloseRejected | Self::Blocked | Self::AwaitingMerge => NotificationPriority::High,
            Self::Started | Self::ReadyReopened | Self::Closed => NotificationPriority::Normal,
        }
    }
}

/// Result of a lifecycle push attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecyclePushResult {
    /// New durable event enqueued (supervisor_queue id).
    Enqueued { notification_id: i64 },
    /// Same transition_key already present — no duplicate row.
    DuplicateSuppressed { notification_id: i64 },
    /// No owning supervisor found for the factory session (non-factory or empty).
    NoSupervisor,
}

/// Build stable transition identity for idempotency.
///
/// Includes factory_session so the same task transition in two concurrent
/// factories never collides or leaks.
pub fn transition_key(
    task_id: &str,
    old_status: TaskStatus,
    new_status: TaskStatus,
    factory_session: Option<&str>,
    kind: LifecycleTransition,
) -> String {
    format!(
        "{task_id}:{old_status}:{new_status}:{}:{}",
        factory_session.unwrap_or(""),
        kind.as_event_type()
    )
}

/// Owning supervisor identity for lifecycle push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwningSupervisor {
    /// Agent id used as `supervisor_queue.supervisor_id` (matches worker_died /
    /// queue_poll conventions — supervisors poll by agent id).
    pub agent_id: String,
    /// Display/pane name (for payloads and diagnostics).
    pub name: String,
}

/// Resolve the owning supervisor for a factory session.
///
/// Session isolation: only agents with `role == Supervisor` and matching
/// `factory_session` (via [`Agent::visible_to_factory_session`]) are considered.
/// Prefers Active/Idle over Stale/Shutdown, then stable name order.
pub fn resolve_owning_supervisor(
    agent_store: &dyn AgentStore,
    factory_session: Option<&str>,
) -> Option<OwningSupervisor> {
    let agents = agent_store.list(None).ok()?;
    let mut candidates: Vec<_> = agents
        .into_iter()
        .filter(|a| a.role == AgentRole::Supervisor)
        .filter(|a| a.visible_to_factory_session(factory_session))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    // Prefer Active/Idle over stale; then stable name order.
    candidates.sort_by(|a, b| {
        use cas_types::AgentStatus;
        let rank = |s: &AgentStatus| match s {
            AgentStatus::Active => 0,
            AgentStatus::Idle => 1,
            _ => 2,
        };
        rank(&a.status)
            .cmp(&rank(&b.status))
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.id.cmp(&b.id))
    });
    let sup = &candidates[0];
    Some(OwningSupervisor {
        agent_id: sup.id.clone(),
        name: if sup.name.is_empty() {
            sup.id.clone()
        } else {
            sup.name.clone()
        },
    })
}

/// Emit one lifecycle transition to the owning supervisor.
///
/// - **Durable:** `supervisor_queue.notify_idempotent` (fails hard if queue write fails)
/// - **Deliverable:** best-effort `prompt_queue` enqueue to supervisor (session-tagged)
///   so the factory daemon can inject without free-form worker prose.
///
/// Task mutation must already have succeeded. Callers must surface queue-write
/// errors and must not claim a successful push when this returns `Err`.
#[allow(clippy::too_many_arguments)]
pub fn emit_task_lifecycle_transition(
    supervisor_queue: &dyn SupervisorQueueStore,
    prompt_queue: Option<&dyn PromptQueueStore>,
    agent_store: &dyn AgentStore,
    task_id: &str,
    task_title: &str,
    old_status: TaskStatus,
    new_status: TaskStatus,
    actor: &str,
    reason: Option<&str>,
    kind: LifecycleTransition,
) -> Result<LifecyclePushResult, String> {
    let factory_session = std::env::var("CAS_FACTORY_SESSION").ok();
    let Some(supervisor) = resolve_owning_supervisor(agent_store, factory_session.as_deref())
    else {
        return Ok(LifecyclePushResult::NoSupervisor);
    };

    let key = transition_key(
        task_id,
        old_status,
        new_status,
        factory_session.as_deref(),
        kind,
    );
    let now = Utc::now();
    let payload = json!({
        "task_id": task_id,
        "title": task_title,
        "old_status": old_status.to_string(),
        "new_status": new_status.to_string(),
        "actor": actor,
        "reason": reason,
        "transition": kind.as_event_type(),
        "factory_session": factory_session,
        "supervisor_id": supervisor.agent_id,
        "supervisor_name": supervisor.name,
        "timestamp": now.to_rfc3339(),
    })
    .to_string();

    // Durable path keys by agent id so queue_poll / worker_died conventions match.
    let result = supervisor_queue
        .notify_idempotent(
            &supervisor.agent_id,
            "task_lifecycle",
            &payload,
            kind.priority(),
            &key,
        )
        .map_err(|e| format!("supervisor_queue write failed: {e}"))?;

    let notification_id = match result {
        NotifyIdempotentResult::Created(id) => id,
        NotifyIdempotentResult::AlreadyExists(id) => {
            return Ok(LifecyclePushResult::DuplicateSuppressed {
                notification_id: id,
            });
        }
    };

    // Delivery path: structured coordination message to supervisor pane.
    if let Some(pq) = prompt_queue {
        let body = format!(
            "<task-lifecycle transition=\"{}\" task_id=\"{}\" old=\"{}\" new=\"{}\" actor=\"{}\" notification_id=\"{}\">\n\
             Task {} — {}\n\
             {}{}\
             </task-lifecycle>",
            kind.as_event_type(),
            task_id,
            old_status,
            new_status,
            actor,
            notification_id,
            task_id,
            task_title,
            reason.map(|r| format!("Reason: {r}\n")).unwrap_or_default(),
            factory_session
                .as_deref()
                .map(|s| format!("Session: {s}\n"))
                .unwrap_or_default(),
        );
        let summary = format!("{}: {}", kind.as_event_type(), task_id);
        if let Some(ref sess) = factory_session {
            pq.enqueue_with_summary(
                actor,
                "supervisor",
                &body,
                Some(sess.as_str()),
                Some(&summary),
            )
            .map_err(|e| format!("prompt_queue write failed after durable enqueue: {e}"))?;
        } else {
            pq.enqueue_with_summary(actor, "supervisor", &body, None, Some(&summary))
                .map_err(|e| format!("prompt_queue write failed after durable enqueue: {e}"))?;
        }
    }

    Ok(LifecyclePushResult::Enqueued { notification_id })
}

impl CasCore {
    /// Push a lifecycle transition after a successful task mutation (cas-062d).
    ///
    /// Returns `Ok(None)` when no factory supervisor is present. Returns `Err`
    /// when the durable supervisor_queue write fails — callers must surface it
    /// and must not claim the push succeeded.
    pub(crate) fn push_task_lifecycle(
        &self,
        task_id: &str,
        task_title: &str,
        old_status: TaskStatus,
        new_status: TaskStatus,
        actor: &str,
        reason: Option<&str>,
        kind: LifecycleTransition,
    ) -> Result<Option<LifecyclePushResult>, String> {
        let agent_store = self
            .open_agent_store()
            .map_err(|e| format!("agent store: {e}"))?;
        let sq = open_supervisor_queue_store(&self.cas_root)
            .map_err(|e| format!("supervisor_queue open: {e}"))?;
        let pq = open_prompt_queue_store(&self.cas_root).ok();
        let result = emit_task_lifecycle_transition(
            sq.as_ref(),
            pq.as_ref().map(|a| a.as_ref() as &dyn PromptQueueStore),
            agent_store.as_ref(),
            task_id,
            task_title,
            old_status,
            new_status,
            actor,
            reason,
            kind,
        )?;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_store::{SqliteAgentStore, SqliteSupervisorQueueStore, SupervisorQueueStore};
    use cas_types::{Agent, AgentRole, AgentStatus};
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialize env mutations in this module's tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn transition_key_includes_session_for_isolation() {
        let a = transition_key(
            "cas-1",
            TaskStatus::InProgress,
            TaskStatus::Closed,
            Some("sess-a"),
            LifecycleTransition::Closed,
        );
        let b = transition_key(
            "cas-1",
            TaskStatus::InProgress,
            TaskStatus::Closed,
            Some("sess-b"),
            LifecycleTransition::Closed,
        );
        assert_ne!(a, b);
        assert!(a.contains("sess-a"));
        assert!(b.contains("sess-b"));
    }

    #[test]
    fn transition_key_stable_for_same_identity() {
        let a = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
        );
        let b = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
        );
        assert_eq!(a, b);
    }

    #[test]
    fn event_types_are_stable_strings() {
        assert_eq!(LifecycleTransition::Started.as_event_type(), "task_started");
        assert_eq!(LifecycleTransition::Blocked.as_event_type(), "task_blocked");
        assert_eq!(
            LifecycleTransition::ReadyReopened.as_event_type(),
            "task_ready"
        );
        assert_eq!(
            LifecycleTransition::CloseRejected.as_event_type(),
            "task_close_rejected"
        );
        assert_eq!(
            LifecycleTransition::AwaitingMerge.as_event_type(),
            "task_awaiting_merge"
        );
        assert_eq!(LifecycleTransition::Closed.as_event_type(), "task_closed");
    }

    fn agent_in_session(id: &str, name: &str, role: AgentRole, session: &str) -> Agent {
        let mut a = Agent::new(id.to_string(), name.to_string());
        a.role = role;
        a.status = AgentStatus::Active;
        a.factory_session = Some(session.to_string());
        a
    }

    #[test]
    fn resolve_owning_supervisor_session_isolation() {
        let temp = TempDir::new().unwrap();
        let agents = SqliteAgentStore::open(temp.path()).unwrap();
        agents.init().unwrap();
        agents
            .register(&agent_in_session(
                "sup-a-id",
                "sup-a",
                AgentRole::Supervisor,
                "sess-a",
            ))
            .unwrap();
        agents
            .register(&agent_in_session(
                "sup-b-id",
                "sup-b",
                AgentRole::Supervisor,
                "sess-b",
            ))
            .unwrap();
        agents
            .register(&agent_in_session(
                "worker-a",
                "worker-a",
                AgentRole::Worker,
                "sess-a",
            ))
            .unwrap();

        let a = resolve_owning_supervisor(&agents, Some("sess-a")).unwrap();
        assert_eq!(a.agent_id, "sup-a-id");
        assert_eq!(a.name, "sup-a");

        let b = resolve_owning_supervisor(&agents, Some("sess-b")).unwrap();
        assert_eq!(b.agent_id, "sup-b-id");

        assert!(resolve_owning_supervisor(&agents, Some("sess-empty")).is_none());
    }

    #[test]
    fn emit_enqueues_once_and_suppresses_duplicate() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("CAS_FACTORY_SESSION").ok();
        // SAFETY: ENV_LOCK held for this test body.
        unsafe {
            std::env::set_var("CAS_FACTORY_SESSION", "sess-emit");
        }

        let temp = TempDir::new().unwrap();
        let agents = SqliteAgentStore::open(temp.path()).unwrap();
        agents.init().unwrap();
        agents
            .register(&agent_in_session(
                "sup-emit",
                "sup-emit-name",
                AgentRole::Supervisor,
                "sess-emit",
            ))
            .unwrap();
        let sq = SqliteSupervisorQueueStore::open(temp.path()).unwrap();
        sq.init().unwrap();

        let r1 = emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-t1",
            "Title",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "worker-1",
            None,
            LifecycleTransition::Started,
        )
        .unwrap();
        let r2 = emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-t1",
            "Title",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "worker-1",
            None,
            LifecycleTransition::Started,
        )
        .unwrap();

        match (r1, r2) {
            (
                LifecyclePushResult::Enqueued {
                    notification_id: id1,
                },
                LifecyclePushResult::DuplicateSuppressed {
                    notification_id: id2,
                },
            ) => assert_eq!(id1, id2),
            other => panic!("expected Enqueued then DuplicateSuppressed, got {other:?}"),
        }
        assert_eq!(sq.pending_count("sup-emit").unwrap(), 1);
        let pending = sq.peek("sup-emit", 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_type, "task_lifecycle");
        assert!(pending[0].payload.contains("task_started"));
        assert_eq!(
            pending[0].transition_key.as_deref(),
            Some("cas-t1:open:in_progress:sess-emit:task_started")
        );

        // SAFETY: restore env under ENV_LOCK.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("CAS_FACTORY_SESSION", v),
                None => std::env::remove_var("CAS_FACTORY_SESSION"),
            }
        }
    }

    #[test]
    fn emit_does_not_cross_factory_sessions() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("CAS_FACTORY_SESSION").ok();
        unsafe {
            std::env::set_var("CAS_FACTORY_SESSION", "sess-a");
        }

        let temp = TempDir::new().unwrap();
        let agents = SqliteAgentStore::open(temp.path()).unwrap();
        agents.init().unwrap();
        agents
            .register(&agent_in_session(
                "sup-a",
                "sup-a",
                AgentRole::Supervisor,
                "sess-a",
            ))
            .unwrap();
        agents
            .register(&agent_in_session(
                "sup-b",
                "sup-b",
                AgentRole::Supervisor,
                "sess-b",
            ))
            .unwrap();
        let sq = SqliteSupervisorQueueStore::open(temp.path()).unwrap();
        sq.init().unwrap();

        emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-x",
            "X",
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            "worker",
            Some("waiting"),
            LifecycleTransition::Blocked,
        )
        .unwrap();

        assert_eq!(sq.pending_count("sup-a").unwrap(), 1);
        assert_eq!(sq.pending_count("sup-b").unwrap(), 0);

        unsafe {
            match prior {
                Some(v) => std::env::set_var("CAS_FACTORY_SESSION", v),
                None => std::env::remove_var("CAS_FACTORY_SESSION"),
            }
        }
    }
}
