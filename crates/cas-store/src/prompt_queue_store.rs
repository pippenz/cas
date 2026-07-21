//! Prompt queue for supervisor → worker communication in factory sessions
//!
//! Allows supervisor agents to send prompts to workers via MCP.
//! Factory TUI polls this queue and injects prompts into worker PTYs.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::Result;
use crate::recording_store::capture_message_event;
use crate::supervisor_queue_store::NotificationPriority;

/// A prompt in the queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuedPrompt {
    /// Unique prompt ID
    pub id: i64,
    /// Source agent (who sent the prompt)
    pub source: String,
    /// Target agent name or "all_workers"
    pub target: String,
    /// The prompt text to inject
    pub prompt: String,
    /// When the prompt was queued
    pub created_at: DateTime<Utc>,
    /// When the prompt was processed (None if pending)
    pub processed_at: Option<DateTime<Utc>>,
    /// Owning factory session for session-scoped delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub factory_session: Option<String>,
    /// Short summary for UI display
    pub summary: Option<String>,
    /// Message priority (lower = higher priority)
    pub priority: NotificationPriority,
    /// When the target agent acknowledged receipt (None if not yet acked)
    pub acked_at: Option<DateTime<Utc>>,
    /// Urgent delivery flag (cas-c931): when true, the daemon breaks the
    /// target's in-flight turn (Esc) and injects via the PTY, bypassing the
    /// Claude Code inbox even in agent-teams mode. Default false = normal
    /// inbox/queue delivery (non-disruptive).
    pub urgent: bool,
}

/// Schema for prompt queue table
const PROMPT_QUEUE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS prompt_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    target TEXT NOT NULL,
    prompt TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    processed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_prompt_queue_pending ON prompt_queue(target) WHERE processed_at IS NULL;
"#;

/// Add factory_session column for multi-session isolation.
/// Uses IF NOT EXISTS via a safe column-add pattern.
const PROMPT_QUEUE_SESSION_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN factory_session TEXT;
"#;

/// Add summary column for UI display.
const PROMPT_QUEUE_SUMMARY_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN summary TEXT;
"#;

/// Add priority column for message ordering (0=Critical, 1=High, 2=Normal).
const PROMPT_QUEUE_PRIORITY_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN priority INTEGER NOT NULL DEFAULT 2;
"#;

/// Add acked_at column for delivery confirmation.
const PROMPT_QUEUE_ACKED_AT_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN acked_at TEXT;
"#;

/// Add urgent column (cas-c931) for interrupt-and-redirect delivery.
/// 0 = normal inbox/queue delivery (default), 1 = break the target's turn
/// (Esc) then inject via PTY, bypassing the Claude Code inbox.
const PROMPT_QUEUE_URGENT_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN urgent INTEGER NOT NULL DEFAULT 0;
"#;

/// Indexes supporting two-lane `peek_for_targets` selection (cas-2bcb).
/// Partial indexes keep the path bounded to pending rows only.
const PROMPT_QUEUE_TWO_LANE_INDEXES: &str = r#"
CREATE INDEX IF NOT EXISTS idx_prompt_queue_session_pending
    ON prompt_queue(factory_session, priority, id)
    WHERE processed_at IS NULL AND factory_session IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_prompt_queue_legacy_pending
    ON prompt_queue(target, priority, id)
    WHERE processed_at IS NULL AND factory_session IS NULL;
"#;

/// Delivery status of a prompt queue message (legacy three-value ladder).
///
/// Preserved for existing MCP/clients. Prefer [`MessageDeliveryReport`] for
/// stage-based observability (cas-2c5f).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MessageStatus {
    /// Message is queued but not yet delivered
    Pending,
    /// Message was injected/delivered but not yet acknowledged by the target
    Delivered,
    /// Target agent has confirmed receipt
    Confirmed,
}

impl std::fmt::Display for MessageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Delivered => write!(f, "delivered"),
            Self::Confirmed => write!(f, "confirmed"),
        }
    }
}

/// Monotonic transport stage CAS can authoritatively observe (cas-2c5f).
///
/// Rank order (never regresses once stamped via `highest_stage`):
/// Enqueued < Selected < Gated < {Dropped|Suppressed|Abandoned|Delivered} < Confirmed
///
/// **Delivered** is only for successful inbox/PTY handoff (`transport_delivered_at`).
/// Legacy `processed_at` alone is **not** sufficient (drops/suppression/abandon also
/// set it). Harness wake/reaction are never stages — see [`ObservationStatus`].
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStage {
    /// Row exists with parseable `created_at`.
    Enqueued,
    /// Daemon selected/peeked the row for a delivery attempt.
    Selected,
    /// Delivery attempt blocked by a gate (pane not ready, target unavailable).
    Gated,
    /// Intentionally not delivered: dead-source drop.
    Dropped,
    /// Intentionally not delivered: idle-message rate-limit suppression.
    Suppressed,
    /// Intentionally not delivered: unknown/stale target abandoned.
    Abandoned,
    /// Authoritative transport handoff succeeded (inbox/PTY inject Ok).
    Delivered,
    /// Target acknowledged (`acked_at` set).
    Confirmed,
}

impl DeliveryStage {
    /// Monotonic rank. Equal ranks (Dropped/Suppressed/Abandoned/Delivered)
    /// are terminal siblings — once any is set, only Confirmed may advance.
    pub fn rank(self) -> u8 {
        match self {
            Self::Enqueued => 0,
            Self::Selected => 1,
            Self::Gated => 2,
            Self::Dropped | Self::Suppressed | Self::Abandoned | Self::Delivered => 3,
            Self::Confirmed => 4,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Enqueued => "enqueued",
            Self::Selected => "selected",
            Self::Gated => "gated",
            Self::Dropped => "dropped",
            Self::Suppressed => "suppressed",
            Self::Abandoned => "abandoned",
            Self::Delivered => "delivered",
            Self::Confirmed => "confirmed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "enqueued" => Some(Self::Enqueued),
            "selected" => Some(Self::Selected),
            "gated" => Some(Self::Gated),
            "dropped" => Some(Self::Dropped),
            "suppressed" => Some(Self::Suppressed),
            "abandoned" => Some(Self::Abandoned),
            "delivered" => Some(Self::Delivered),
            "confirmed" => Some(Self::Confirmed),
            _ => None,
        }
    }

    /// Advance only if `proposed` ranks strictly higher (monotonic).
    pub fn advance(self, proposed: Self) -> Self {
        if proposed.rank() > self.rank() {
            proposed
        } else {
            self
        }
    }
}

impl std::fmt::Display for DeliveryStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Why a message has not completed transport confirmation (cas-2c5f).
///
/// Only reasons CAS can authoritatively stamp or derive without false precision.
/// Inaccurate queue-head inference was removed (review reject of ae8f47b).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PendingReason {
    /// Row is not eligible for the observing session's selection rules.
    SessionIneligible,
    /// Target agent/pane is not available to receive delivery.
    TargetUnavailable,
    /// Delivery gated (e.g. pane not ready for injection).
    GatedNotReady,
    /// Last adapter attempt failed; row left pending for retry.
    AdapterRetryable,
    /// Enqueued/selected and not known blocked; awaiting next delivery tick.
    AwaitingDelivery,
    /// Authoritative transport delivered; waiting for target `message_ack`.
    AwaitingAck,
    /// Terminal non-delivery: dead worker source dropped.
    DroppedDeadSource,
    /// Terminal non-delivery: duplicate idle suppression.
    SuppressedIdle,
    /// Terminal non-delivery: unknown/stale target abandoned.
    AbandonedUnknownTarget,
}

impl PendingReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionIneligible => "session_ineligible",
            Self::TargetUnavailable => "target_unavailable",
            Self::GatedNotReady => "gated_not_ready",
            Self::AdapterRetryable => "adapter_retryable",
            Self::AwaitingDelivery => "awaiting_delivery",
            Self::AwaitingAck => "awaiting_ack",
            Self::DroppedDeadSource => "dropped_dead_source",
            Self::SuppressedIdle => "suppressed_idle",
            Self::AbandonedUnknownTarget => "abandoned_unknown_target",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "session_ineligible" => Some(Self::SessionIneligible),
            "target_unavailable" => Some(Self::TargetUnavailable),
            "gated_not_ready" => Some(Self::GatedNotReady),
            "adapter_retryable" => Some(Self::AdapterRetryable),
            "awaiting_delivery" => Some(Self::AwaitingDelivery),
            "awaiting_ack" => Some(Self::AwaitingAck),
            "dropped_dead_source" => Some(Self::DroppedDeadSource),
            "suppressed_idle" => Some(Self::SuppressedIdle),
            "abandoned_unknown_target" => Some(Self::AbandonedUnknownTarget),
            // Legacy alias from first implementation (no longer inferred).
            "behind_queue_head" => Some(Self::AwaitingDelivery),
            _ => None,
        }
    }

    /// Stage implied when this reason is first observed (before monotonic merge).
    fn implied_stage(self) -> DeliveryStage {
        match self {
            Self::GatedNotReady | Self::TargetUnavailable => DeliveryStage::Gated,
            Self::DroppedDeadSource => DeliveryStage::Dropped,
            Self::SuppressedIdle => DeliveryStage::Suppressed,
            Self::AbandonedUnknownTarget => DeliveryStage::Abandoned,
            Self::AdapterRetryable
            | Self::AwaitingDelivery
            | Self::SessionIneligible
            | Self::AwaitingAck => DeliveryStage::Selected,
        }
    }
}

impl std::fmt::Display for PendingReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Evidence CAS does **not** fabricate (harness wake / model reaction).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ObservationStatus {
    /// No authoritative CAS observation for this stage.
    #[default]
    Unobserved,
}

impl std::fmt::Display for ObservationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unobserved => write!(f, "unobserved"),
        }
    }
}

/// Stage-based delivery report for one prompt_queue message (cas-2c5f).
///
/// Additive to legacy [`MessageStatus`]. Wake/reaction are always
/// [`ObservationStatus::Unobserved`] — never inferred from timestamps.
///
/// `delivered_at` is **only** set after successful transport handoff
/// (`transport_delivered_at`), never from legacy `processed_at` alone.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MessageDeliveryReport {
    pub id: i64,
    /// Legacy three-value status (processed_at/acked_at) for older clients.
    pub legacy_status: MessageStatus,
    /// Highest monotonic stage reached (authoritative columns only).
    pub stage: DeliveryStage,
    pub source: String,
    pub target: String,
    pub factory_session: Option<String>,
    pub priority: u8,
    pub urgent: bool,
    pub enqueued_at: DateTime<Utc>,
    pub selected_at: Option<DateTime<Utc>>,
    /// Authoritative transport handoff time only (not legacy processed_at).
    pub delivered_at: Option<DateTime<Utc>>,
    pub confirmed_at: Option<DateTime<Utc>>,
    /// Present when waiting/blocked or for terminal non-delivery explanation.
    pub pending_reason: Option<PendingReason>,
    /// Human-readable detail for the pending reason (error text, …).
    pub pending_detail: Option<String>,
    pub wake: ObservationStatus,
    pub reaction: ObservationStatus,
}

/// Stage-observability columns (cas-2c5f). Idempotent ALTERs.
const PROMPT_QUEUE_SELECTED_AT_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN selected_at TEXT;
"#;
const PROMPT_QUEUE_PENDING_REASON_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN last_pending_reason TEXT;
"#;
const PROMPT_QUEUE_PENDING_DETAIL_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN last_pending_detail TEXT;
"#;
const PROMPT_QUEUE_TRANSPORT_DELIVERED_AT_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN transport_delivered_at TEXT;
"#;
const PROMPT_QUEUE_HIGHEST_STAGE_MIGRATION: &str = r#"
ALTER TABLE prompt_queue ADD COLUMN highest_stage TEXT;
"#;

/// Trait for prompt queue operations
pub trait PromptQueueStore: Send + Sync {
    /// Initialize the store (create tables)
    fn init(&self) -> Result<()>;

    /// Queue a prompt for a target agent
    fn enqueue(&self, source: &str, target: &str, prompt: &str) -> Result<i64>;

    /// Queue a prompt tagged with a factory session for isolation
    fn enqueue_with_session(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: &str,
    ) -> Result<i64>;

    /// Queue a prompt with session, summary, and priority for UI display
    fn enqueue_with_summary(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
    ) -> Result<i64> {
        self.enqueue_full(source, target, prompt, factory_session, summary, None)
    }

    /// Queue a prompt with all options including priority.
    ///
    /// Equivalent to [`PromptQueueStore::enqueue_urgent`] with `urgent = false`.
    fn enqueue_full(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
        priority: Option<NotificationPriority>,
    ) -> Result<i64> {
        self.enqueue_urgent(
            source,
            target,
            prompt,
            factory_session,
            summary,
            priority,
            false,
        )
    }

    /// Queue a prompt with all options, including the cas-c931 `urgent` flag.
    ///
    /// When `urgent` is true, the daemon delivers via interrupt-and-redirect:
    /// it breaks the target worker's in-flight turn (Esc) and injects the
    /// message via the PTY, bypassing the Claude Code inbox even in agent-teams
    /// mode. When false, delivery is unchanged (inbox/queue, non-disruptive).
    fn enqueue_urgent(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
        priority: Option<NotificationPriority>,
        urgent: bool,
    ) -> Result<i64>;

    /// Poll for pending prompts for a specific target (marks as processed)
    fn poll_for_target(&self, target: &str, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Poll for pending prompts for a specific target within a factory session.
    ///
    /// `None` preserves legacy behavior. When a session is supplied, tagged
    /// rows only match that session, while NULL-session legacy rows still use
    /// the historical target/all_workers matching path.
    fn poll_for_target_with_session(
        &self,
        target: &str,
        factory_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedPrompt>>;

    /// Poll all pending prompts (for Factory TUI to process)
    fn poll_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Peek at pending prompts without marking as processed
    fn peek_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Peek at pending prompts for specific targets only.
    ///
    /// # Eligibility (applied before LIMIT)
    /// - **Session lane:** rows with `factory_session` equal to the supplied
    ///   session (never matched by target-name collision alone).
    /// - **Legacy lane:** NULL-session rows whose `target` is in `targets`
    ///   (historical compatibility arm).
    ///
    /// # Cross-lane selection contract (when `factory_session` is set)
    /// Two independent peeks (each `ORDER BY priority ASC, id ASC LIMIT n`)
    /// are merged with **global priority first**, then same-priority lane
    /// fairness (cas-2bcb / cas-04a6 R1):
    /// 1. Walk priority bands from highest urgency (lowest numeric priority)
    ///    to lowest. No lower-priority row is selected while any higher-
    ///    priority eligible row remains in either lane's candidate set.
    /// 2. Within a single priority band, apply a bounded two-lane quota so
    ///    neither session nor legacy can permanently occupy the band's
    ///    remaining slots: `ceil(remaining/2)` session + `floor(remaining/2)`
    ///    legacy when both have work at that priority; unused quota fills
    ///    the other lane. Final order within a band: `id ASC` (FIFO).
    /// 3. `limit == 1`: the single slot is the highest-priority eligible
    ///    head; only when both heads share the same priority does equal-
    ///    priority fairness apply (session preferred at limit=1).
    ///
    /// Without a session tag, behavior is the historical single-lane target
    /// filter. Session isolation: other sessions' tagged rows never leak.
    fn peek_for_targets(
        &self,
        targets: &[&str],
        factory_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedPrompt>>;

    /// Mark a prompt as processed
    fn mark_processed(&self, prompt_id: i64) -> Result<()>;

    /// Acknowledge receipt of a prompt (target agent confirms delivery)
    fn ack(&self, prompt_id: i64) -> Result<()>;

    /// Get messages that were processed but not acked within the timeout
    fn unacked(&self, timeout_secs: i64, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Get delivery status of a specific message (legacy three-value ladder).
    fn message_status(&self, prompt_id: i64) -> Result<Option<MessageStatus>>;

    /// Stage-based delivery report (cas-2c5f). `None` if the id does not exist.
    /// Returns `Err` if required timestamps are corrupt (never fabricates `now`).
    fn message_delivery_report(&self, prompt_id: i64) -> Result<Option<MessageDeliveryReport>>;

    /// Record that the daemon selected/peeked this message for a delivery attempt.
    fn record_selected(&self, prompt_id: i64) -> Result<()>;

    /// Record a durable pending reason; advances highest_stage monotonically.
    fn record_pending_reason(
        &self,
        prompt_id: i64,
        reason: PendingReason,
        detail: Option<&str>,
    ) -> Result<()>;

    /// Authoritative successful transport handoff (inbox/PTY Ok).
    /// Sets `transport_delivered_at` + stage Delivered + legacy `processed_at`.
    fn mark_transport_delivered(&self, prompt_id: i64) -> Result<()>;

    /// Dead-source drop: marks processed for queue drainage without transport success.
    fn mark_dropped(&self, prompt_id: i64, detail: Option<&str>) -> Result<()>;

    /// Idle-message suppression: processed without transport success.
    fn mark_suppressed(&self, prompt_id: i64, detail: Option<&str>) -> Result<()>;

    /// Unknown-target abandon: processed without transport success.
    fn mark_abandoned(&self, prompt_id: i64, detail: Option<&str>) -> Result<()>;

    /// Get count of pending prompts
    fn pending_count(&self) -> Result<usize>;

    /// Clear all prompts (for cleanup)
    fn clear(&self) -> Result<usize>;

    /// Clear old processed prompts (cleanup)
    fn cleanup_old(&self, older_than_secs: i64) -> Result<usize>;

    /// Close the store
    fn close(&self) -> Result<()>;
}

/// SQLite-based prompt queue store
pub struct SqlitePromptQueueStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqlitePromptQueueStore {
    /// Open or create a SQLite prompt queue store
    pub fn open(cas_dir: &Path) -> Result<Self> {
        let db_path = cas_dir.join("cas.db");
        let conn = crate::shared_db::shared_connection(&db_path)?;

        Ok(Self { conn })
    }

    fn parse_datetime(s: &str) -> Option<DateTime<Utc>> {
        if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
            return Some(dt.with_timezone(&Utc));
        }
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
            return Some(Utc.from_utc_datetime(&dt));
        }
        None
    }

    fn prompt_from_row(row: &rusqlite::Row) -> rusqlite::Result<QueuedPrompt> {
        let processed_at_str: Option<String> = row.get(5)?;
        let processed_at = processed_at_str.and_then(|s| Self::parse_datetime(&s));
        let summary: Option<String> = row.get(6).unwrap_or(None);
        let priority: u8 = row.get(7).unwrap_or(2);
        let acked_at_str: Option<String> = row.get(8).unwrap_or(None);
        let acked_at = acked_at_str.and_then(|s| Self::parse_datetime(&s));
        // Column 9 = urgent (cas-c931). Tolerate absence on legacy rows/tables.
        let urgent: bool = row.get::<_, i64>(9).map(|v| v != 0).unwrap_or(false);
        let factory_session: Option<String> = row.get(10).unwrap_or(None);

        Ok(QueuedPrompt {
            id: row.get(0)?,
            source: row.get(1)?,
            target: row.get(2)?,
            prompt: row.get(3)?,
            created_at: Self::parse_datetime(&row.get::<_, String>(4)?).unwrap_or_else(Utc::now),
            processed_at,
            factory_session,
            summary,
            priority: NotificationPriority::from(priority),
            acked_at,
            urgent,
        })
    }

    /// Merge session-lane and legacy-lane peeks.
    ///
    /// Contract (see `PromptQueueStore::peek_for_targets`):
    /// 1. Global priority bands first (never emit priority P+1 while any
    ///    priority ≤P candidate remains).
    /// 2. Within one priority band only: bounded two-lane quota
    ///    (ceil(n/2) session + floor(n/2) legacy; unused fills the other).
    /// 3. Within a band, FIFO by id across the selected set.
    fn merge_two_lane_peeks(
        session: Vec<QueuedPrompt>,
        legacy: Vec<QueuedPrompt>,
        limit: usize,
    ) -> Vec<QueuedPrompt> {
        if limit == 0 {
            return Vec::new();
        }
        if session.is_empty() {
            return legacy.into_iter().take(limit).collect();
        }
        if legacy.is_empty() {
            return session.into_iter().take(limit).collect();
        }

        // Inputs are already ORDER BY priority ASC, id ASC per lane.
        let mut s_idx = 0usize;
        let mut l_idx = 0usize;
        let mut selected: Vec<QueuedPrompt> = Vec::with_capacity(limit);

        while selected.len() < limit && (s_idx < session.len() || l_idx < legacy.len()) {
            let next_priority = match (session.get(s_idx), legacy.get(l_idx)) {
                (Some(s), Some(l)) => (s.priority as u8).min(l.priority as u8),
                (Some(s), None) => s.priority as u8,
                (None, Some(l)) => l.priority as u8,
                (None, None) => break,
            };

            // Drain the full same-priority band from each lane head.
            let s_start = s_idx;
            while s_idx < session.len() && session[s_idx].priority as u8 == next_priority {
                s_idx += 1;
            }
            let l_start = l_idx;
            while l_idx < legacy.len() && legacy[l_idx].priority as u8 == next_priority {
                l_idx += 1;
            }

            let remaining = limit - selected.len();
            let band = Self::fair_quota_same_priority(
                &session[s_start..s_idx],
                &legacy[l_start..l_idx],
                remaining,
            );
            selected.extend(band);
        }

        selected
    }

    /// Bounded two-lane fairness among rows that already share one priority.
    /// Session reserve = ceil(limit/2), legacy = floor(limit/2); unused fills
    /// the other lane. Output ordered by id ASC (equal-priority FIFO).
    fn fair_quota_same_priority(
        session: &[QueuedPrompt],
        legacy: &[QueuedPrompt],
        limit: usize,
    ) -> Vec<QueuedPrompt> {
        if limit == 0 {
            return Vec::new();
        }
        if session.is_empty() {
            return legacy.iter().take(limit).cloned().collect();
        }
        if legacy.is_empty() {
            return session.iter().take(limit).cloned().collect();
        }

        let session_quota = (limit + 1) / 2;
        let legacy_quota = limit / 2;

        let mut s_take = session_quota.min(session.len());
        let mut l_take = legacy_quota.min(legacy.len());

        // Give unused reserve to the other lane.
        let used = s_take + l_take;
        if used < limit {
            let spare = limit - used;
            let s_extra = (session.len() - s_take).min(spare);
            s_take += s_extra;
            let spare = limit - s_take - l_take;
            let l_extra = (legacy.len() - l_take).min(spare);
            l_take += l_extra;
        }

        let mut selected: Vec<QueuedPrompt> = Vec::with_capacity(s_take + l_take);
        selected.extend(session.iter().take(s_take).cloned());
        selected.extend(legacy.iter().take(l_take).cloned());
        selected.sort_by_key(|p| p.id);
        selected
    }

    fn query_lane(
        conn: &Connection,
        sql: &str,
        params: &[Box<dyn rusqlite::ToSql>],
    ) -> Result<Vec<QueuedPrompt>> {
        let mut stmt = conn.prepare_cached(sql)?;
        let prompts = stmt
            .query_map(
                rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                Self::prompt_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(prompts)
    }

    fn require_datetime(s: &str, field: &str, id: i64) -> Result<DateTime<Utc>> {
        Self::parse_datetime(s).ok_or_else(|| {
            crate::error::StoreError::Parse(format!(
                "prompt_queue id={id}: corrupt/unparseable {field} timestamp: {s:?}"
            ))
        })
    }

    fn optional_datetime(s: Option<&str>, field: &str, id: i64) -> Result<Option<DateTime<Utc>>> {
        match s {
            None => Ok(None),
            Some(raw) => Ok(Some(Self::require_datetime(raw, field, id)?)),
        }
    }

    /// Read current highest_stage, merge with proposed, persist if advanced.
    fn advance_highest_stage(
        conn: &Connection,
        prompt_id: i64,
        proposed: DeliveryStage,
    ) -> Result<DeliveryStage> {
        let current_s: Option<String> = conn
            .query_row(
                "SELECT highest_stage FROM prompt_queue WHERE id = ?",
                params![prompt_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        let current = current_s
            .as_deref()
            .and_then(DeliveryStage::parse)
            .unwrap_or(DeliveryStage::Enqueued);
        let next = current.advance(proposed);
        if next != current {
            conn.execute(
                "UPDATE prompt_queue SET highest_stage = ? WHERE id = ?",
                params![next.as_str(), prompt_id],
            )?;
        }
        Ok(next)
    }

    fn stamp_terminal_non_delivery(
        &self,
        prompt_id: i64,
        stage: DeliveryStage,
        reason: PendingReason,
        detail: Option<&str>,
    ) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            Self::advance_highest_stage(&conn, prompt_id, stage)?;
            conn.execute(
                "UPDATE prompt_queue
                 SET selected_at = COALESCE(selected_at, ?),
                     processed_at = COALESCE(processed_at, ?),
                     last_pending_reason = ?,
                     last_pending_detail = ?
                 WHERE id = ?",
                params![now, now, reason.as_str(), detail, prompt_id],
            )?;
            Ok(())
        })
    }
}

impl PromptQueueStore for SqlitePromptQueueStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(PROMPT_QUEUE_SCHEMA)?;

        // Add factory_session column if missing (safe migration for multi-session isolation)
        let has_session_col = conn
            .prepare_cached("SELECT factory_session FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_session_col {
            conn.execute_batch(PROMPT_QUEUE_SESSION_MIGRATION)?;
        }

        // Add summary column if missing
        let has_summary_col = conn
            .prepare_cached("SELECT summary FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_summary_col {
            conn.execute_batch(PROMPT_QUEUE_SUMMARY_MIGRATION)?;
        }

        // Add priority column if missing
        let has_priority_col = conn
            .prepare_cached("SELECT priority FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_priority_col {
            conn.execute_batch(PROMPT_QUEUE_PRIORITY_MIGRATION)?;
        }

        // Add acked_at column if missing (delivery confirmation)
        let has_acked_at_col = conn
            .prepare_cached("SELECT acked_at FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_acked_at_col {
            conn.execute_batch(PROMPT_QUEUE_ACKED_AT_MIGRATION)?;
        }

        // Add urgent column if missing (cas-c931 interrupt-and-redirect)
        let has_urgent_col = conn
            .prepare_cached("SELECT urgent FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_urgent_col {
            conn.execute_batch(PROMPT_QUEUE_URGENT_MIGRATION)?;
        }

        // Stage-observability columns (cas-2c5f)
        let has_selected_at = conn
            .prepare_cached("SELECT selected_at FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_selected_at {
            conn.execute_batch(PROMPT_QUEUE_SELECTED_AT_MIGRATION)?;
        }
        let has_pending_reason = conn
            .prepare_cached("SELECT last_pending_reason FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_pending_reason {
            conn.execute_batch(PROMPT_QUEUE_PENDING_REASON_MIGRATION)?;
        }
        let has_pending_detail = conn
            .prepare_cached("SELECT last_pending_detail FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_pending_detail {
            conn.execute_batch(PROMPT_QUEUE_PENDING_DETAIL_MIGRATION)?;
        }
        let has_transport_delivered = conn
            .prepare_cached("SELECT transport_delivered_at FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_transport_delivered {
            conn.execute_batch(PROMPT_QUEUE_TRANSPORT_DELIVERED_AT_MIGRATION)?;
        }
        let has_highest_stage = conn
            .prepare_cached("SELECT highest_stage FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_highest_stage {
            conn.execute_batch(PROMPT_QUEUE_HIGHEST_STAGE_MIGRATION)?;
        }

        // Two-lane peek indexes (idempotent CREATE INDEX IF NOT EXISTS).
        conn.execute_batch(PROMPT_QUEUE_TWO_LANE_INDEXES)?;

        Ok(())
    }

    fn enqueue(&self, source: &str, target: &str, prompt: &str) -> Result<i64> {
        self.enqueue_full(source, target, prompt, None, None, None)
    }

    fn enqueue_with_session(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: &str,
    ) -> Result<i64> {
        self.enqueue_full(source, target, prompt, Some(factory_session), None, None)
    }

    fn enqueue_urgent(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
        priority: Option<NotificationPriority>,
        urgent: bool,
    ) -> Result<i64> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            let prio: i32 = priority.unwrap_or(NotificationPriority::Normal).into();
            let urgent_flag: i64 = if urgent { 1 } else { 0 };

            conn.execute(
            "INSERT INTO prompt_queue (source, target, prompt, created_at, factory_session, summary, priority, urgent) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            params![source, target, prompt, now, factory_session, summary, prio, urgent_flag],
        )?;

            let id = conn.last_insert_rowid();
            let _ = capture_message_event(&conn, source, target);
            Ok(id)
        }) // with_write_retry
    }

    fn poll_for_target(&self, target: &str, limit: usize) -> Result<Vec<QueuedPrompt>> {
        self.poll_for_target_with_session(target, None, limit)
    }

    fn poll_for_target_with_session(
        &self,
        target: &str,
        factory_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedPrompt>> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();

            let (sql, prompt_params): (&str, Vec<Box<dyn rusqlite::ToSql>>) = if let Some(session) =
                factory_session
            {
                (
                    "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE processed_at IS NULL
               AND (
                    (factory_session = ? AND (target = ? OR target = 'all_workers'))
                    OR (factory_session IS NULL AND (target = ? OR target = 'all_workers'))
               )
             ORDER BY priority ASC, id ASC
             LIMIT ?",
                    vec![
                        Box::new(session.to_string()),
                        Box::new(target.to_string()),
                        Box::new(target.to_string()),
                        Box::new(limit as i64),
                    ],
                )
            } else {
                (
                    "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE (target = ? OR target = 'all_workers') AND processed_at IS NULL
             ORDER BY priority ASC, id ASC
             LIMIT ?",
                    vec![Box::new(target.to_string()), Box::new(limit as i64)],
                )
            };

            let mut stmt = conn.prepare_cached(sql)?;

            let prompts: Vec<QueuedPrompt> = stmt
                .query_map(
                    rusqlite::params_from_iter(prompt_params.iter().map(|p| p.as_ref())),
                    Self::prompt_from_row,
                )?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Mark them as processed
            if !prompts.is_empty() {
                let ids: Vec<i64> = prompts.iter().map(|p| p.id).collect();
                let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "UPDATE prompt_queue SET processed_at = ?, last_pending_reason = NULL, last_pending_detail = NULL WHERE id IN ({})",
                    placeholders.join(", ")
                );

                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];
                for id in ids {
                    params.push(Box::new(id));
                }

                conn.execute(
                    &sql,
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                )?;
            }

            Ok(prompts)
        }) // with_write_retry
    }

    fn poll_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();

            let mut stmt = conn.prepare_cached(
            "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE processed_at IS NULL
             ORDER BY priority ASC, id ASC
             LIMIT ?",
        )?;

            let prompts: Vec<QueuedPrompt> = stmt
                .query_map(params![limit as i64], Self::prompt_from_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            // Mark them as processed
            if !prompts.is_empty() {
                let ids: Vec<i64> = prompts.iter().map(|p| p.id).collect();
                let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
                let sql = format!(
                    "UPDATE prompt_queue SET processed_at = ?, last_pending_reason = NULL, last_pending_detail = NULL WHERE id IN ({})",
                    placeholders.join(", ")
                );

                let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(now)];
                for id in ids {
                    params.push(Box::new(id));
                }

                conn.execute(
                    &sql,
                    rusqlite::params_from_iter(params.iter().map(|p| p.as_ref())),
                )?;
            }

            Ok(prompts)
        }) // with_write_retry
    }

    fn peek_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare_cached(
            "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE processed_at IS NULL
             ORDER BY priority ASC, id ASC
             LIMIT ?",
        )?;

        let prompts = stmt
            .query_map(params![limit as i64], Self::prompt_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(prompts)
    }

    fn peek_for_targets(
        &self,
        targets: &[&str],
        factory_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedPrompt>> {
        if limit == 0 || (targets.is_empty() && factory_session.is_none()) {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().unwrap();

        // Legacy path (no session): single-lane target filter.
        let Some(session) = factory_session else {
            if targets.is_empty() {
                return Ok(Vec::new());
            }
            let placeholders: Vec<&str> = std::iter::repeat_n("?", targets.len()).collect();
            let sql = format!(
                "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
                 FROM prompt_queue
                 WHERE processed_at IS NULL
                   AND target IN ({})
                 ORDER BY priority ASC, id ASC
                 LIMIT ?",
                placeholders.join(", ")
            );
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = targets
                .iter()
                .map(|t| Box::new(t.to_string()) as Box<dyn rusqlite::ToSql>)
                .collect();
            params.push(Box::new(limit as i64));
            return Self::query_lane(&conn, &sql, &params);
        };

        // Live-session path: two indexable peeks + bounded two-lane merge
        // (cas-2bcb). Each lane is LIMIT-bounded so neither can permanently
        // occupy the caller's window.
        let session_sql = "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE processed_at IS NULL
               AND factory_session = ?
             ORDER BY priority ASC, id ASC
             LIMIT ?";
        let session_params: Vec<Box<dyn rusqlite::ToSql>> =
            vec![Box::new(session.to_string()), Box::new(limit as i64)];
        let session_lane = Self::query_lane(&conn, session_sql, &session_params)?;

        let legacy_lane = if targets.is_empty() {
            Vec::new()
        } else {
            let placeholders: Vec<&str> = std::iter::repeat_n("?", targets.len()).collect();
            let legacy_sql = format!(
                "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
                 FROM prompt_queue
                 WHERE processed_at IS NULL
                   AND factory_session IS NULL
                   AND target IN ({})
                 ORDER BY priority ASC, id ASC
                 LIMIT ?",
                placeholders.join(", ")
            );
            let mut legacy_params: Vec<Box<dyn rusqlite::ToSql>> = targets
                .iter()
                .map(|t| Box::new(t.to_string()) as Box<dyn rusqlite::ToSql>)
                .collect();
            legacy_params.push(Box::new(limit as i64));
            Self::query_lane(&conn, &legacy_sql, &legacy_params)?
        };

        Ok(Self::merge_two_lane_peeks(session_lane, legacy_lane, limit))
    }

    fn mark_processed(&self, prompt_id: i64) -> Result<()> {
        // Legacy API: sets processed_at only. Does **not** stamp authoritative
        // transport delivery (cas-2c5f). Prefer mark_transport_delivered /
        // mark_dropped / mark_suppressed / mark_abandoned on the delivery path.
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            conn.execute(
                "UPDATE prompt_queue SET processed_at = ? WHERE id = ?",
                params![now, prompt_id],
            )?;
            Ok(())
        }) // with_write_retry
    }

    fn ack(&self, prompt_id: i64) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            let _ = Self::advance_highest_stage(&conn, prompt_id, DeliveryStage::Confirmed);
            conn.execute(
                "UPDATE prompt_queue SET acked_at = ? WHERE id = ? AND acked_at IS NULL",
                params![now, prompt_id],
            )?;
            // rows_affected == 0 means either not found or already acked — both idempotent
            Ok(())
        }) // with_write_retry
    }

    fn unacked(&self, timeout_secs: i64, limit: usize) -> Result<Vec<QueuedPrompt>> {
        let conn = self.conn.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::seconds(timeout_secs)).to_rfc3339();

        let mut stmt = conn.prepare_cached(
            "SELECT id, source, target, prompt, created_at, processed_at, summary, priority, acked_at, urgent, factory_session
             FROM prompt_queue
             WHERE processed_at IS NOT NULL
               AND processed_at < ?
               AND acked_at IS NULL
             ORDER BY priority ASC, id ASC
             LIMIT ?",
        )?;

        let prompts = stmt
            .query_map(params![cutoff, limit as i64], Self::prompt_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(prompts)
    }

    fn message_status(&self, prompt_id: i64) -> Result<Option<MessageStatus>> {
        // Legacy ladder: processed_at / acked_at only (includes non-delivery
        // drains). Structured stage uses separate columns.
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT processed_at, acked_at FROM prompt_queue WHERE id = ?",
            params![prompt_id],
            |row| {
                let processed_at: Option<String> = row.get(0)?;
                let acked_at: Option<String> = row.get(1)?;
                Ok((processed_at, acked_at))
            },
        );
        match result {
            Ok((_, Some(_))) => Ok(Some(MessageStatus::Confirmed)),
            Ok((Some(_), None)) => Ok(Some(MessageStatus::Delivered)),
            Ok((None, _)) => Ok(Some(MessageStatus::Pending)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    fn message_delivery_report(&self, prompt_id: i64) -> Result<Option<MessageDeliveryReport>> {
        let conn = self.conn.lock().unwrap();

        let row = conn.query_row(
            "SELECT id, source, target, created_at, processed_at, factory_session,
                    priority, acked_at, urgent, selected_at, last_pending_reason,
                    last_pending_detail, transport_delivered_at, highest_stage
             FROM prompt_queue WHERE id = ?",
            params![prompt_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, u8>(6).unwrap_or(2),
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8).map(|v| v != 0).unwrap_or(false),
                    row.get::<_, Option<String>>(9).unwrap_or(None),
                    row.get::<_, Option<String>>(10).unwrap_or(None),
                    row.get::<_, Option<String>>(11).unwrap_or(None),
                    row.get::<_, Option<String>>(12).unwrap_or(None),
                    row.get::<_, Option<String>>(13).unwrap_or(None),
                ))
            },
        );

        let (
            id,
            source,
            target,
            created_at_s,
            processed_at_s,
            factory_session,
            priority,
            acked_at_s,
            urgent,
            selected_at_s,
            stored_reason,
            stored_detail,
            transport_delivered_s,
            highest_stage_s,
        ) = match row {
            Ok(v) => v,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(e) => return Err(e.into()),
        };

        // Never fabricate timestamps — corrupt created_at is a hard error.
        let enqueued_at = Self::require_datetime(&created_at_s, "created_at", id)?;
        let selected_at = Self::optional_datetime(selected_at_s.as_deref(), "selected_at", id)?;
        let delivered_at = Self::optional_datetime(
            transport_delivered_s.as_deref(),
            "transport_delivered_at",
            id,
        )?;
        let confirmed_at = Self::optional_datetime(acked_at_s.as_deref(), "acked_at", id)?;
        // Legacy processed_at only for MessageStatus compatibility.
        let legacy_processed = processed_at_s.is_some();

        let legacy_status = if confirmed_at.is_some() {
            MessageStatus::Confirmed
        } else if legacy_processed {
            MessageStatus::Delivered
        } else {
            MessageStatus::Pending
        };

        let stored_pending = stored_reason.as_deref().and_then(PendingReason::parse);
        let mut stage = highest_stage_s
            .as_deref()
            .and_then(DeliveryStage::parse)
            .unwrap_or(DeliveryStage::Enqueued);

        // Reconcile from authoritative columns (still monotonic via advance).
        if confirmed_at.is_some() {
            stage = stage.advance(DeliveryStage::Confirmed);
        } else if delivered_at.is_some() {
            stage = stage.advance(DeliveryStage::Delivered);
        } else if let Some(reason) = stored_pending {
            stage = stage.advance(reason.implied_stage());
        } else if selected_at.is_some() {
            stage = stage.advance(DeliveryStage::Selected);
        }

        let (pending_reason, pending_detail) = match stage {
            DeliveryStage::Confirmed => (None, None),
            DeliveryStage::Delivered => (
                Some(PendingReason::AwaitingAck),
                Some("transport delivered; waiting for message_ack".into()),
            ),
            DeliveryStage::Dropped => (
                Some(stored_pending.unwrap_or(PendingReason::DroppedDeadSource)),
                stored_detail,
            ),
            DeliveryStage::Suppressed => (
                Some(stored_pending.unwrap_or(PendingReason::SuppressedIdle)),
                stored_detail,
            ),
            DeliveryStage::Abandoned => (
                Some(stored_pending.unwrap_or(PendingReason::AbandonedUnknownTarget)),
                stored_detail,
            ),
            DeliveryStage::Gated | DeliveryStage::Selected | DeliveryStage::Enqueued => {
                if let Some(reason) = stored_pending {
                    (Some(reason), stored_detail)
                } else {
                    // No false queue-head inference — less specific but truthful.
                    (
                        Some(PendingReason::AwaitingDelivery),
                        Some("awaiting authoritative delivery-path observation".into()),
                    )
                }
            }
        };

        Ok(Some(MessageDeliveryReport {
            id,
            legacy_status,
            stage,
            source,
            target,
            factory_session,
            priority,
            urgent,
            enqueued_at,
            selected_at,
            delivered_at,
            confirmed_at,
            pending_reason,
            pending_detail,
            wake: ObservationStatus::Unobserved,
            reaction: ObservationStatus::Unobserved,
        }))
    }

    fn record_selected(&self, prompt_id: i64) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            Self::advance_highest_stage(&conn, prompt_id, DeliveryStage::Selected)?;
            conn.execute(
                "UPDATE prompt_queue
                 SET selected_at = COALESCE(selected_at, ?)
                 WHERE id = ?",
                params![now, prompt_id],
            )?;
            Ok(())
        })
    }

    fn record_pending_reason(
        &self,
        prompt_id: i64,
        reason: PendingReason,
        detail: Option<&str>,
    ) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            // Monotonic: Gated stays Gated even if later AdapterRetryable is stamped.
            Self::advance_highest_stage(&conn, prompt_id, reason.implied_stage())?;
            conn.execute(
                "UPDATE prompt_queue
                 SET selected_at = COALESCE(selected_at, ?),
                     last_pending_reason = ?,
                     last_pending_detail = ?
                 WHERE id = ?",
                params![now, reason.as_str(), detail, prompt_id],
            )?;
            Ok(())
        })
    }

    fn mark_transport_delivered(&self, prompt_id: i64) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            Self::advance_highest_stage(&conn, prompt_id, DeliveryStage::Delivered)?;
            conn.execute(
                "UPDATE prompt_queue
                 SET transport_delivered_at = COALESCE(transport_delivered_at, ?),
                     processed_at = COALESCE(processed_at, ?),
                     selected_at = COALESCE(selected_at, ?),
                     last_pending_reason = NULL,
                     last_pending_detail = NULL
                 WHERE id = ?",
                params![now, now, now, prompt_id],
            )?;
            Ok(())
        })
    }

    fn mark_dropped(&self, prompt_id: i64, detail: Option<&str>) -> Result<()> {
        self.stamp_terminal_non_delivery(
            prompt_id,
            DeliveryStage::Dropped,
            PendingReason::DroppedDeadSource,
            detail,
        )
    }

    fn mark_suppressed(&self, prompt_id: i64, detail: Option<&str>) -> Result<()> {
        self.stamp_terminal_non_delivery(
            prompt_id,
            DeliveryStage::Suppressed,
            PendingReason::SuppressedIdle,
            detail,
        )
    }

    fn mark_abandoned(&self, prompt_id: i64, detail: Option<&str>) -> Result<()> {
        self.stamp_terminal_non_delivery(
            prompt_id,
            DeliveryStage::Abandoned,
            PendingReason::AbandonedUnknownTarget,
            detail,
        )
    }

    fn pending_count(&self) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM prompt_queue WHERE processed_at IS NULL",
            [],
            |row| row.get(0),
        )?;

        Ok(count as usize)
    }

    fn clear(&self) -> Result<usize> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let rows = conn.execute("DELETE FROM prompt_queue", [])?;
            Ok(rows)
        }) // with_write_retry
    }

    fn cleanup_old(&self, older_than_secs: i64) -> Result<usize> {
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let cutoff = (Utc::now() - chrono::Duration::seconds(older_than_secs)).to_rfc3339();

            let rows = conn.execute(
                "DELETE FROM prompt_queue WHERE processed_at IS NOT NULL AND processed_at < ?",
                params![cutoff],
            )?;

            Ok(rows)
        }) // with_write_retry
    }

    fn close(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::prompt_queue_store::*;
    use rusqlite::params;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SqlitePromptQueueStore) {
        let temp = TempDir::new().unwrap();
        let store = SqlitePromptQueueStore::open(temp.path()).unwrap();
        store.init().unwrap();
        (temp, store)
    }

    #[test]
    fn test_enqueue_and_poll() {
        let (_temp, store) = create_test_store();

        // Queue a prompt
        let id = store
            .enqueue("supervisor", "swift-fox", "Hello worker!")
            .unwrap();
        assert!(id > 0);

        // Poll should return it
        let prompts = store.poll_all(10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].target, "swift-fox");
        assert_eq!(prompts[0].prompt, "Hello worker!");

        // Polling again should return empty (already processed)
        let prompts = store.poll_all(10).unwrap();
        assert!(prompts.is_empty());
    }

    #[test]
    fn test_all_workers_target() {
        let (_temp, store) = create_test_store();

        // Queue to all_workers
        store
            .enqueue("supervisor", "all_workers", "Everyone listen up!")
            .unwrap();

        // Any worker should see it
        let prompts = store.poll_for_target("swift-fox", 10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].target, "all_workers");
    }

    #[test]
    fn test_peek_does_not_process() {
        let (_temp, store) = create_test_store();

        store.enqueue("supervisor", "worker-1", "Test").unwrap();

        // Peek should return prompt
        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);

        // Peek again should still return it
        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);

        // Pending count should be 1
        assert_eq!(store.pending_count().unwrap(), 1);
    }

    #[test]
    fn test_fifo_ordering() {
        let (_temp, store) = create_test_store();

        store.enqueue("supervisor", "worker", "First").unwrap();
        store.enqueue("supervisor", "worker", "Second").unwrap();
        store.enqueue("supervisor", "worker", "Third").unwrap();

        let prompts = store.poll_all(10).unwrap();
        assert_eq!(prompts.len(), 3);
        assert_eq!(prompts[0].prompt, "First");
        assert_eq!(prompts[1].prompt, "Second");
        assert_eq!(prompts[2].prompt, "Third");
    }

    #[test]
    fn test_retry_semantics_when_not_marked_processed() {
        let (_temp, store) = create_test_store();

        let prompt_id = store.enqueue("supervisor", "worker-1", "Retry me").unwrap();

        // Simulate failed injection: prompt is read via peek but not acked.
        let pending = store.peek_all(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(store.pending_count().unwrap(), 1);

        // Prompt remains available for retry.
        let retry_pending = store.peek_all(10).unwrap();
        assert_eq!(retry_pending.len(), 1);
        assert_eq!(retry_pending[0].id, prompt_id);

        // Simulate successful retry path by explicitly acknowledging it.
        store.mark_processed(prompt_id).unwrap();
        assert_eq!(store.pending_count().unwrap(), 0);
    }

    #[test]
    fn test_session_isolation_peek_for_targets() {
        let (_temp, store) = create_test_store();

        // Session A messages
        store
            .enqueue_with_session("supervisor-a", "worker-a1", "Task for A1", "session-a")
            .unwrap();
        store
            .enqueue_with_session("supervisor-a", "worker-a2", "Task for A2", "session-a")
            .unwrap();

        // Session B messages
        store
            .enqueue_with_session("supervisor-b", "worker-b1", "Task for B1", "session-b")
            .unwrap();

        // Legacy message (no session tag)
        store
            .enqueue("supervisor-a", "worker-a1", "Legacy msg")
            .unwrap();

        // Session A should only see its own messages + legacy for its targets
        let targets_a = &["supervisor-a", "worker-a1", "worker-a2", "all_workers"];
        let prompts_a = store
            .peek_for_targets(targets_a, Some("session-a"), 10)
            .unwrap();
        assert_eq!(prompts_a.len(), 3); // 2 session-tagged + 1 legacy by target match
        assert!(prompts_a.iter().all(|p| p.target != "worker-b1"));
        assert_eq!(
            prompts_a
                .iter()
                .filter(|p| p.factory_session.as_deref() == Some("session-a"))
                .count(),
            2
        );

        // Session B should only see its own messages
        let targets_b = &["supervisor-b", "worker-b1", "all_workers"];
        let prompts_b = store
            .peek_for_targets(targets_b, Some("session-b"), 10)
            .unwrap();
        assert_eq!(prompts_b.len(), 1);
        assert_eq!(prompts_b[0].target, "worker-b1");
        assert_eq!(prompts_b[0].factory_session.as_deref(), Some("session-b"));
    }

    #[test]
    fn test_enqueue_with_session_tags_correctly() {
        let (_temp, store) = create_test_store();

        store
            .enqueue_with_session("sup", "worker-1", "Hello", "my-session")
            .unwrap();

        // peek_all still sees it (no session filter)
        let all = store.peek_all(10).unwrap();
        assert_eq!(all.len(), 1);

        // A tagged row must not leak to another session even when target matches.
        let by_target = store
            .peek_for_targets(&["worker-1"], Some("other-session"), 10)
            .unwrap();
        assert_eq!(by_target.len(), 0);

        // peek_for_targets with wrong target but matching session still sees it
        let by_session = store
            .peek_for_targets(&["nonexistent"], Some("my-session"), 10)
            .unwrap();
        assert_eq!(by_session.len(), 1); // session match
        assert_eq!(by_session[0].factory_session.as_deref(), Some("my-session"));
    }

    #[test]
    fn test_tagged_delivery_does_not_cross_sessions_on_name_collision() {
        let (_temp, store) = create_test_store();

        store
            .enqueue_with_session("supervisor-a", "worker", "session A", "session-a")
            .unwrap();
        store
            .enqueue_with_session("supervisor-b", "worker", "session B", "session-b")
            .unwrap();
        store
            .enqueue("legacy-supervisor", "worker", "legacy")
            .unwrap();

        let session_a = store
            .peek_for_targets(&["worker"], Some("session-a"), 10)
            .unwrap();
        assert_eq!(session_a.len(), 2);
        assert!(session_a.iter().any(|p| p.prompt == "session A"));
        assert!(session_a.iter().any(|p| p.prompt == "legacy"));
        assert!(!session_a.iter().any(|p| p.prompt == "session B"));

        let session_b = store
            .poll_for_target_with_session("worker", Some("session-b"), 10)
            .unwrap();
        assert_eq!(session_b.len(), 2);
        assert!(session_b.iter().any(|p| p.prompt == "session B"));
        assert!(session_b.iter().any(|p| p.prompt == "legacy"));
        assert!(!session_b.iter().any(|p| p.prompt == "session A"));

        let remaining = store.peek_all(10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].prompt, "session A");
    }

    #[test]
    fn test_priority_ordering() {
        let (_temp, store) = create_test_store();

        // Enqueue in reverse priority order: normal first, then critical
        store
            .enqueue_full(
                "supervisor",
                "worker",
                "Normal update",
                None,
                None,
                Some(NotificationPriority::Normal),
            )
            .unwrap();
        store
            .enqueue_full(
                "supervisor",
                "worker",
                "Critical blocker",
                None,
                None,
                Some(NotificationPriority::Critical),
            )
            .unwrap();
        store
            .enqueue_full(
                "supervisor",
                "worker",
                "High priority",
                None,
                None,
                Some(NotificationPriority::High),
            )
            .unwrap();

        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 3);
        // Critical (0) should come first, then High (1), then Normal (2)
        assert_eq!(prompts[0].prompt, "Critical blocker");
        assert_eq!(prompts[0].priority, NotificationPriority::Critical);
        assert_eq!(prompts[1].prompt, "High priority");
        assert_eq!(prompts[1].priority, NotificationPriority::High);
        assert_eq!(prompts[2].prompt, "Normal update");
        assert_eq!(prompts[2].priority, NotificationPriority::Normal);
    }

    #[test]
    fn test_default_priority_is_normal() {
        let (_temp, store) = create_test_store();

        store
            .enqueue("supervisor", "worker", "Default priority")
            .unwrap();

        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].priority, NotificationPriority::Normal);
    }

    #[test]
    fn test_priority_with_peek_for_targets() {
        let (_temp, store) = create_test_store();

        store
            .enqueue_full(
                "worker",
                "supervisor",
                "Status update",
                Some("session-1"),
                None,
                Some(NotificationPriority::Normal),
            )
            .unwrap();
        store
            .enqueue_full(
                "worker",
                "supervisor",
                "BLOCKED: need help",
                Some("session-1"),
                None,
                Some(NotificationPriority::High),
            )
            .unwrap();

        let prompts = store
            .peek_for_targets(&["supervisor"], Some("session-1"), 10)
            .unwrap();
        assert_eq!(prompts.len(), 2);
        // High priority should come first
        assert_eq!(prompts[0].prompt, "BLOCKED: need help");
        assert_eq!(prompts[1].prompt, "Status update");
    }

    #[test]
    fn test_ack_delivery_confirmation() {
        let (_temp, store) = create_test_store();

        let id = store.enqueue("supervisor", "worker-1", "Do task").unwrap();

        // Initially pending
        let status = store.message_status(id).unwrap();
        assert_eq!(status, Some(MessageStatus::Pending));

        // Mark as processed (delivered)
        store.mark_processed(id).unwrap();
        let status = store.message_status(id).unwrap();
        assert_eq!(status, Some(MessageStatus::Delivered));

        // Ack (confirmed)
        store.ack(id).unwrap();
        let status = store.message_status(id).unwrap();
        assert_eq!(status, Some(MessageStatus::Confirmed));

        // Ack is idempotent
        store.ack(id).unwrap();

        // Peek shows acked_at is set
        let prompts = store.poll_for_target("worker-1", 10).unwrap();
        assert!(prompts.is_empty()); // already processed
    }

    #[test]
    fn test_ack_nonexistent_is_idempotent() {
        let (_temp, store) = create_test_store();
        // Acking a nonexistent prompt is idempotent — no error
        let result = store.ack(99999);
        assert!(result.is_ok());
    }

    #[test]
    fn test_message_status_nonexistent() {
        let (_temp, store) = create_test_store();
        let status = store.message_status(99999).unwrap();
        assert_eq!(status, None);
    }

    #[test]
    fn test_unacked_timeout() {
        let (_temp, store) = create_test_store();

        let id1 = store.enqueue("supervisor", "worker-1", "Msg 1").unwrap();
        let id2 = store.enqueue("supervisor", "worker-2", "Msg 2").unwrap();

        // Process both
        store.mark_processed(id1).unwrap();
        store.mark_processed(id2).unwrap();

        // Ack only one
        store.ack(id2).unwrap();

        // With timeout=0, all delivered-but-unacked messages should appear
        let unacked = store.unacked(0, 10).unwrap();
        assert_eq!(unacked.len(), 1);
        assert_eq!(unacked[0].id, id1);
        assert_eq!(unacked[0].prompt, "Msg 1");
    }

    #[test]
    fn test_unacked_respects_timeout() {
        let (_temp, store) = create_test_store();

        let id = store.enqueue("supervisor", "worker-1", "Recent").unwrap();
        store.mark_processed(id).unwrap();

        // With a large timeout, the recently processed message should NOT appear
        let unacked = store.unacked(3600, 10).unwrap();
        assert!(unacked.is_empty());
    }

    // ---- cas-c931: urgent (interrupt-and-redirect) flag ----

    #[test]
    fn test_enqueue_full_defaults_urgent_false() {
        let (_temp, store) = create_test_store();
        store
            .enqueue_full("supervisor", "worker", "Normal note", None, None, None)
            .unwrap();
        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(
            !prompts[0].urgent,
            "non-urgent enqueue_full must default urgent=false"
        );
    }

    #[test]
    fn test_enqueue_urgent_roundtrips() {
        let (_temp, store) = create_test_store();
        let id = store
            .enqueue_urgent(
                "supervisor",
                "worker",
                "STOP — you are editing the wrong file",
                Some("sess-1"),
                Some("redirect"),
                Some(NotificationPriority::Critical),
                true,
            )
            .unwrap();
        assert!(id > 0);

        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert!(prompts[0].urgent, "urgent flag must round-trip as true");
        assert_eq!(prompts[0].priority, NotificationPriority::Critical);

        // Also visible via the session/target peek used by the daemon.
        let by_target = store
            .peek_for_targets(&["worker"], Some("sess-1"), 10)
            .unwrap();
        assert_eq!(by_target.len(), 1);
        assert!(by_target[0].urgent);
    }

    #[test]
    fn test_urgent_and_normal_coexist() {
        let (_temp, store) = create_test_store();
        store
            .enqueue_full("supervisor", "worker", "fyi", None, None, None)
            .unwrap();
        store
            .enqueue_urgent(
                "supervisor",
                "worker",
                "abort now",
                None,
                None,
                Some(NotificationPriority::Critical),
                true,
            )
            .unwrap();

        let prompts = store.poll_for_target("worker", 10).unwrap();
        assert_eq!(prompts.len(), 2);
        // Critical/urgent should sort ahead of the normal note.
        assert_eq!(prompts[0].prompt, "abort now");
        assert!(prompts[0].urgent);
        assert_eq!(prompts[1].prompt, "fyi");
        assert!(!prompts[1].urgent);
    }

    #[test]
    fn test_urgent_column_migration_on_legacy_table() {
        // Simulate a pre-cas-c931 prompt_queue table (no urgent column) and
        // confirm init() adds the column non-destructively and old rows read
        // back as urgent=false.
        let temp = TempDir::new().unwrap();
        let db_path = temp.path().join("cas.db");
        {
            let conn = rusqlite::Connection::open(&db_path).unwrap();
            conn.execute_batch(
                "CREATE TABLE prompt_queue (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    source TEXT NOT NULL,
                    target TEXT NOT NULL,
                    prompt TEXT NOT NULL,
                    created_at TEXT NOT NULL DEFAULT (datetime('now')),
                    processed_at TEXT,
                    factory_session TEXT,
                    summary TEXT,
                    priority INTEGER NOT NULL DEFAULT 2,
                    acked_at TEXT
                );",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO prompt_queue (source, target, prompt, created_at) VALUES ('s','w','legacy', datetime('now'))",
                [],
            )
            .unwrap();
        }

        let store = SqlitePromptQueueStore::open(temp.path()).unwrap();
        store.init().unwrap(); // must add the urgent column without dropping the legacy row

        let prompts = store.peek_all(10).unwrap();
        assert_eq!(prompts.len(), 1);
        assert_eq!(prompts[0].prompt, "legacy");
        assert!(
            !prompts[0].urgent,
            "legacy rows must read back urgent=false"
        );

        // New urgent inserts work after migration.
        store
            .enqueue_urgent("s", "w", "new urgent", None, None, None, true)
            .unwrap();
        let prompts = store.poll_for_target("w", 10).unwrap();
        assert!(prompts.iter().any(|p| p.prompt == "new urgent" && p.urgent));
    }

    /// cas-2bcb / cas-04a6 R1: lower-ID NULL-session legacy rows must not
    /// occupy the fetch LIMIT ahead of eligible live-session rows.
    #[test]
    fn test_live_session_not_starved_by_legacy_null_session_hol() {
        let (_temp, store) = create_test_store();
        const LIMIT: usize = 10;

        for i in 0..(LIMIT + 5) {
            store
                .enqueue("old-worker", "supervisor", &format!("legacy backlog {i}"))
                .unwrap();
        }

        let live_id = store
            .enqueue_with_session(
                "worker-live",
                "supervisor",
                "live session coordination",
                "session-live",
            )
            .unwrap();

        let peeked = store
            .peek_for_targets(
                &["supervisor", "director", "all_workers", "worker-live"],
                Some("session-live"),
                LIMIT,
            )
            .unwrap();

        assert_eq!(
            peeked.len(),
            LIMIT,
            "peek must still respect the caller LIMIT"
        );
        assert!(
            peeked.iter().any(|p| p.id == live_id),
            "live-session row must appear in one peek despite >LIMIT lower-ID NULL-session backlog; got ids {:?}",
            peeked.iter().map(|p| p.id).collect::<Vec<_>>()
        );
        assert!(
            peeked.iter().any(|p| p.factory_session.is_none()),
            "legacy NULL-session rows remain eligible under the two-lane quota"
        );
    }

    /// Symmetric fairness: sustained live-session traffic must not starve
    /// eligible legacy NULL-session rows either (supervisor reject of pure
    /// session-first ordering).
    #[test]
    fn test_legacy_not_starved_by_live_session_traffic() {
        let (_temp, store) = create_test_store();
        const LIMIT: usize = 10;

        for i in 0..(LIMIT + 5) {
            store
                .enqueue_with_session(
                    "worker",
                    "supervisor",
                    &format!("live backlog {i}"),
                    "session-live",
                )
                .unwrap();
        }
        let legacy_id = store.enqueue("old", "supervisor", "lonely legacy").unwrap();

        let peeked = store
            .peek_for_targets(&["supervisor"], Some("session-live"), LIMIT)
            .unwrap();

        assert_eq!(peeked.len(), LIMIT);
        assert!(
            peeked.iter().any(|p| p.id == legacy_id),
            "legacy row must appear in one peek despite >LIMIT live-session backlog; got {:?}",
            peeked
                .iter()
                .map(|p| (p.id, p.factory_session.as_deref()))
                .collect::<Vec<_>>()
        );
        assert!(
            peeked
                .iter()
                .any(|p| p.factory_session.as_deref() == Some("session-live")),
            "session lane also represented"
        );
    }

    /// Repeated peek+mark batches: both lanes drain with bounded progress
    /// (neither lane stuck forever while the other has work).
    #[test]
    fn test_two_lane_bounded_progress_across_repeated_peeks() {
        let (_temp, store) = create_test_store();
        const LIMIT: usize = 10;
        const PER_LANE: usize = 25;

        let mut session_ids = Vec::new();
        let mut legacy_ids = Vec::new();
        for i in 0..PER_LANE {
            session_ids.push(
                store
                    .enqueue_with_session("w", "supervisor", &format!("sess {i}"), "session-a")
                    .unwrap(),
            );
            legacy_ids.push(
                store
                    .enqueue("old", "supervisor", &format!("leg {i}"))
                    .unwrap(),
            );
        }

        let mut session_seen = 0usize;
        let mut legacy_seen = 0usize;
        let mut rounds = 0usize;
        loop {
            let batch = store
                .peek_for_targets(&["supervisor"], Some("session-a"), LIMIT)
                .unwrap();
            if batch.is_empty() {
                break;
            }
            rounds += 1;
            assert!(rounds <= PER_LANE * 2, "must drain without infinite loop");

            let sess_in_batch = batch
                .iter()
                .filter(|p| p.factory_session.as_deref() == Some("session-a"))
                .count();
            let leg_in_batch = batch.iter().filter(|p| p.factory_session.is_none()).count();
            // While both lanes still have residual work beyond this batch,
            // each peek must take from both (bounded dual progress).
            let session_remaining_before = PER_LANE - session_seen;
            let legacy_remaining_before = PER_LANE - legacy_seen;
            if session_remaining_before > 0 && legacy_remaining_before > 0 && LIMIT >= 2 {
                assert!(
                    sess_in_batch >= 1 && leg_in_batch >= 1,
                    "round {rounds}: both lanes must progress while both pending; \
                     session={sess_in_batch} legacy={leg_in_batch}"
                );
            }
            session_seen += sess_in_batch;
            legacy_seen += leg_in_batch;

            // Within each lane contribution, FIFO by id.
            let sess_ids: Vec<i64> = batch
                .iter()
                .filter(|p| p.factory_session.as_deref() == Some("session-a"))
                .map(|p| p.id)
                .collect();
            assert!(
                sess_ids.windows(2).all(|w| w[0] < w[1]),
                "session lane FIFO violated: {sess_ids:?}"
            );
            let leg_ids: Vec<i64> = batch
                .iter()
                .filter(|p| p.factory_session.is_none())
                .map(|p| p.id)
                .collect();
            assert!(
                leg_ids.windows(2).all(|w| w[0] < w[1]),
                "legacy lane FIFO violated: {leg_ids:?}"
            );

            // Priority non-decreasing across the merged delivery set.
            for window in batch.windows(2) {
                assert!(window[0].priority as u8 <= window[1].priority as u8);
            }

            for p in &batch {
                store.mark_processed(p.id).unwrap();
            }
        }

        assert_eq!(session_seen, PER_LANE, "all session rows drained");
        assert_eq!(legacy_seen, PER_LANE, "all legacy rows drained");
    }

    /// Priority authoritative across lanes; FIFO within lane; isolation holds.
    #[test]
    fn test_peek_for_targets_priority_fifo_and_isolation_with_legacy() {
        let (_temp, store) = create_test_store();

        store
            .enqueue_with_session("sup-b", "worker", "other session", "session-b")
            .unwrap();

        let first = store
            .enqueue_with_session("worker-a", "supervisor", "live first", "session-a")
            .unwrap();
        let second = store
            .enqueue_with_session("worker-a", "supervisor", "live second", "session-a")
            .unwrap();
        let urgent = store
            .enqueue_urgent(
                "worker-a",
                "supervisor",
                "live urgent",
                Some("session-a"),
                Some("urgent"),
                Some(NotificationPriority::Critical),
                true,
            )
            .unwrap();

        for i in 0..15 {
            store
                .enqueue("old", "supervisor", &format!("legacy tail {i}"))
                .unwrap();
        }

        let peeked = store
            .peek_for_targets(&["supervisor", "worker"], Some("session-a"), 10)
            .unwrap();

        assert!(
            !peeked
                .iter()
                .any(|p| p.factory_session.as_deref() == Some("session-b")),
            "other session must not leak"
        );

        let live: Vec<_> = peeked
            .iter()
            .filter(|p| p.factory_session.as_deref() == Some("session-a"))
            .collect();
        assert_eq!(
            live.len(),
            3,
            "all three session-a rows must fit under quota"
        );
        assert_eq!(live[0].id, urgent, "urgent eligible precedes normal");
        assert_eq!(live[1].id, first, "equal-priority FIFO: first then second");
        assert_eq!(live[2].id, second);

        for window in peeked.windows(2) {
            assert!(
                window[0].priority as u8 <= window[1].priority as u8,
                "priority order violated: {:?} then {:?}",
                window[0].priority,
                window[1].priority
            );
        }
    }

    /// Pure merge helper: global priority before same-priority lane fairness.
    #[test]
    fn test_merge_two_lane_peeks_quota_contract() {
        fn row(id: i64, session: Option<&str>, priority: u8) -> QueuedPrompt {
            QueuedPrompt {
                id,
                source: "s".into(),
                target: "t".into(),
                prompt: format!("p{id}"),
                created_at: Utc::now(),
                processed_at: None,
                factory_session: session.map(|s| s.to_string()),
                summary: None,
                priority: NotificationPriority::from(priority),
                acked_at: None,
                urgent: priority == 0,
            }
        }

        // Equal priority both lanes full, limit 10 → 5+5 lane fairness.
        let session: Vec<_> = (1..=20).map(|i| row(i, Some("s"), 2)).collect();
        let legacy: Vec<_> = (100..=119).map(|i| row(i, None, 2)).collect();
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert_eq!(merged.len(), 10);
        assert_eq!(
            merged
                .iter()
                .filter(|p| p.factory_session.is_some())
                .count(),
            5
        );
        assert_eq!(
            merged
                .iter()
                .filter(|p| p.factory_session.is_none())
                .count(),
            5
        );

        // One live + many legacy (same priority) → live included; remainder legacy.
        let session = vec![row(50, Some("s"), 2)];
        let legacy: Vec<_> = (1..=20).map(|i| row(i, None, 2)).collect();
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert!(merged.iter().any(|p| p.id == 50));
        assert_eq!(merged.len(), 10);
        assert_eq!(
            merged
                .iter()
                .filter(|p| p.factory_session.is_none())
                .count(),
            9
        );

        // CRITICAL: session has 10 Critical, legacy 10 Normal, limit=10 →
        // all 10 Critical; zero Normal (global priority before lane quota).
        let session: Vec<_> = (1..=10).map(|i| row(i, Some("s"), 0)).collect();
        let legacy: Vec<_> = (100..=109).map(|i| row(i, None, 2)).collect();
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert_eq!(merged.len(), 10);
        assert!(
            merged
                .iter()
                .all(|p| p.priority == NotificationPriority::Critical),
            "must not admit Normal while Critical remains; got {:?}",
            merged
                .iter()
                .map(|p| (p.id, p.priority as u8))
                .collect::<Vec<_>>()
        );

        // Symmetric: session 10 Normal, legacy 10 Critical → all Critical.
        let session: Vec<_> = (1..=10).map(|i| row(i, Some("s"), 2)).collect();
        let legacy: Vec<_> = (100..=109).map(|i| row(i, None, 0)).collect();
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert_eq!(merged.len(), 10);
        assert!(
            merged
                .iter()
                .all(|p| p.priority == NotificationPriority::Critical)
        );

        // limit=1: Critical legacy beats Normal session.
        let session = vec![row(2, Some("s"), 2)];
        let legacy = vec![row(1, None, 0)];
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, 1);
        assert_eq!(merged[0].priority, NotificationPriority::Critical);

        // limit=1 equal priority → session preferred (same-priority fairness).
        let session = vec![row(2, Some("s"), 2)];
        let legacy = vec![row(1, None, 2)];
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 1);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, 2);
        assert!(merged[0].factory_session.is_some());
    }

    /// Store-level asymmetric priority + limit=1 regressions (supervisor gate).
    #[test]
    fn test_global_priority_before_lane_fairness_asymmetric_and_limit_one() {
        let (_temp, store) = create_test_store();

        // 10 Critical session + 10 Normal legacy → peek LIMIT 10 is all Critical.
        for i in 0..10 {
            store
                .enqueue_urgent(
                    "w",
                    "supervisor",
                    &format!("crit-sess {i}"),
                    Some("session-p"),
                    None,
                    Some(NotificationPriority::Critical),
                    true,
                )
                .unwrap();
        }
        for i in 0..10 {
            store
                .enqueue("old", "supervisor", &format!("norm-leg {i}"))
                .unwrap();
        }
        let peeked = store
            .peek_for_targets(&["supervisor"], Some("session-p"), 10)
            .unwrap();
        assert_eq!(peeked.len(), 10);
        assert!(
            peeked
                .iter()
                .all(|p| p.priority == NotificationPriority::Critical
                    && p.factory_session.as_deref() == Some("session-p")),
            "Critical session must fill LIMIT before any Normal legacy; got {:?}",
            peeked
                .iter()
                .map(|p| (p.priority as u8, p.factory_session.as_deref()))
                .collect::<Vec<_>>()
        );

        // Fresh store: Normal session flood + Critical legacy heads.
        let (_temp2, store2) = create_test_store();
        for i in 0..10 {
            store2
                .enqueue_with_session("w", "supervisor", &format!("norm {i}"), "session-p")
                .unwrap();
        }
        for i in 0..10 {
            store2
                .enqueue_urgent(
                    "old",
                    "supervisor",
                    &format!("crit-leg {i}"),
                    None,
                    None,
                    Some(NotificationPriority::Critical),
                    true,
                )
                .unwrap();
        }
        let peeked = store2
            .peek_for_targets(&["supervisor"], Some("session-p"), 10)
            .unwrap();
        assert_eq!(peeked.len(), 10);
        assert!(
            peeked.iter().all(
                |p| p.priority == NotificationPriority::Critical && p.factory_session.is_none()
            ),
            "Critical legacy must fill LIMIT before Normal session; got {:?}",
            peeked
                .iter()
                .map(|p| (p.priority as u8, p.factory_session.as_deref()))
                .collect::<Vec<_>>()
        );

        // limit=1: Critical legacy over Normal session.
        let (_temp3, store3) = create_test_store();
        store3
            .enqueue_with_session("w", "supervisor", "normal live", "session-p")
            .unwrap();
        let crit_id = store3
            .enqueue_urgent(
                "old",
                "supervisor",
                "critical legacy",
                None,
                None,
                Some(NotificationPriority::Critical),
                true,
            )
            .unwrap();
        let peeked = store3
            .peek_for_targets(&["supervisor"], Some("session-p"), 1)
            .unwrap();
        assert_eq!(peeked.len(), 1);
        assert_eq!(
            peeked[0].id, crit_id,
            "limit=1 must pick Critical legacy over Normal session"
        );
    }

    /// Scale + query plan: 10× backlog; EXPLAIN must name the expected indexes
    /// and must not use SCAN prompt_queue or USE TEMP B-TREE for the lane queries.
    #[test]
    fn test_two_lane_peek_scale_and_query_plan() {
        let (_temp, store) = create_test_store();
        const LIMIT: usize = 10;
        const BACKLOG: usize = LIMIT * 10;

        for i in 0..BACKLOG {
            store
                .enqueue_with_session("w", "supervisor", &format!("s{i}"), "session-scale")
                .unwrap();
            store
                .enqueue("old", "supervisor", &format!("l{i}"))
                .unwrap();
        }

        let started = std::time::Instant::now();
        let peeked = store
            .peek_for_targets(&["supervisor"], Some("session-scale"), LIMIT)
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(peeked.len(), LIMIT);
        assert!(
            peeked
                .iter()
                .any(|p| p.factory_session.as_deref() == Some("session-scale"))
        );
        assert!(peeked.iter().any(|p| p.factory_session.is_none()));
        assert!(
            elapsed < std::time::Duration::from_millis(500),
            "peek over 10× backlog must stay fast; took {elapsed:?}"
        );

        fn explain_plan(
            conn: &rusqlite::Connection,
            sql: &str,
            params: &[&dyn rusqlite::ToSql],
        ) -> String {
            let mut plan = String::new();
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
            let rows = stmt
                .query_map(params, |row| Ok(row.get::<_, String>(3)?))
                .unwrap();
            for r in rows {
                plan.push_str(&r.unwrap());
                plan.push('\n');
            }
            plan
        }

        fn assert_index_plan(plan: &str, expected_index: &str, lane: &str) {
            let lower = plan.to_lowercase();
            assert!(
                plan.contains(expected_index),
                "{lane} plan must name {expected_index}; plan was:\n{plan}"
            );
            assert!(
                lower.contains("search") || lower.contains("using index"),
                "{lane} plan must SEARCH via index; plan was:\n{plan}"
            );
            // Reject full table scan of prompt_queue and temp sort materialization.
            assert!(
                !lower.contains("scan prompt_queue"),
                "{lane} plan must not SCAN prompt_queue; plan was:\n{plan}"
            );
            assert!(
                !lower.contains("use temp b-tree"),
                "{lane} plan must not USE TEMP B-TREE; plan was:\n{plan}"
            );
        }

        let conn = store.conn.lock().unwrap();
        let session_plan = explain_plan(
            &conn,
            "SELECT id FROM prompt_queue
             WHERE processed_at IS NULL AND factory_session = ?
             ORDER BY priority ASC, id ASC
             LIMIT ?",
            &[&"session-scale", &(LIMIT as i64)],
        );
        let legacy_plan = explain_plan(
            &conn,
            "SELECT id FROM prompt_queue
             WHERE processed_at IS NULL
               AND factory_session IS NULL
               AND target IN (?)
             ORDER BY priority ASC, id ASC
             LIMIT ?",
            &[&"supervisor", &(LIMIT as i64)],
        );

        assert_index_plan(&session_plan, "idx_prompt_queue_session_pending", "session");
        assert_index_plan(&legacy_plan, "idx_prompt_queue_legacy_pending", "legacy");
    }

    /// cas-2c5f: enqueued → selected → transport delivered → confirmed.
    /// mark_processed alone must NOT yield structured Delivered.
    #[test]
    fn test_message_delivery_report_stage_ladder_authoritative_delivery() {
        let (_temp, store) = create_test_store();
        let id = store
            .enqueue_with_session("sup", "worker", "hello", "sess-1")
            .unwrap();

        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.legacy_status, MessageStatus::Pending);
        assert_eq!(r.stage, DeliveryStage::Enqueued);
        assert_eq!(r.pending_reason, Some(PendingReason::AwaitingDelivery));
        assert_eq!(r.wake, ObservationStatus::Unobserved);
        assert_eq!(r.reaction, ObservationStatus::Unobserved);
        assert!(r.delivered_at.is_none());

        store.record_selected(id).unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Selected);

        // Legacy mark_processed (queue drain) is NOT transport success.
        store.mark_processed(id).unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(
            r.legacy_status,
            MessageStatus::Delivered,
            "legacy ladder still uses processed_at"
        );
        assert_ne!(
            r.stage,
            DeliveryStage::Delivered,
            "structured Delivered requires transport_delivered_at"
        );
        assert!(r.delivered_at.is_none());

        // Authoritative handoff.
        store.mark_transport_delivered(id).unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Delivered);
        assert_eq!(r.pending_reason, Some(PendingReason::AwaitingAck));
        assert!(r.delivered_at.is_some());

        store.ack(id).unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Confirmed);
        assert_eq!(r.legacy_status, MessageStatus::Confirmed);
        assert!(r.pending_reason.is_none());
        assert_eq!(r.wake, ObservationStatus::Unobserved);
    }

    /// Gated must not regress to Selected when adapter failure is later stamped.
    #[test]
    fn test_message_delivery_report_stage_monotonic_gated_then_adapter() {
        let (_temp, store) = create_test_store();
        let id = store.enqueue("s", "worker", "body").unwrap();

        store
            .record_pending_reason(
                id,
                PendingReason::GatedNotReady,
                Some("pane not ready for injection"),
            )
            .unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Gated);

        store
            .record_pending_reason(
                id,
                PendingReason::AdapterRetryable,
                Some("inject failed: broken pipe"),
            )
            .unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(
            r.stage,
            DeliveryStage::Gated,
            "stage must not regress Gated→Selected on adapter retry"
        );
        assert_eq!(r.pending_reason, Some(PendingReason::AdapterRetryable));
        assert!(r.delivered_at.is_none());
    }

    #[test]
    fn test_delivery_path_drop_suppress_abandon_never_transport_delivered() {
        let (_temp, store) = create_test_store();

        let drop_id = store.enqueue("dead-w", "supervisor", "from dead").unwrap();
        store
            .mark_dropped(drop_id, Some("source worker is dead"))
            .unwrap();
        let r = store.message_delivery_report(drop_id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Dropped);
        assert_eq!(r.pending_reason, Some(PendingReason::DroppedDeadSource));
        assert!(r.delivered_at.is_none());
        assert_eq!(r.legacy_status, MessageStatus::Delivered); // processed_at set

        let sup_id = store.enqueue("w", "supervisor", "standing by").unwrap();
        store
            .mark_suppressed(sup_id, Some("duplicate idle"))
            .unwrap();
        let r = store.message_delivery_report(sup_id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Suppressed);
        assert!(r.delivered_at.is_none());

        let ab_id = store.enqueue("s", "ghost-worker", "hi").unwrap();
        store
            .mark_abandoned(ab_id, Some("target not in session"))
            .unwrap();
        let r = store.message_delivery_report(ab_id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Abandoned);
        assert!(r.delivered_at.is_none());
    }

    #[test]
    fn test_success_transport_delivery_stamps_delivered() {
        let (_temp, store) = create_test_store();
        let id = store.enqueue("sup", "worker", "do work").unwrap();
        store.record_selected(id).unwrap();
        store.mark_transport_delivered(id).unwrap();
        let r = store.message_delivery_report(id).unwrap().unwrap();
        assert_eq!(r.stage, DeliveryStage::Delivered);
        assert!(r.delivered_at.is_some());
        assert_eq!(r.legacy_status, MessageStatus::Delivered);
        assert_eq!(r.pending_reason, Some(PendingReason::AwaitingAck));
    }

    #[test]
    fn test_message_delivery_report_no_false_queue_head() {
        // Lower-id peer must not invent behind_queue_head (removed inaccurate heuristic).
        let (_temp, store) = create_test_store();
        let _first = store
            .enqueue_with_session("w", "supervisor", "first", "sess")
            .unwrap();
        let second = store
            .enqueue_with_session("w", "supervisor", "second", "sess")
            .unwrap();
        let r = store.message_delivery_report(second).unwrap().unwrap();
        assert_eq!(r.pending_reason, Some(PendingReason::AwaitingDelivery));
        assert_ne!(
            r.pending_reason.map(|p| p.as_str()),
            Some("behind_queue_head")
        );
    }

    #[test]
    fn test_message_delivery_report_unknown_id() {
        let (_temp, store) = create_test_store();
        assert!(store.message_delivery_report(999_999).unwrap().is_none());
        assert!(store.message_status(999_999).unwrap().is_none());
    }

    #[test]
    fn test_message_delivery_report_corrupt_created_at_errors() {
        let (_temp, store) = create_test_store();
        let id = store.enqueue("s", "w", "x").unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE prompt_queue SET created_at = 'not-a-timestamp' WHERE id = ?",
                params![id],
            )
            .unwrap();
        }
        let err = store.message_delivery_report(id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("corrupt") || msg.contains("unparseable") || msg.contains("parse"),
            "expected parse error, got {msg}"
        );
    }
}
