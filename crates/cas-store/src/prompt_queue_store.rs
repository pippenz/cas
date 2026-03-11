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
    /// Short summary for UI display
    pub summary: Option<String>,
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

    /// Queue a prompt with session and summary for UI display
    fn enqueue_with_summary(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
    ) -> Result<i64>;

    /// Poll for pending prompts for a specific target (marks as processed)
    fn poll_for_target(&self, target: &str, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Poll all pending prompts (for Factory TUI to process)
    fn poll_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Peek at pending prompts without marking as processed
    fn peek_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>>;

    /// Peek at pending prompts for specific targets only.
    ///
    /// Only returns messages whose target is in `targets`.
    /// When `factory_session` is provided, also returns session-tagged messages
    /// for that session (even if target doesn't match — belt-and-suspenders).
    /// This prevents one factory daemon from consuming messages meant for
    /// another daemon's workers/supervisor in multi-session setups.
    fn peek_for_targets(
        &self,
        targets: &[&str],
        factory_session: Option<&str>,
        limit: usize,
    ) -> Result<Vec<QueuedPrompt>>;

    /// Mark a prompt as processed
    fn mark_processed(&self, prompt_id: i64) -> Result<()>;

    /// Get count of pending prompts
    fn pending_count(&self) -> Result<usize>;

    /// Clear all prompts (for cleanup)
    fn clear(&self) -> Result<usize>;

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

        Ok(QueuedPrompt {
            id: row.get(0)?,
            source: row.get(1)?,
            target: row.get(2)?,
            prompt: row.get(3)?,
            created_at: Self::parse_datetime(&row.get::<_, String>(4)?).unwrap_or_else(Utc::now),
            processed_at,
            summary,
        })
    }
}

impl PromptQueueStore for SqlitePromptQueueStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(PROMPT_QUEUE_SCHEMA)?;

        // Add factory_session column if missing (safe migration for multi-session isolation)
        let has_session_col = conn
            .prepare("SELECT factory_session FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_session_col {
            conn.execute_batch(PROMPT_QUEUE_SESSION_MIGRATION)?;
        }

        // Add summary column if missing
        let has_summary_col = conn
            .prepare("SELECT summary FROM prompt_queue LIMIT 0")
            .is_ok();
        if !has_summary_col {
            conn.execute_batch(PROMPT_QUEUE_SUMMARY_MIGRATION)?;
        }

        Ok(())
    }

    fn enqueue(&self, source: &str, target: &str, prompt: &str) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO prompt_queue (source, target, prompt, created_at) VALUES (?, ?, ?, ?)",
            params![source, target, prompt, now],
        )?;

        let id = conn.last_insert_rowid();

        // Capture event for recording playback
        let _ = capture_message_event(&conn, source, target);

        Ok(id)
    }

    fn enqueue_with_session(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: &str,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO prompt_queue (source, target, prompt, created_at, factory_session) VALUES (?, ?, ?, ?, ?)",
            params![source, target, prompt, now, factory_session],
        )?;

        let id = conn.last_insert_rowid();
        let _ = capture_message_event(&conn, source, target);
        Ok(id)
    }

    fn enqueue_with_summary(
        &self,
        source: &str,
        target: &str,
        prompt: &str,
        factory_session: Option<&str>,
        summary: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO prompt_queue (source, target, prompt, created_at, factory_session, summary) VALUES (?, ?, ?, ?, ?, ?)",
            params![source, target, prompt, now, factory_session, summary],
        )?;

        let id = conn.last_insert_rowid();
        let _ = capture_message_event(&conn, source, target);
        Ok(id)
    }

    fn poll_for_target(&self, target: &str, limit: usize) -> Result<Vec<QueuedPrompt>> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // Get pending prompts for this target or "all_workers"
        let mut stmt = conn.prepare(
            "SELECT id, source, target, prompt, created_at, processed_at, summary
             FROM prompt_queue
             WHERE (target = ? OR target = 'all_workers') AND processed_at IS NULL
             ORDER BY created_at ASC
             LIMIT ?",
        )?;

        let prompts: Vec<QueuedPrompt> = stmt
            .query_map(params![target, limit as i64], Self::prompt_from_row)?
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
    }

    fn poll_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        let mut stmt = conn.prepare(
            "SELECT id, source, target, prompt, created_at, processed_at, summary
             FROM prompt_queue
             WHERE processed_at IS NULL
             ORDER BY created_at ASC
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
    }

    fn peek_all(&self, limit: usize) -> Result<Vec<QueuedPrompt>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT id, source, target, prompt, created_at, processed_at, summary
             FROM prompt_queue
             WHERE processed_at IS NULL
             ORDER BY created_at ASC
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
        if targets.is_empty() && factory_session.is_none() {
            return Ok(Vec::new());
        }

        let conn = self.conn.lock().unwrap();

        // Build WHERE clause: match by target OR by factory_session.
        // Target matching catches legacy messages (no session tag).
        // Session matching catches new messages even if target list is stale.
        let mut conditions = Vec::new();
        let mut param_values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if !targets.is_empty() {
            let placeholders: Vec<&str> = std::iter::repeat_n("?", targets.len()).collect();
            conditions.push(format!("target IN ({})", placeholders.join(", ")));
            for t in targets {
                param_values.push(Box::new(t.to_string()));
            }
        }

        if let Some(session) = factory_session {
            conditions.push("factory_session = ?".to_string());
            param_values.push(Box::new(session.to_string()));
        }

        let where_clause = conditions.join(" OR ");
        let sql = format!(
            "SELECT id, source, target, prompt, created_at, processed_at, summary
             FROM prompt_queue
             WHERE processed_at IS NULL
               AND ({where_clause})
             ORDER BY created_at ASC
             LIMIT ?"
        );

        param_values.push(Box::new(limit as i64));

        let mut stmt = conn.prepare(&sql)?;
        let prompts = stmt
            .query_map(
                rusqlite::params_from_iter(param_values.iter().map(|p| p.as_ref())),
                Self::prompt_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(prompts)
    }

    fn mark_processed(&self, prompt_id: i64) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "UPDATE prompt_queue SET processed_at = ? WHERE id = ?",
            params![now, prompt_id],
        )?;

        Ok(())
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
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM prompt_queue", [])?;
        Ok(rows)
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

        // Session B should only see its own messages
        let targets_b = &["supervisor-b", "worker-b1", "all_workers"];
        let prompts_b = store
            .peek_for_targets(targets_b, Some("session-b"), 10)
            .unwrap();
        assert_eq!(prompts_b.len(), 1);
        assert_eq!(prompts_b[0].target, "worker-b1");
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

        // peek_for_targets with wrong session but matching target still sees it
        let by_target = store
            .peek_for_targets(&["worker-1"], Some("other-session"), 10)
            .unwrap();
        assert_eq!(by_target.len(), 1); // target match

        // peek_for_targets with wrong target but matching session still sees it
        let by_session = store
            .peek_for_targets(&["nonexistent"], Some("my-session"), 10)
            .unwrap();
        assert_eq!(by_session.len(), 1); // session match
    }
}
