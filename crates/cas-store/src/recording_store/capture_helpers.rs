use cas_types::RecordingEventType;
use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, params};

pub fn get_active_recording_with_conn(
    conn: &Connection,
    session_id: &str,
) -> std::result::Result<Option<(String, DateTime<Utc>)>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, started_at FROM recordings
         WHERE session_id = ? AND ended_at IS NULL
         ORDER BY started_at DESC LIMIT 1",
        params![session_id],
        |row| {
            let id: String = row.get(0)?;
            let started_at_str: String = row.get(1)?;
            let started_at = DateTime::parse_from_rfc3339(&started_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok((id, started_at))
        },
    )
    .optional()
}

/// Get ANY active recording using an existing connection.
/// This is useful when we don't have a session_id but want to capture events
/// to the current active recording (e.g., in factory mode with a single recording).
/// Returns the recording ID and started_at timestamp if there's an active recording.
pub fn get_any_active_recording_with_conn(
    conn: &Connection,
) -> std::result::Result<Option<(String, DateTime<Utc>)>, rusqlite::Error> {
    conn.query_row(
        "SELECT id, started_at FROM recordings
         WHERE ended_at IS NULL
         ORDER BY started_at DESC LIMIT 1",
        [],
        |row| {
            let id: String = row.get(0)?;
            let started_at_str: String = row.get(1)?;
            let started_at = DateTime::parse_from_rfc3339(&started_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());
            Ok((id, started_at))
        },
    )
    .optional()
}

/// Record an event to the recording_events table using an existing connection.
/// This should be called alongside record_event_with_conn to capture events
/// for recording playback.
///
/// # Arguments
/// * `conn` - Database connection (typically from the store holding the lock)
/// * `session_id` - Session ID to find the active recording
/// * `event_type` - Type of recording event
/// * `entity_type` - Optional CAS entity type (task, entry, agent, etc.)
/// * `entity_id` - Optional CAS entity ID
/// * `metadata` - Optional JSON metadata
///
/// # Returns
/// Returns Ok(Some(event_id)) if an event was recorded, Ok(None) if no active recording,
/// or an error if the database operation failed.
pub fn record_recording_event_with_conn(
    conn: &Connection,
    session_id: &str,
    event_type: RecordingEventType,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    // Check for active recording
    let active = get_active_recording_with_conn(conn, session_id)?;

    if let Some((recording_id, started_at)) = active {
        // Calculate timestamp_ms from recording start
        let now = Utc::now();
        let timestamp_ms = (now - started_at).num_milliseconds();

        conn.execute(
            "INSERT INTO recording_events (recording_id, timestamp_ms, event_type,
             entity_type, entity_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                recording_id,
                timestamp_ms,
                event_type.to_string(),
                entity_type,
                entity_id,
                metadata,
            ],
        )?;
        Ok(Some(conn.last_insert_rowid()))
    } else {
        // No active recording for this session
        Ok(None)
    }
}

/// Helper to record a task event to the recording.
/// Wraps record_recording_event_with_conn with task-specific event types.
pub fn record_task_event_with_conn(
    conn: &Connection,
    session_id: &str,
    event_type: RecordingEventType,
    task_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    record_recording_event_with_conn(
        conn,
        session_id,
        event_type,
        Some("task"),
        Some(task_id),
        metadata,
    )
}

/// Record an event to ANY active recording (session-agnostic).
/// This is the primary function for capturing events in factory mode where
/// a single recording captures all agent activity.
pub fn capture_recording_event(
    conn: &Connection,
    event_type: RecordingEventType,
    entity_type: Option<&str>,
    entity_id: Option<&str>,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    // Check for any active recording
    let active = get_any_active_recording_with_conn(conn)?;

    if let Some((recording_id, started_at)) = active {
        // Calculate timestamp_ms from recording start
        let now = Utc::now();
        let timestamp_ms = (now - started_at).num_milliseconds();

        conn.execute(
            "INSERT INTO recording_events (recording_id, timestamp_ms, event_type,
             entity_type, entity_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                recording_id,
                timestamp_ms,
                event_type.to_string(),
                entity_type,
                entity_id,
                metadata,
            ],
        )?;
        Ok(Some(conn.last_insert_rowid()))
    } else {
        // No active recording
        Ok(None)
    }
}

/// Capture a task event to any active recording.
pub fn capture_task_event(
    conn: &Connection,
    event_type: RecordingEventType,
    task_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    capture_recording_event(conn, event_type, Some("task"), Some(task_id), metadata)
}

/// Capture an agent event to any active recording.
pub fn capture_agent_event(
    conn: &Connection,
    event_type: RecordingEventType,
    agent_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    capture_recording_event(conn, event_type, Some("agent"), Some(agent_id), metadata)
}

/// Capture a memory event to any active recording.
pub fn capture_memory_event(
    conn: &Connection,
    entry_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    capture_recording_event(
        conn,
        RecordingEventType::MemoryCreated,
        Some("entry"),
        Some(entry_id),
        metadata,
    )
}

/// Capture a message event to any active recording.
pub fn capture_message_event(
    conn: &Connection,
    from_agent: &str,
    to_agent: &str,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    let metadata = serde_json::json!({
        "from": from_agent,
        "to": to_agent
    })
    .to_string();

    capture_recording_event(
        conn,
        RecordingEventType::MessageSent,
        Some("message"),
        None,
        Some(&metadata),
    )
}

/// Helper to record an agent event to the recording.
pub fn record_agent_event_with_conn(
    conn: &Connection,
    session_id: &str,
    event_type: RecordingEventType,
    agent_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    record_recording_event_with_conn(
        conn,
        session_id,
        event_type,
        Some("agent"),
        Some(agent_id),
        metadata,
    )
}

/// Helper to record a memory event to the recording.
pub fn record_memory_event_with_conn(
    conn: &Connection,
    session_id: &str,
    entry_id: &str,
    metadata: Option<&str>,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    record_recording_event_with_conn(
        conn,
        session_id,
        RecordingEventType::MemoryCreated,
        Some("entry"),
        Some(entry_id),
        metadata,
    )
}

/// Helper to record a message event to the recording.
pub fn record_message_event_with_conn(
    conn: &Connection,
    session_id: &str,
    from_agent: &str,
    to_agent: &str,
) -> std::result::Result<Option<i64>, rusqlite::Error> {
    let metadata = serde_json::json!({
        "from": from_agent,
        "to": to_agent
    })
    .to_string();

    record_recording_event_with_conn(
        conn,
        session_id,
        RecordingEventType::MessageSent,
        Some("message"),
        None,
        Some(&metadata),
    )
}
