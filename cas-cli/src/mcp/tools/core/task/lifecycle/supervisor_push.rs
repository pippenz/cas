//! Centralized task lifecycle → owning-supervisor push (cas-062d / cas-17e4).
//!
//! Single transition-to-event seam so start / blocked / ready / close-rejected /
//! awaiting-merge / closed cannot drift. Events are durable in
//! `supervisor_queue` (idempotent by **occurrence** identity) and delivered via
//! `prompt_queue` as an outbox step: replaying an existing durable row retries
//! missing prompt delivery rather than suppressing it.

use chrono::{DateTime, Utc};
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
    /// New durable event enqueued and prompt delivery completed (or no prompt path).
    Enqueued { notification_id: i64 },
    /// Durable row already present; prompt delivery was completed (or re-stamped) without
    /// inserting a second durable event.
    Recovered { notification_id: i64 },
    /// Same occurrence fully complete (durable + prompt) — no new side effects.
    AlreadyComplete { notification_id: i64 },
    /// No owning supervisor found for the factory session (non-factory or empty).
    NoSupervisor,
}

/// Build occurrence-scoped transition identity for idempotency (cas-17e4).
///
/// Includes:
/// - factory_session so concurrent factories never collide/leak
/// - occurrence_id (typically post-mutation `task.updated_at`) so two legitimate
///   Open→InProgress cycles (start → block → ready → start) produce distinct events,
///   while retrying the *same* occurrence still dedupes
pub fn transition_key(
    task_id: &str,
    old_status: TaskStatus,
    new_status: TaskStatus,
    factory_session: Option<&str>,
    kind: LifecycleTransition,
    occurrence_id: &str,
) -> String {
    format!(
        "{task_id}:{old_status}:{new_status}:{}:{}:{occurrence_id}",
        factory_session.unwrap_or(""),
        kind.as_event_type()
    )
}

/// Format occurrence id from a post-mutation timestamp (stable for that write).
pub fn occurrence_from_updated_at(updated_at: DateTime<Utc>) -> String {
    updated_at.to_rfc3339()
}

/// Truthful repair guidance after task mutation succeeded but lifecycle push failed.
///
/// Never claims that re-running the task operation is safe — status may already
/// make that operation illegal/no-op. Names current state and the outbox replay path.
pub fn lifecycle_push_failure_message(
    task_id: &str,
    current_status: TaskStatus,
    kind: LifecycleTransition,
    transition_key: &str,
    error: &str,
) -> String {
    format!(
        "Task {task_id} is already {current_status}; supervisor lifecycle push \
         for {} failed: {error}. \
         Task state was NOT rolled back. \
         Repair: replay lifecycle outbox for transition_key={transition_key} \
         (durable event may already exist — prompt delivery will retry). \
         Do NOT re-run the original task operation solely to recover the event; \
         that operation may now be illegal or a no-op for status={current_status}.",
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

fn build_prompt_body(
    kind: LifecycleTransition,
    task_id: &str,
    task_title: &str,
    old_status: TaskStatus,
    new_status: TaskStatus,
    actor: &str,
    reason: Option<&str>,
    notification_id: i64,
    factory_session: Option<&str>,
    occurrence_id: &str,
) -> String {
    format!(
        "<task-lifecycle transition=\"{}\" task_id=\"{}\" old=\"{}\" new=\"{}\" actor=\"{}\" \
         notification_id=\"{}\" occurrence=\"{}\">\n\
         Task {} — {}\n\
         {}{}\
         </task-lifecycle>",
        kind.as_event_type(),
        task_id,
        old_status,
        new_status,
        actor,
        notification_id,
        occurrence_id,
        task_id,
        task_title,
        reason.map(|r| format!("Reason: {r}\n")).unwrap_or_default(),
        factory_session
            .map(|s| format!("Session: {s}\n"))
            .unwrap_or_default(),
    )
}

/// Emit one lifecycle transition to the owning supervisor (outbox workflow).
///
/// 1. **Durable:** `supervisor_queue.notify_idempotent` keyed by occurrence identity
/// 2. **Prompt:** if not yet marked `prompt_delivered`, enqueue to prompt_queue then stamp
///
/// Replaying the same occurrence after a prompt failure retries prompt delivery and
/// stamps delivery exactly once. Distinct occurrences (different `occurrence_id`) always
/// create distinct durable rows.
///
/// Task mutation must already have succeeded. Callers must surface errors with
/// [`lifecycle_push_failure_message`] — never claim the original task op retry is safe.
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
    occurrence_id: &str,
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
        occurrence_id,
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
        "occurrence_id": occurrence_id,
        "transition_key": key,
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

    let (notification_id, already_existed, prompt_already_delivered) = match result {
        NotifyIdempotentResult::Created(id) => (id, false, false),
        NotifyIdempotentResult::AlreadyExists {
            id,
            prompt_delivered,
        } => (id, true, prompt_delivered),
    };

    // Fully complete occurrence — no side effects.
    if prompt_already_delivered {
        return Ok(LifecyclePushResult::AlreadyComplete { notification_id });
    }

    // Outbox step: deliver real-time prompt (retry-safe when durable already exists).
    if let Some(pq) = prompt_queue {
        let body = build_prompt_body(
            kind,
            task_id,
            task_title,
            old_status,
            new_status,
            actor,
            reason,
            notification_id,
            factory_session.as_deref(),
            occurrence_id,
        );
        let summary = format!("{}: {} ({})", kind.as_event_type(), task_id, occurrence_id);
        // Include notification_id in source tag so duplicate retries are diagnosable;
        // body carries notification_id so consumers can dedupe if a race double-enqueues.
        let source = format!("lifecycle:{notification_id}");
        pq.enqueue_with_summary(
            &source,
            "supervisor",
            &body,
            factory_session.as_deref(),
            Some(&summary),
        )
        .map_err(|e| {
            format!(
                "prompt_queue write failed after durable enqueue \
                 (notification_id={notification_id}, transition_key={key}): {e}"
            )
        })?;
    }

    supervisor_queue
        .mark_prompt_delivered(notification_id)
        .map_err(|e| {
            format!(
                "failed to stamp prompt_delivered_at for notification_id={notification_id}: {e}"
            )
        })?;

    if already_existed {
        Ok(LifecyclePushResult::Recovered { notification_id })
    } else {
        Ok(LifecyclePushResult::Enqueued { notification_id })
    }
}

impl CasCore {
    /// Push a lifecycle transition after a successful task mutation (cas-062d / cas-17e4).
    ///
    /// `occurrence_id` must identify this mutation (typically
    /// [`occurrence_from_updated_at`] of the post-write `updated_at`).
    ///
    /// Returns `Ok(None)` only when wrapping returns None internally (always Some result).
    /// Returns `Err` when durable write or prompt outbox step fails — callers must surface
    /// via [`lifecycle_push_failure_message`].
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn push_task_lifecycle(
        &self,
        task_id: &str,
        task_title: &str,
        old_status: TaskStatus,
        new_status: TaskStatus,
        actor: &str,
        reason: Option<&str>,
        kind: LifecycleTransition,
        occurrence_id: &str,
    ) -> Result<LifecyclePushResult, String> {
        let agent_store = self
            .open_agent_store()
            .map_err(|e| format!("agent store: {e}"))?;
        let sq = open_supervisor_queue_store(&self.cas_root)
            .map_err(|e| format!("supervisor_queue open: {e}"))?;
        let pq = open_prompt_queue_store(&self.cas_root).ok();
        emit_task_lifecycle_transition(
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
            occurrence_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_store::{
        PromptQueueStore, SqliteAgentStore, SqlitePromptQueueStore, SqliteSupervisorQueueStore,
        SupervisorQueueStore,
    };
    use cas_types::{Agent, AgentRole, AgentStatus};
    use std::sync::Mutex;
    use tempfile::TempDir;

    /// Serialize env mutations in this module's tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn transition_key_includes_session_and_occurrence() {
        let a = transition_key(
            "cas-1",
            TaskStatus::InProgress,
            TaskStatus::Closed,
            Some("sess-a"),
            LifecycleTransition::Closed,
            "occ-1",
        );
        let b = transition_key(
            "cas-1",
            TaskStatus::InProgress,
            TaskStatus::Closed,
            Some("sess-b"),
            LifecycleTransition::Closed,
            "occ-1",
        );
        let c = transition_key(
            "cas-1",
            TaskStatus::InProgress,
            TaskStatus::Closed,
            Some("sess-a"),
            LifecycleTransition::Closed,
            "occ-2",
        );
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert!(a.contains("sess-a"));
        assert!(a.contains("occ-1"));
    }

    #[test]
    fn transition_key_stable_for_same_occurrence() {
        let a = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
            "t1",
        );
        let b = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
            "t1",
        );
        assert_eq!(a, b);
    }

    #[test]
    fn two_start_cycles_get_distinct_keys() {
        let start1 = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
            "t-start-1",
        );
        let start2 = transition_key(
            "cas-1",
            TaskStatus::Open,
            TaskStatus::InProgress,
            Some("s"),
            LifecycleTransition::Started,
            "t-start-2",
        );
        assert_ne!(start1, start2);
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

    #[test]
    fn failure_message_never_claims_task_op_retry_is_safe() {
        let msg = lifecycle_push_failure_message(
            "cas-x",
            TaskStatus::InProgress,
            LifecycleTransition::Started,
            "key",
            "prompt failed",
        );
        assert!(msg.contains("already in_progress"));
        assert!(msg.contains("Do NOT re-run"));
        assert!(!msg.to_lowercase().contains("retry is safe"));
        assert!(msg.contains("transition_key=key"));
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
    fn emit_enqueues_once_and_suppresses_same_occurrence() {
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
            "occ-1",
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
            "occ-1",
        )
        .unwrap();

        match (r1, r2) {
            (
                LifecyclePushResult::Enqueued {
                    notification_id: id1,
                },
                LifecyclePushResult::AlreadyComplete {
                    notification_id: id2,
                },
            ) => assert_eq!(id1, id2),
            other => panic!("expected Enqueued then AlreadyComplete, got {other:?}"),
        }
        assert_eq!(sq.pending_count("sup-emit").unwrap(), 1);
        let pending = sq.peek("sup-emit", 10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].event_type, "task_lifecycle");
        assert!(pending[0].payload.contains("task_started"));
        assert_eq!(
            pending[0].transition_key.as_deref(),
            Some("cas-t1:open:in_progress:sess-emit:task_started:occ-1")
        );
        assert!(pending[0].prompt_delivered_at.is_some());

        // SAFETY: restore env under ENV_LOCK.
        unsafe {
            match prior {
                Some(v) => std::env::set_var("CAS_FACTORY_SESSION", v),
                None => std::env::remove_var("CAS_FACTORY_SESSION"),
            }
        }
    }

    #[test]
    fn emit_two_start_cycles_create_two_events() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("CAS_FACTORY_SESSION").ok();
        unsafe {
            std::env::set_var("CAS_FACTORY_SESSION", "sess-cycle");
        }

        let temp = TempDir::new().unwrap();
        let agents = SqliteAgentStore::open(temp.path()).unwrap();
        agents.init().unwrap();
        agents
            .register(&agent_in_session(
                "sup-c",
                "sup-c",
                AgentRole::Supervisor,
                "sess-cycle",
            ))
            .unwrap();
        let sq = SqliteSupervisorQueueStore::open(temp.path()).unwrap();
        sq.init().unwrap();

        // start₁
        emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-c",
            "C",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "w",
            None,
            LifecycleTransition::Started,
            "t1",
        )
        .unwrap();
        // block
        emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-c",
            "C",
            TaskStatus::InProgress,
            TaskStatus::Blocked,
            "w",
            None,
            LifecycleTransition::Blocked,
            "t2",
        )
        .unwrap();
        // ready
        emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-c",
            "C",
            TaskStatus::Blocked,
            TaskStatus::Open,
            "w",
            None,
            LifecycleTransition::ReadyReopened,
            "t3",
        )
        .unwrap();
        // start₂ — same old/new/kind as start₁ but different occurrence
        emit_task_lifecycle_transition(
            &sq,
            None,
            &agents,
            "cas-c",
            "C",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "w",
            None,
            LifecycleTransition::Started,
            "t4",
        )
        .unwrap();

        assert_eq!(sq.pending_count("sup-c").unwrap(), 4);
        let pending = sq.peek("sup-c", 20).unwrap();
        let started: Vec<_> = pending
            .iter()
            .filter(|n| n.payload.contains("task_started"))
            .collect();
        assert_eq!(started.len(), 2, "two legitimate starts must both emit");

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
            "occ-x",
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

    /// Simulate partial failure: durable insert without prompt stamp, then recover.
    #[test]
    fn durable_without_prompt_recovers_exactly_once_on_replay() {
        let _lock = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("CAS_FACTORY_SESSION").ok();
        unsafe {
            std::env::set_var("CAS_FACTORY_SESSION", "sess-outbox");
        }

        let temp = TempDir::new().unwrap();
        let agents = SqliteAgentStore::open(temp.path()).unwrap();
        agents.init().unwrap();
        agents
            .register(&agent_in_session(
                "sup-o",
                "sup-o",
                AgentRole::Supervisor,
                "sess-outbox",
            ))
            .unwrap();
        let sq = SqliteSupervisorQueueStore::open(temp.path()).unwrap();
        sq.init().unwrap();
        let pq = SqlitePromptQueueStore::open(temp.path()).unwrap();
        pq.init().unwrap();

        let key = "cas-o:open:in_progress:sess-outbox:task_started:occ-outbox";
        // Inject partial failure: durable row exists, prompt not stamped.
        let created = sq
            .notify_idempotent(
                "sup-o",
                "task_lifecycle",
                r#"{"task_id":"cas-o"}"#,
                NotificationPriority::Normal,
                key,
            )
            .unwrap();
        let id = match created {
            NotifyIdempotentResult::Created(id) => id,
            other => panic!("expected Created, got {other:?}"),
        };
        assert!(sq.get_by_transition_key(key).unwrap().unwrap().prompt_delivered_at.is_none());

        // Replay via emit: must deliver prompt + stamp, not insert second durable row.
        let r2 = emit_task_lifecycle_transition(
            &sq,
            Some(&pq as &dyn PromptQueueStore),
            &agents,
            "cas-o",
            "O",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "w",
            None,
            LifecycleTransition::Started,
            "occ-outbox",
        )
        .expect("replay must succeed");
        assert!(
            matches!(r2, LifecyclePushResult::Recovered { notification_id } if notification_id == id),
            "got {r2:?}"
        );
        assert_eq!(sq.pending_count("sup-o").unwrap(), 1);
        assert!(sq.get_by_transition_key(key).unwrap().unwrap().prompt_delivered_at.is_some());
        assert_eq!(pq.pending_count().unwrap(), 1);

        // Third call: fully complete — no additional prompt row.
        let r3 = emit_task_lifecycle_transition(
            &sq,
            Some(&pq as &dyn PromptQueueStore),
            &agents,
            "cas-o",
            "O",
            TaskStatus::Open,
            TaskStatus::InProgress,
            "w",
            None,
            LifecycleTransition::Started,
            "occ-outbox",
        )
        .unwrap();
        assert!(matches!(r3, LifecyclePushResult::AlreadyComplete { .. }));
        assert_eq!(pq.pending_count().unwrap(), 1, "exactly-once prompt delivery");

        unsafe {
            match prior {
                Some(v) => std::env::set_var("CAS_FACTORY_SESSION", v),
                None => std::env::remove_var("CAS_FACTORY_SESSION"),
            }
        }
    }
}
