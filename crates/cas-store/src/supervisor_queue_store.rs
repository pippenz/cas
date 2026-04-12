//! Supervisor queue storage for factory session Director → Supervisor communication
//!
//! The supervisor queue allows the Director (TUI) to batch notifications and
//! send them to the Supervisor agent. This enables asynchronous, prioritized
//! event delivery in multi-agent factory sessions.

use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crate::Result;
use crate::error::StoreError;

/// Priority levels for supervisor queue notifications
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u8)]
pub enum NotificationPriority {
    /// Critical - agent died, blocking error
    Critical = 0,
    /// High - task completed, needs review
    High = 1,
    /// Normal - status update, informational
    Normal = 2,
}

impl From<u8> for NotificationPriority {
    fn from(value: u8) -> Self {
        match value {
            0 => NotificationPriority::Critical,
            1 => NotificationPriority::High,
            _ => NotificationPriority::Normal,
        }
    }
}

impl From<NotificationPriority> for i32 {
    fn from(value: NotificationPriority) -> Self {
        value as i32
    }
}

/// A notification in the supervisor queue
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SupervisorNotification {
    /// Unique notification ID
    pub id: i64,
    /// Target supervisor agent ID
    pub supervisor_id: String,
    /// Type of event (e.g., "task_completed", "worker_died", "task_blocked")
    pub event_type: String,
    /// JSON payload with event details
    pub payload: String,
    /// Notification priority
    pub priority: NotificationPriority,
    /// When the notification was created
    pub created_at: DateTime<Utc>,
    /// When the notification was processed (None if pending)
    pub processed_at: Option<DateTime<Utc>>,
}

/// Schema for supervisor queue table
const SUPERVISOR_QUEUE_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS supervisor_queue (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    supervisor_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 2,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    processed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_supervisor_queue_supervisor ON supervisor_queue(supervisor_id);
CREATE INDEX IF NOT EXISTS idx_supervisor_queue_pending ON supervisor_queue(supervisor_id, priority) WHERE processed_at IS NULL;
"#;

/// Trait for supervisor queue operations
pub trait SupervisorQueueStore: Send + Sync {
    /// Initialize the store (create tables)
    fn init(&self) -> Result<()>;

    /// Queue a notification for a supervisor
    fn notify(
        &self,
        supervisor_id: &str,
        event_type: &str,
        payload: &str,
        priority: NotificationPriority,
    ) -> Result<i64>;

    /// Poll for pending notifications (ordered by priority, then created_at)
    /// Returns up to `limit` notifications and marks them as processed
    fn poll(&self, supervisor_id: &str, limit: usize) -> Result<Vec<SupervisorNotification>>;

    /// Peek at pending notifications without marking them as processed
    fn peek(&self, supervisor_id: &str, limit: usize) -> Result<Vec<SupervisorNotification>>;

    /// Acknowledge a specific notification (mark as processed)
    fn ack(&self, notification_id: i64) -> Result<()>;

    /// Get count of pending notifications for a supervisor
    fn pending_count(&self, supervisor_id: &str) -> Result<usize>;

    /// Get all pending notifications for a supervisor (without processing)
    fn list_pending(&self, supervisor_id: &str) -> Result<Vec<SupervisorNotification>>;

    /// Clear all notifications for a supervisor (processed and pending)
    fn clear(&self, supervisor_id: &str) -> Result<usize>;

    /// Clear old processed notifications (cleanup)
    fn cleanup_old(&self, older_than_secs: i64) -> Result<usize>;

    /// Close the store
    fn close(&self) -> Result<()>;
}

/// SQLite-based supervisor queue store
pub struct SqliteSupervisorQueueStore {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteSupervisorQueueStore {
    /// Open or create a SQLite supervisor queue store
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

    fn notification_from_row(row: &rusqlite::Row) -> rusqlite::Result<SupervisorNotification> {
        let processed_at_str: Option<String> = row.get(6)?;
        let processed_at = processed_at_str.and_then(|s| Self::parse_datetime(&s));

        Ok(SupervisorNotification {
            id: row.get(0)?,
            supervisor_id: row.get(1)?,
            event_type: row.get(2)?,
            payload: row.get(3)?,
            priority: NotificationPriority::from(row.get::<_, u8>(4)?),
            created_at: Self::parse_datetime(&row.get::<_, String>(5)?).unwrap_or_else(Utc::now),
            processed_at,
        })
    }
}

impl SupervisorQueueStore for SqliteSupervisorQueueStore {
    fn init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SUPERVISOR_QUEUE_SCHEMA)?;
        Ok(())
    }

    fn notify(
        &self,
        supervisor_id: &str,
        event_type: &str,
        payload: &str,
        priority: NotificationPriority,
    ) -> Result<i64> {
        crate::shared_db::with_write_retry(|| {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        conn.execute(
            "INSERT INTO supervisor_queue (supervisor_id, event_type, payload, priority, created_at)
             VALUES (?, ?, ?, ?, ?)",
            params![supervisor_id, event_type, payload, i32::from(priority), now],
        )?;

        let id = conn.last_insert_rowid();
        Ok(id)
        }) // with_write_retry
    }

    fn poll(&self, supervisor_id: &str, limit: usize) -> Result<Vec<SupervisorNotification>> {
        crate::shared_db::with_write_retry(|| {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // Get pending notifications ordered by priority (ascending = critical first), then created_at
        let mut stmt = conn.prepare_cached(
            "SELECT id, supervisor_id, event_type, payload, priority, created_at, processed_at
             FROM supervisor_queue
             WHERE supervisor_id = ? AND processed_at IS NULL
             ORDER BY priority ASC, created_at ASC
             LIMIT ?",
        )?;

        let notifications: Vec<SupervisorNotification> = stmt
            .query_map(
                params![supervisor_id, limit as i64],
                Self::notification_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        // Mark them as processed
        if !notifications.is_empty() {
            let ids: Vec<i64> = notifications.iter().map(|n| n.id).collect();
            let placeholders: Vec<String> = ids.iter().map(|_| "?".to_string()).collect();
            let sql = format!(
                "UPDATE supervisor_queue SET processed_at = ? WHERE id IN ({})",
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

        Ok(notifications)
        }) // with_write_retry
    }

    fn peek(&self, supervisor_id: &str, limit: usize) -> Result<Vec<SupervisorNotification>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare_cached(
            "SELECT id, supervisor_id, event_type, payload, priority, created_at, processed_at
             FROM supervisor_queue
             WHERE supervisor_id = ? AND processed_at IS NULL
             ORDER BY priority ASC, created_at ASC
             LIMIT ?",
        )?;

        let notifications = stmt
            .query_map(
                params![supervisor_id, limit as i64],
                Self::notification_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(notifications)
    }

    fn ack(&self, notification_id: i64) -> Result<()> {
        crate::shared_db::with_write_retry(|| {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        let rows = conn.execute(
            "UPDATE supervisor_queue SET processed_at = ? WHERE id = ? AND processed_at IS NULL",
            params![now, notification_id],
        )?;

        if rows == 0 {
            return Err(StoreError::NotFound(format!(
                "Notification not found or already processed: {notification_id}"
            )));
        }

        Ok(())
        }) // with_write_retry
    }

    fn pending_count(&self, supervisor_id: &str) -> Result<usize> {
        let conn = self.conn.lock().unwrap();

        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM supervisor_queue WHERE supervisor_id = ? AND processed_at IS NULL",
            params![supervisor_id],
            |row| row.get(0),
        )?;

        Ok(count as usize)
    }

    fn list_pending(&self, supervisor_id: &str) -> Result<Vec<SupervisorNotification>> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare_cached(
            "SELECT id, supervisor_id, event_type, payload, priority, created_at, processed_at
             FROM supervisor_queue
             WHERE supervisor_id = ? AND processed_at IS NULL
             ORDER BY priority ASC, created_at ASC",
        )?;

        let notifications = stmt
            .query_map(params![supervisor_id], Self::notification_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(notifications)
    }

    fn clear(&self, supervisor_id: &str) -> Result<usize> {
        crate::shared_db::with_write_retry(|| {
        let conn = self.conn.lock().unwrap();

        let rows = conn.execute(
            "DELETE FROM supervisor_queue WHERE supervisor_id = ?",
            params![supervisor_id],
        )?;

        Ok(rows)
        }) // with_write_retry
    }

    fn cleanup_old(&self, older_than_secs: i64) -> Result<usize> {
        crate::shared_db::with_write_retry(|| {
        let conn = self.conn.lock().unwrap();
        let cutoff = (Utc::now() - chrono::Duration::seconds(older_than_secs)).to_rfc3339();

        let rows = conn.execute(
            "DELETE FROM supervisor_queue WHERE processed_at IS NOT NULL AND processed_at < ?",
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
    use crate::supervisor_queue_store::*;
    use tempfile::TempDir;

    fn create_test_store() -> (TempDir, SqliteSupervisorQueueStore) {
        let temp = TempDir::new().unwrap();
        let store = SqliteSupervisorQueueStore::open(temp.path()).unwrap();
        store.init().unwrap();
        (temp, store)
    }

    #[test]
    fn test_notify_and_poll() {
        let (_temp, store) = create_test_store();

        // Queue some notifications
        let id1 = store
            .notify(
                "supervisor-1",
                "task_completed",
                r#"{"task_id": "task-1"}"#,
                NotificationPriority::Normal,
            )
            .unwrap();
        let id2 = store
            .notify(
                "supervisor-1",
                "worker_died",
                r#"{"worker_id": "worker-1"}"#,
                NotificationPriority::Critical,
            )
            .unwrap();

        assert!(id1 > 0);
        assert!(id2 > id1);

        // Poll should return critical first
        let notifications = store.poll("supervisor-1", 10).unwrap();
        assert_eq!(notifications.len(), 2);
        assert_eq!(notifications[0].event_type, "worker_died"); // Critical
        assert_eq!(notifications[1].event_type, "task_completed"); // Normal

        // Polling again should return empty (already processed)
        let notifications = store.poll("supervisor-1", 10).unwrap();
        assert!(notifications.is_empty());
    }

    #[test]
    fn test_peek_does_not_process() {
        let (_temp, store) = create_test_store();

        store
            .notify(
                "supervisor-1",
                "task_completed",
                "{}",
                NotificationPriority::Normal,
            )
            .unwrap();

        // Peek should return notification
        let notifications = store.peek("supervisor-1", 10).unwrap();
        assert_eq!(notifications.len(), 1);

        // Peek again should still return it (not processed)
        let notifications = store.peek("supervisor-1", 10).unwrap();
        assert_eq!(notifications.len(), 1);

        // Pending count should be 1
        let count = store.pending_count("supervisor-1").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_ack() {
        let (_temp, store) = create_test_store();

        let id = store
            .notify(
                "supervisor-1",
                "task_completed",
                "{}",
                NotificationPriority::Normal,
            )
            .unwrap();

        // Ack the notification
        store.ack(id).unwrap();

        // Should now be processed
        let count = store.pending_count("supervisor-1").unwrap();
        assert_eq!(count, 0);

        // Ack again should fail
        assert!(store.ack(id).is_err());
    }

    #[test]
    fn test_clear() {
        let (_temp, store) = create_test_store();

        store
            .notify("supervisor-1", "event1", "{}", NotificationPriority::Normal)
            .unwrap();
        store
            .notify("supervisor-1", "event2", "{}", NotificationPriority::Normal)
            .unwrap();
        store
            .notify("supervisor-2", "event3", "{}", NotificationPriority::Normal)
            .unwrap();

        // Clear supervisor-1's notifications
        let cleared = store.clear("supervisor-1").unwrap();
        assert_eq!(cleared, 2);

        // supervisor-1 should have no pending
        let count = store.pending_count("supervisor-1").unwrap();
        assert_eq!(count, 0);

        // supervisor-2 should still have 1
        let count = store.pending_count("supervisor-2").unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_priority_ordering() {
        let (_temp, store) = create_test_store();

        // Queue in reverse priority order
        store
            .notify("supervisor-1", "normal", "{}", NotificationPriority::Normal)
            .unwrap();
        store
            .notify("supervisor-1", "high", "{}", NotificationPriority::High)
            .unwrap();
        store
            .notify(
                "supervisor-1",
                "critical",
                "{}",
                NotificationPriority::Critical,
            )
            .unwrap();

        // Poll should return in priority order
        let notifications = store.poll("supervisor-1", 10).unwrap();
        assert_eq!(notifications.len(), 3);
        assert_eq!(notifications[0].event_type, "critical");
        assert_eq!(notifications[1].event_type, "high");
        assert_eq!(notifications[2].event_type, "normal");
    }

    #[test]
    fn test_different_supervisors() {
        let (_temp, store) = create_test_store();

        store
            .notify("supervisor-1", "event1", "{}", NotificationPriority::Normal)
            .unwrap();
        store
            .notify("supervisor-2", "event2", "{}", NotificationPriority::Normal)
            .unwrap();

        // Each supervisor should only see their own notifications
        let notifications = store.poll("supervisor-1", 10).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].event_type, "event1");

        let notifications = store.poll("supervisor-2", 10).unwrap();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].event_type, "event2");
    }
}
