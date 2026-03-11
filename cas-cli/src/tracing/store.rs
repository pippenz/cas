use std::path::Path;

use chrono::{DateTime, Utc};

use crate::error::MemError;
use crate::tracing::{
    BufferedObservation, SurfacedItem, ToolTrace, TraceEvent, TraceEventType, TraceStats,
};

/// Trace store for persisting trace events
pub struct TraceStore {
    db: rusqlite::Connection,
}

impl TraceStore {
    /// Open or create trace store
    pub fn open(path: &Path) -> Result<Self, MemError> {
        let db = rusqlite::Connection::open(path)?;

        db.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS trace_events (
                id TEXT PRIMARY KEY,
                event_type TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                session_id TEXT,
                duration_ms INTEGER NOT NULL,
                input TEXT NOT NULL,
                output TEXT NOT NULL,
                metadata TEXT NOT NULL,
                success INTEGER NOT NULL,
                error TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_trace_type ON trace_events(event_type);
            CREATE INDEX IF NOT EXISTS idx_trace_timestamp ON trace_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_trace_session ON trace_events(session_id);

            -- Rich tool traces for learning loop detection
            CREATE TABLE IF NOT EXISTS tool_traces (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                sequence_pos INTEGER NOT NULL,
                file_path TEXT,
                is_dependency INTEGER NOT NULL DEFAULT 0,
                command TEXT,
                command_type TEXT,
                exit_code INTEGER,
                success INTEGER NOT NULL DEFAULT 1,
                error_snippet TEXT,
                error_type TEXT,
                output_snippet TEXT,
                lines_added INTEGER,
                lines_removed INTEGER,
                old_content TEXT,
                new_content TEXT,
                old_content_hash TEXT,
                new_content_hash TEXT,
                search_pattern TEXT,
                search_results_count INTEGER,
                url TEXT,
                prev_tool TEXT,
                prev_failed INTEGER NOT NULL DEFAULT 0,
                time_since_prev_ms INTEGER,
                attempt_id TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_tool_trace_session ON tool_traces(session_id);
            CREATE INDEX IF NOT EXISTS idx_tool_trace_timestamp ON tool_traces(timestamp);
            CREATE INDEX IF NOT EXISTS idx_tool_trace_tool ON tool_traces(tool_name);
            CREATE INDEX IF NOT EXISTS idx_tool_trace_success ON tool_traces(success);
            CREATE INDEX IF NOT EXISTS idx_tool_trace_attempt ON tool_traces(attempt_id);

            -- Surfaced items tracking for feedback nudging
            CREATE TABLE IF NOT EXISTS surfaced_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                item_type TEXT NOT NULL,
                item_preview TEXT,
                surfaced_at TEXT NOT NULL,
                feedback_given INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_surfaced_session ON surfaced_items(session_id);
            CREATE INDEX IF NOT EXISTS idx_surfaced_item ON surfaced_items(item_id);
            CREATE INDEX IF NOT EXISTS idx_surfaced_feedback ON surfaced_items(feedback_given);

            -- Observation buffer for session-level synthesis
            CREATE TABLE IF NOT EXISTS observation_buffer (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id TEXT NOT NULL,
                tool_name TEXT NOT NULL,
                file_path TEXT,
                content TEXT NOT NULL,
                exit_code INTEGER,
                is_error INTEGER NOT NULL DEFAULT 0,
                timestamp TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_obs_buffer_session ON observation_buffer(session_id);
            "#,
        )?;

        // Migration: Add new columns to existing tool_traces table
        // SQLite doesn't support IF NOT EXISTS for columns, so we try and ignore errors
        let migrations = [
            "ALTER TABLE tool_traces ADD COLUMN error_type TEXT",
            "ALTER TABLE tool_traces ADD COLUMN old_content TEXT",
            "ALTER TABLE tool_traces ADD COLUMN new_content TEXT",
            "ALTER TABLE tool_traces ADD COLUMN old_content_hash TEXT",
            "ALTER TABLE tool_traces ADD COLUMN new_content_hash TEXT",
            "ALTER TABLE tool_traces ADD COLUMN search_pattern TEXT",
            "ALTER TABLE tool_traces ADD COLUMN search_results_count INTEGER",
            "ALTER TABLE tool_traces ADD COLUMN url TEXT",
        ];
        for migration in migrations {
            let _ = db.execute(migration, []);
        }

        // Create index for error_type after migrations add the column
        let _ = db.execute(
            "CREATE INDEX IF NOT EXISTS idx_tool_trace_error_type ON tool_traces(error_type)",
            [],
        );

        Ok(Self { db })
    }

    /// Record a trace event
    pub fn record(&self, event: &TraceEvent) -> Result<(), MemError> {
        self.db.execute(
            r#"
            INSERT INTO trace_events
            (id, event_type, timestamp, session_id, duration_ms, input, output, metadata, success, error)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
            rusqlite::params![
                event.id,
                event.event_type.to_string(),
                event.timestamp.to_rfc3339(),
                event.session_id,
                event.duration_ms as i64,
                event.input,
                event.output,
                event.metadata,
                event.success as i32,
                event.error,
            ],
        )?;

        Ok(())
    }

    /// Get recent trace events
    pub fn get_recent(&self, limit: usize) -> Result<Vec<TraceEvent>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, event_type, timestamp, session_id, duration_ms,
                   input, output, metadata, success, error
            FROM trace_events
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map([limit as i64], |row| {
            let event_type_str: String = row.get(1)?;
            let event_type =
                TraceEventType::parse(&event_type_str).unwrap_or(TraceEventType::ContextInjection);

            Ok(TraceEvent {
                id: row.get(0)?,
                event_type,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                session_id: row.get(3)?,
                duration_ms: row.get::<_, i64>(4)? as u64,
                input: row.get(5)?,
                output: row.get(6)?,
                metadata: row.get(7)?,
                success: row.get::<_, i32>(8)? != 0,
                error: row.get(9)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Get trace events by type
    pub fn get_by_type(
        &self,
        event_type: TraceEventType,
        limit: usize,
    ) -> Result<Vec<TraceEvent>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, event_type, timestamp, session_id, duration_ms,
                   input, output, metadata, success, error
            FROM trace_events
            WHERE event_type = ?
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map(
            rusqlite::params![event_type.to_string(), limit as i64],
            |row| {
                let event_type_str: String = row.get(1)?;
                let event_type = TraceEventType::parse(&event_type_str)
                    .unwrap_or(TraceEventType::ContextInjection);

                Ok(TraceEvent {
                    id: row.get(0)?,
                    event_type,
                    timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                    session_id: row.get(3)?,
                    duration_ms: row.get::<_, i64>(4)? as u64,
                    input: row.get(5)?,
                    output: row.get(6)?,
                    metadata: row.get(7)?,
                    success: row.get::<_, i32>(8)? != 0,
                    error: row.get(9)?,
                })
            },
        )?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Get trace events by session
    pub fn get_by_session(&self, session_id: &str) -> Result<Vec<TraceEvent>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, event_type, timestamp, session_id, duration_ms,
                   input, output, metadata, success, error
            FROM trace_events
            WHERE session_id = ?
            ORDER BY timestamp ASC
            "#,
        )?;

        let rows = stmt.query_map([session_id], |row| {
            let event_type_str: String = row.get(1)?;
            let event_type =
                TraceEventType::parse(&event_type_str).unwrap_or(TraceEventType::ContextInjection);

            Ok(TraceEvent {
                id: row.get(0)?,
                event_type,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                session_id: row.get(3)?,
                duration_ms: row.get::<_, i64>(4)? as u64,
                input: row.get(5)?,
                output: row.get(6)?,
                metadata: row.get(7)?,
                success: row.get::<_, i32>(8)? != 0,
                error: row.get(9)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Get aggregated stats by event type
    pub fn get_stats(&self) -> Result<Vec<TraceStats>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT event_type,
                   COUNT(*) as count,
                   AVG(duration_ms) as avg_duration,
                   SUM(CASE WHEN success = 1 THEN 1 ELSE 0 END) as success_count,
                   SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END) as failure_count
            FROM trace_events
            GROUP BY event_type
            ORDER BY count DESC
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            let event_type_str: String = row.get(0)?;
            let event_type =
                TraceEventType::parse(&event_type_str).unwrap_or(TraceEventType::ContextInjection);

            Ok(TraceStats {
                event_type,
                count: row.get::<_, i64>(1)? as u64,
                avg_duration_ms: row.get(2)?,
                success_count: row.get::<_, i64>(3)? as u64,
                failure_count: row.get::<_, i64>(4)? as u64,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Clear old trace events (older than days)
    pub fn clear_old(&self, days: i64) -> Result<usize, MemError> {
        let cutoff = Utc::now() - chrono::Duration::days(days);
        let count = self.db.execute(
            "DELETE FROM trace_events WHERE timestamp < ?",
            [cutoff.to_rfc3339()],
        )?;
        Ok(count)
    }

    /// Get a single trace event by ID
    pub fn get(&self, id: &str) -> Result<Option<TraceEvent>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, event_type, timestamp, session_id, duration_ms,
                   input, output, metadata, success, error
            FROM trace_events
            WHERE id = ?
            "#,
        )?;

        let mut rows = stmt.query([id])?;

        if let Some(row) = rows.next()? {
            let event_type_str: String = row.get(1)?;
            let event_type =
                TraceEventType::parse(&event_type_str).unwrap_or(TraceEventType::ContextInjection);

            Ok(Some(TraceEvent {
                id: row.get(0)?,
                event_type,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                session_id: row.get(3)?,
                duration_ms: row.get::<_, i64>(4)? as u64,
                input: row.get(5)?,
                output: row.get(6)?,
                metadata: row.get(7)?,
                success: row.get::<_, i32>(8)? != 0,
                error: row.get(9)?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Search trace events by content in input, output, or metadata
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<TraceEvent>, MemError> {
        let pattern = format!("%{query}%");
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, event_type, timestamp, session_id, duration_ms,
                   input, output, metadata, success, error
            FROM trace_events
            WHERE input LIKE ?1 OR output LIKE ?1 OR metadata LIKE ?1 OR error LIKE ?1
            ORDER BY timestamp DESC
            LIMIT ?2
            "#,
        )?;

        let rows = stmt.query_map(rusqlite::params![pattern, limit as i64], |row| {
            let event_type_str: String = row.get(1)?;
            let event_type =
                TraceEventType::parse(&event_type_str).unwrap_or(TraceEventType::ContextInjection);

            Ok(TraceEvent {
                id: row.get(0)?,
                event_type,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                session_id: row.get(3)?,
                duration_ms: row.get::<_, i64>(4)? as u64,
                input: row.get(5)?,
                output: row.get(6)?,
                metadata: row.get(7)?,
                success: row.get::<_, i32>(8)? != 0,
                error: row.get(9)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Get trace count
    pub fn count(&self) -> Result<usize, MemError> {
        let count: i64 = self
            .db
            .query_row("SELECT COUNT(*) FROM trace_events", [], |row| row.get(0))?;
        Ok(count as usize)
    }

    /// Record a rich tool trace
    pub fn record_tool_trace(&self, trace: &ToolTrace) -> Result<(), MemError> {
        self.db.execute(
            r#"
            INSERT INTO tool_traces
            (id, session_id, timestamp, tool_name, sequence_pos, file_path, is_dependency,
             command, command_type, exit_code, success, error_snippet, error_type, output_snippet,
             lines_added, lines_removed, old_content, new_content, old_content_hash, new_content_hash,
             search_pattern, search_results_count, url, prev_tool, prev_failed, time_since_prev_ms, attempt_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27)
            "#,
            rusqlite::params![
                trace.id,
                trace.session_id,
                trace.timestamp.to_rfc3339(),
                trace.tool_name,
                trace.sequence_pos,
                trace.file_path,
                trace.is_dependency as i32,
                trace.command,
                trace.command_type,
                trace.exit_code,
                trace.success as i32,
                trace.error_snippet,
                trace.error_type,
                trace.output_snippet,
                trace.lines_added,
                trace.lines_removed,
                trace.old_content,
                trace.new_content,
                trace.old_content_hash,
                trace.new_content_hash,
                trace.search_pattern,
                trace.search_results_count,
                trace.url,
                trace.prev_tool,
                trace.prev_failed as i32,
                trace.time_since_prev_ms.map(|v| v as i64),
                trace.attempt_id,
            ],
        )?;
        Ok(())
    }

    /// Get tool traces for a session
    pub fn get_tool_traces(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<ToolTrace>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, session_id, timestamp, tool_name, sequence_pos, file_path, is_dependency,
                   command, command_type, exit_code, success, error_snippet, error_type, output_snippet,
                   lines_added, lines_removed, old_content, new_content, old_content_hash, new_content_hash,
                   search_pattern, search_results_count, url, prev_tool, prev_failed, time_since_prev_ms, attempt_id
            FROM tool_traces
            WHERE session_id = ?
            ORDER BY sequence_pos ASC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map(rusqlite::params![session_id, limit as i64], |row| {
            Ok(ToolTrace {
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                tool_name: row.get(3)?,
                sequence_pos: row.get(4)?,
                file_path: row.get(5)?,
                is_dependency: row.get::<_, i32>(6)? != 0,
                command: row.get(7)?,
                command_type: row.get(8)?,
                exit_code: row.get(9)?,
                success: row.get::<_, i32>(10)? != 0,
                error_snippet: row.get(11)?,
                error_type: row.get(12)?,
                output_snippet: row.get(13)?,
                lines_added: row.get(14)?,
                lines_removed: row.get(15)?,
                old_content: row.get(16)?,
                new_content: row.get(17)?,
                old_content_hash: row.get(18)?,
                new_content_hash: row.get(19)?,
                search_pattern: row.get(20)?,
                search_results_count: row.get(21)?,
                url: row.get(22)?,
                prev_tool: row.get(23)?,
                prev_failed: row.get::<_, i32>(24)? != 0,
                time_since_prev_ms: row.get::<_, Option<i64>>(25)?.map(|v| v as u64),
                attempt_id: row.get(26)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Get last tool trace for session (for sequence tracking)
    pub fn get_last_tool_trace(&self, session_id: &str) -> Result<Option<ToolTrace>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, session_id, timestamp, tool_name, sequence_pos, file_path, is_dependency,
                   command, command_type, exit_code, success, error_snippet, error_type, output_snippet,
                   lines_added, lines_removed, old_content, new_content, old_content_hash, new_content_hash,
                   search_pattern, search_results_count, url, prev_tool, prev_failed, time_since_prev_ms, attempt_id
            FROM tool_traces
            WHERE session_id = ?
            ORDER BY sequence_pos DESC
            LIMIT 1
            "#,
        )?;

        let mut rows = stmt.query_map(rusqlite::params![session_id], |row| {
            Ok(ToolTrace {
                id: row.get(0)?,
                session_id: row.get(1)?,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                tool_name: row.get(3)?,
                sequence_pos: row.get(4)?,
                file_path: row.get(5)?,
                is_dependency: row.get::<_, i32>(6)? != 0,
                command: row.get(7)?,
                command_type: row.get(8)?,
                exit_code: row.get(9)?,
                success: row.get::<_, i32>(10)? != 0,
                error_snippet: row.get(11)?,
                error_type: row.get(12)?,
                output_snippet: row.get(13)?,
                lines_added: row.get(14)?,
                lines_removed: row.get(15)?,
                old_content: row.get(16)?,
                new_content: row.get(17)?,
                old_content_hash: row.get(18)?,
                new_content_hash: row.get(19)?,
                search_pattern: row.get(20)?,
                search_results_count: row.get(21)?,
                url: row.get(22)?,
                prev_tool: row.get(23)?,
                prev_failed: row.get::<_, i32>(24)? != 0,
                time_since_prev_ms: row.get::<_, Option<i64>>(25)?.map(|v| v as u64),
                attempt_id: row.get(26)?,
            })
        })?;

        match rows.next() {
            Some(Ok(trace)) => Ok(Some(trace)),
            Some(Err(e)) => Err(MemError::Database(e)),
            None => Ok(None),
        }
    }

    /// Record a surfaced item for feedback tracking
    pub fn record_surfaced_item(&self, item: &SurfacedItem) -> Result<(), MemError> {
        self.db.execute(
            r#"
            INSERT INTO surfaced_items (session_id, item_id, item_type, item_preview, surfaced_at, feedback_given)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            "#,
            rusqlite::params![
                item.session_id,
                item.item_id,
                item.item_type,
                item.item_preview,
                item.surfaced_at.to_rfc3339(),
                item.feedback_given as i32,
            ],
        )?;
        Ok(())
    }

    /// Get surfaced items from recent sessions that haven't received feedback
    pub fn get_unfeedback_surfaced_items(
        &self,
        limit: usize,
    ) -> Result<Vec<SurfacedItem>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT session_id, item_id, item_type, item_preview, surfaced_at, feedback_given
            FROM surfaced_items
            WHERE feedback_given = 0
            ORDER BY surfaced_at DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map([limit as i64], |row| {
            Ok(SurfacedItem {
                session_id: row.get(0)?,
                item_id: row.get(1)?,
                item_type: row.get(2)?,
                item_preview: row.get(3)?,
                surfaced_at: DateTime::parse_from_rfc3339(&row.get::<_, String>(4)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                feedback_given: row.get::<_, i32>(5)? != 0,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Mark a surfaced item as having received feedback
    pub fn mark_surfaced_feedback(&self, item_id: &str) -> Result<(), MemError> {
        self.db.execute(
            "UPDATE surfaced_items SET feedback_given = 1 WHERE item_id = ?",
            [item_id],
        )?;
        Ok(())
    }

    /// Check if an item was surfaced in the current session (for implicit feedback)
    pub fn was_surfaced_in_session(
        &self,
        session_id: &str,
        item_id: &str,
    ) -> Result<bool, MemError> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM surfaced_items WHERE session_id = ? AND item_id = ? AND feedback_given = 0",
            rusqlite::params![session_id, item_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Add an observation to the buffer for later synthesis
    pub fn buffer_observation(&self, obs: &BufferedObservation) -> Result<(), MemError> {
        self.db.execute(
            r#"
            INSERT INTO observation_buffer (session_id, tool_name, file_path, content, exit_code, is_error, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            "#,
            rusqlite::params![
                obs.session_id,
                obs.tool_name,
                obs.file_path,
                obs.content,
                obs.exit_code,
                obs.is_error as i32,
                obs.timestamp.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get buffered observations for a session
    pub fn get_buffered_observations(
        &self,
        session_id: &str,
    ) -> Result<Vec<BufferedObservation>, MemError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT session_id, tool_name, file_path, content, exit_code, is_error, timestamp
            FROM observation_buffer
            WHERE session_id = ?
            ORDER BY timestamp ASC
            "#,
        )?;

        let rows = stmt.query_map([session_id], |row| {
            Ok(BufferedObservation {
                session_id: row.get(0)?,
                tool_name: row.get(1)?,
                file_path: row.get(2)?,
                content: row.get(3)?,
                exit_code: row.get(4)?,
                is_error: row.get::<_, i32>(5)? != 0,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(MemError::Database)
    }

    /// Clear buffered observations for a session (after synthesis)
    pub fn clear_observation_buffer(&self, session_id: &str) -> Result<usize, MemError> {
        let count = self.db.execute(
            "DELETE FROM observation_buffer WHERE session_id = ?",
            [session_id],
        )?;
        Ok(count)
    }

    /// Get observation buffer count for a session
    pub fn observation_buffer_count(&self, session_id: &str) -> Result<usize, MemError> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM observation_buffer WHERE session_id = ?",
            [session_id],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }
}
