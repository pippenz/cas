//! Prompt queue for supervisor → worker communication in factory sessions
//!
//! Allows supervisor agents to send prompts to workers via MCP.
//! Factory TUI polls this queue and injects prompts into worker PTYs.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, params};
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

/// Delivery status of a prompt queue message
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
    /// Two independent ordered peeks (each `ORDER BY priority ASC, id ASC`)
    /// are merged with a **bounded two-lane quota** so neither lane can
    /// permanently occupy the entire LIMIT window (cas-2bcb / cas-04a6 R1):
    /// - When both lanes have pending work and `limit >= 2`: reserve
    ///   `ceil(limit/2)` slots for the session lane and `floor(limit/2)` for
    ///   the legacy lane; unused quota is filled from the other lane.
    /// - When only one lane has work: that lane may use the full LIMIT.
    /// - Final delivery order: `priority ASC, id ASC` across the selected
    ///   set (priority is authoritative across lanes; FIFO is stable
    ///   *within* each lane's contribution).
    /// - `limit == 1` with both lanes pending: session lane wins the single
    ///   slot (live coordination is the hot path); legacy still progresses
    ///   on subsequent peeks once session pressure drops or limit >= 2.
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

    /// Get delivery status of a specific message
    fn message_status(&self, prompt_id: i64) -> Result<Option<MessageStatus>>;

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

    /// Merge session-lane and legacy-lane peeks with a bounded two-lane quota.
    ///
    /// Contract (see `PromptQueueStore::peek_for_targets`):
    /// - both non-empty + limit>=2 → ceil(n/2) session reserve, floor(n/2) legacy reserve
    /// - unused reserve fills from the other lane
    /// - final order: priority ASC, id ASC
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

        let session_quota = (limit + 1) / 2;
        let legacy_quota = limit / 2;

        let mut session_iter = session.into_iter();
        let mut legacy_iter = legacy.into_iter();
        let mut selected: Vec<QueuedPrompt> = Vec::with_capacity(limit);

        for _ in 0..session_quota {
            match session_iter.next() {
                Some(row) => selected.push(row),
                None => break,
            }
        }
        for _ in 0..legacy_quota {
            match legacy_iter.next() {
                Some(row) => selected.push(row),
                None => break,
            }
        }

        // Remainder: merge leftover heads by priority then id (same key as final sort).
        while selected.len() < limit {
            let next = match (session_iter.as_slice().first(), legacy_iter.as_slice().first()) {
                (None, None) => break,
                (Some(_), None) => session_iter.next(),
                (None, Some(_)) => legacy_iter.next(),
                (Some(s), Some(l)) => {
                    let take_session = match (s.priority as u8).cmp(&(l.priority as u8)) {
                        std::cmp::Ordering::Less => true,
                        std::cmp::Ordering::Greater => false,
                        std::cmp::Ordering::Equal => s.id <= l.id,
                    };
                    if take_session {
                        session_iter.next()
                    } else {
                        legacy_iter.next()
                    }
                }
            };
            match next {
                Some(row) => selected.push(row),
                None => break,
            }
        }

        selected.sort_by(|a, b| {
            (a.priority as u8)
                .cmp(&(b.priority as u8))
                .then(a.id.cmp(&b.id))
        });
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
                    "UPDATE prompt_queue SET processed_at = ? WHERE id IN ({})",
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
                    "UPDATE prompt_queue SET processed_at = ? WHERE id IN ({})",
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
        let session_params: Vec<Box<dyn rusqlite::ToSql>> = vec![
            Box::new(session.to_string()),
            Box::new(limit as i64),
        ];
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

        assert_eq!(peeked.len(), LIMIT, "peek must still respect the caller LIMIT");
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
        let legacy_id = store
            .enqueue("old", "supervisor", "lonely legacy")
            .unwrap();

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
                    .enqueue_with_session(
                        "w",
                        "supervisor",
                        &format!("sess {i}"),
                        "session-a",
                    )
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
        assert_eq!(live.len(), 3, "all three session-a rows must fit under quota");
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

    /// Pure merge helper unit: documents the intentional cross-lane contract.
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

        // Both lanes full, limit 10 → 5+5 reserve.
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
        assert_eq!(merged.iter().filter(|p| p.factory_session.is_none()).count(), 5);

        // One live + many legacy → live always included; remainder fills legacy.
        let session = vec![row(50, Some("s"), 2)];
        let legacy: Vec<_> = (1..=20).map(|i| row(i, None, 2)).collect();
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert!(merged.iter().any(|p| p.id == 50));
        assert_eq!(merged.len(), 10);
        assert_eq!(merged.iter().filter(|p| p.factory_session.is_none()).count(), 9);

        // Urgent legacy outranks normal session in final order.
        let session = vec![row(2, Some("s"), 2)];
        let legacy = vec![row(1, None, 0)];
        let merged = SqlitePromptQueueStore::merge_two_lane_peeks(session, legacy, 10);
        assert_eq!(merged[0].id, 1);
        assert_eq!(merged[0].priority, NotificationPriority::Critical);
        assert_eq!(merged[1].id, 2);
    }

    /// Scale + query plan: 10× backlog peeks stay LIMIT-bounded; EXPLAIN uses
    /// the partial two-lane indexes rather than a full table scan of all rows.
    #[test]
    fn test_two_lane_peek_scale_and_query_plan() {
        let (_temp, store) = create_test_store();
        const LIMIT: usize = 10;
        // 10× the daemon LIMIT on each lane (cas-2bcb scalability AC).
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

        // EXPLAIN QUERY PLAN for each lane — expect index usage on the
        // partial two-lane indexes (not a full scan of every prompt_queue row).
        let conn = store.conn.lock().unwrap();
        let mut session_plan = String::new();
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM prompt_queue
                 WHERE processed_at IS NULL AND factory_session = ?
                 ORDER BY priority ASC, id ASC
                 LIMIT ?",
            )
            .unwrap();
        let rows = stmt
            .query_map(params!["session-scale", LIMIT as i64], |row| {
                Ok(format!(
                    "{}|{}|{}|{}",
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?
                ))
            })
            .unwrap();
        for r in rows {
            session_plan.push_str(&r.unwrap());
            session_plan.push('\n');
        }

        let mut legacy_plan = String::new();
        let mut stmt = conn
            .prepare(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM prompt_queue
                 WHERE processed_at IS NULL
                   AND factory_session IS NULL
                   AND target IN (?)
                 ORDER BY priority ASC, id ASC
                 LIMIT ?",
            )
            .unwrap();
        let rows = stmt
            .query_map(params!["supervisor", LIMIT as i64], |row| {
                Ok(format!(
                    "{}|{}|{}|{}",
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?
                ))
            })
            .unwrap();
        for r in rows {
            legacy_plan.push_str(&r.unwrap());
            legacy_plan.push('\n');
        }

        // Plans must mention an index (partial two-lane indexes) and must not
        // be an unconstrained SCAN of the whole table without a covering path.
        assert!(
            session_plan.to_lowercase().contains("index")
                || session_plan.to_lowercase().contains("using"),
            "session lane plan should use an index; plan was:\n{session_plan}"
        );
        assert!(
            legacy_plan.to_lowercase().contains("index")
                || legacy_plan.to_lowercase().contains("using"),
            "legacy lane plan should use an index; plan was:\n{legacy_plan}"
        );
        // Document plans in assertion messages for supervisor review evidence.
        assert!(
            !session_plan.is_empty() && !legacy_plan.is_empty(),
            "session_plan=\n{session_plan}\nlegacy_plan=\n{legacy_plan}"
        );
    }
}
