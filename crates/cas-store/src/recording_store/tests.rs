use crate::RecordingStore;
use crate::recording_store::{
    SqliteRecordingStore, get_active_recording_with_conn, record_agent_event_with_conn,
    record_message_event_with_conn, record_task_event_with_conn,
};
use cas_types::{Recording, RecordingAgent, RecordingEvent, RecordingEventType, RecordingQuery};
use chrono::Utc;
use rusqlite::{Connection, params};
use tempfile::TempDir;

fn create_test_store() -> (TempDir, SqliteRecordingStore) {
    let temp = TempDir::new().unwrap();
    let conn = Connection::open(temp.path().join("cas.db")).unwrap();

    // Create recording tables
    conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS recordings (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                duration_ms INTEGER,
                file_path TEXT NOT NULL,
                file_size INTEGER,
                title TEXT,
                description TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE TABLE IF NOT EXISTS recording_agents (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recording_id TEXT NOT NULL,
                agent_name TEXT NOT NULL,
                agent_type TEXT NOT NULL,
                file_path TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                FOREIGN KEY (recording_id) REFERENCES recordings(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS recording_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                recording_id TEXT NOT NULL,
                timestamp_ms INTEGER NOT NULL,
                event_type TEXT NOT NULL,
                entity_type TEXT,
                entity_id TEXT,
                metadata TEXT,
                FOREIGN KEY (recording_id) REFERENCES recordings(id) ON DELETE CASCADE
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS recordings_fts USING fts5(
                recording_id UNINDEXED,
                content,
                content_type UNINDEXED,
                timestamp_ms UNINDEXED
            );

            CREATE INDEX IF NOT EXISTS idx_recordings_session ON recordings(session_id);
            CREATE INDEX IF NOT EXISTS idx_recordings_started ON recordings(started_at DESC);
            CREATE INDEX IF NOT EXISTS idx_recording_agents_name ON recording_agents(agent_name);
            CREATE INDEX IF NOT EXISTS idx_recording_agents_recording ON recording_agents(recording_id);
            CREATE INDEX IF NOT EXISTS idx_recording_events_recording ON recording_events(recording_id);
            CREATE INDEX IF NOT EXISTS idx_recording_events_timestamp ON recording_events(timestamp_ms);
            "#,
        )
        .unwrap();

    drop(conn);

    let store = SqliteRecordingStore::open(temp.path()).unwrap();
    (temp, store)
}

#[test]
fn test_recording_crud() {
    let (_temp, store) = create_test_store();

    // Create
    let mut recording = Recording::new("/path/to/rec.bin".to_string());
    recording.session_id = Some("session-1".to_string());
    recording.title = Some("Test Recording".to_string());

    store.add(&recording).unwrap();

    // Read
    let fetched = store.get(&recording.id).unwrap();
    assert_eq!(fetched.session_id, Some("session-1".to_string()));
    assert_eq!(fetched.title, Some("Test Recording".to_string()));
    assert_eq!(fetched.file_path, "/path/to/rec.bin");

    // Update
    let mut updated = fetched;
    updated.end();
    updated.file_size = Some(1024);
    store.update(&updated).unwrap();

    let refetched = store.get(&recording.id).unwrap();
    assert!(refetched.ended_at.is_some());
    assert!(refetched.duration_ms.is_some());
    assert_eq!(refetched.file_size, Some(1024));

    // Delete
    store.delete(&recording.id).unwrap();
    assert!(store.get(&recording.id).is_err());
}

#[test]
fn test_recording_agents() {
    let (_temp, store) = create_test_store();

    let recording = Recording::new("/path/to/rec.bin".to_string());
    store.add(&recording).unwrap();

    // Add agents
    let agent1 = RecordingAgent::new(
        recording.id.clone(),
        "swift-fox".to_string(),
        "worker".to_string(),
        "/path/to/swift-fox.bin".to_string(),
    );
    let agent2 = RecordingAgent::new(
        recording.id.clone(),
        "proud-finch".to_string(),
        "supervisor".to_string(),
        "/path/to/proud-finch.bin".to_string(),
    );

    store.add_agent(&agent1).unwrap();
    store.add_agent(&agent2).unwrap();

    // Get agents
    let agents = store.get_agents(&recording.id).unwrap();
    assert_eq!(agents.len(), 2);

    // Delete agents
    store.delete_agents(&recording.id).unwrap();
    let agents = store.get_agents(&recording.id).unwrap();
    assert_eq!(agents.len(), 0);
}

#[test]
fn test_recording_events() {
    let (_temp, store) = create_test_store();

    let recording = Recording::new("/path/to/rec.bin".to_string());
    store.add(&recording).unwrap();

    // Add events
    let event1 = RecordingEvent::for_entity(
        recording.id.clone(),
        1000,
        RecordingEventType::TaskCreated,
        "task".to_string(),
        "cas-1234".to_string(),
    );
    let event2 = RecordingEvent::for_entity(
        recording.id.clone(),
        2000,
        RecordingEventType::TaskCompleted,
        "task".to_string(),
        "cas-1234".to_string(),
    );

    store.add_event(&event1).unwrap();
    store.add_event(&event2).unwrap();

    // Get events
    let events = store.get_events(&recording.id).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].timestamp_ms, 1000);
    assert_eq!(events[1].timestamp_ms, 2000);

    // Get events in range
    let events = store.get_events_in_range(&recording.id, 500, 1500).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].timestamp_ms, 1000);

    // Get events for entity
    let events = store
        .get_events_for_entity(&recording.id, "task", "cas-1234")
        .unwrap();
    assert_eq!(events.len(), 2);
}

#[test]
fn test_query_recordings() {
    let (_temp, store) = create_test_store();

    // Create recordings
    let rec1 = Recording::for_session("session-1".to_string(), "/path/1.bin".to_string());
    let rec2 = Recording::for_session("session-2".to_string(), "/path/2.bin".to_string());
    let rec3 = Recording::for_session("session-1".to_string(), "/path/3.bin".to_string());

    store.add(&rec1).unwrap();
    store.add(&rec2).unwrap();
    store.add(&rec3).unwrap();

    // Add agent to rec1
    let agent = RecordingAgent::new(
        rec1.id.clone(),
        "swift-fox".to_string(),
        "worker".to_string(),
        "/path/agent.bin".to_string(),
    );
    store.add_agent(&agent).unwrap();

    // Query by session
    let query = RecordingQuery::new().for_session("session-1".to_string());
    let results = store.query(&query).unwrap();
    assert_eq!(results.len(), 2);

    // Query by agent
    let query = RecordingQuery::new().by_agent("swift-fox".to_string());
    let results = store.query(&query).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, rec1.id);

    // Query with limit
    let query = RecordingQuery::new().with_limit(2);
    let results = store.query(&query).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_list_by_session() {
    let (_temp, store) = create_test_store();

    let rec1 = Recording::for_session("session-1".to_string(), "/path/1.bin".to_string());
    let rec2 = Recording::for_session("session-2".to_string(), "/path/2.bin".to_string());
    let rec3 = Recording::for_session("session-1".to_string(), "/path/3.bin".to_string());

    store.add(&rec1).unwrap();
    store.add(&rec2).unwrap();
    store.add(&rec3).unwrap();

    let results = store.list_by_session("session-1").unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_list_by_agent() {
    let (_temp, store) = create_test_store();

    let rec1 = Recording::new("/path/1.bin".to_string());
    let rec2 = Recording::new("/path/2.bin".to_string());

    store.add(&rec1).unwrap();
    store.add(&rec2).unwrap();

    // Add agents
    store
        .add_agent(&RecordingAgent::new(
            rec1.id.clone(),
            "swift-fox".to_string(),
            "worker".to_string(),
            "/agent1.bin".to_string(),
        ))
        .unwrap();
    store
        .add_agent(&RecordingAgent::new(
            rec2.id.clone(),
            "proud-finch".to_string(),
            "supervisor".to_string(),
            "/agent2.bin".to_string(),
        ))
        .unwrap();

    let results = store.list_by_agent("swift-fox").unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, rec1.id);
}

#[test]
fn test_fts_search() {
    let (_temp, store) = create_test_store();

    let recording = Recording::new("/path/to/rec.bin".to_string());
    store.add(&recording).unwrap();

    // Add FTS content
    store
        .add_fts_content(&recording.id, "hello world test", "output", 1000)
        .unwrap();
    store
        .add_fts_content(&recording.id, "goodbye world", "output", 2000)
        .unwrap();

    // Search
    let results = store.search_fts("hello", 10).unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].0, recording.id);
    assert_eq!(results[0].1, 1000);

    let results = store.search_fts("world", 10).unwrap();
    assert_eq!(results.len(), 2);
}

#[test]
fn test_get_active_recording() {
    let (temp, _store) = create_test_store();
    let conn = Connection::open(temp.path().join("cas.db")).unwrap();

    // Create a recording with session_id
    let recording = Recording::for_session("session-1".to_string(), "/path/rec.bin".to_string());
    conn.execute(
        "INSERT INTO recordings (id, session_id, started_at, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            recording.id,
            recording.session_id,
            recording.started_at.to_rfc3339(),
            recording.file_path,
            recording.created_at.to_rfc3339(),
        ],
    )
    .unwrap();

    // Should find active recording (ended_at is NULL)
    let active = get_active_recording_with_conn(&conn, "session-1").unwrap();
    assert!(active.is_some());
    let (id, _) = active.unwrap();
    assert_eq!(id, recording.id);

    // Should not find active recording for different session
    let active = get_active_recording_with_conn(&conn, "session-2").unwrap();
    assert!(active.is_none());

    // End the recording
    conn.execute(
        "UPDATE recordings SET ended_at = ? WHERE id = ?",
        params![Utc::now().to_rfc3339(), recording.id],
    )
    .unwrap();

    // Should not find active recording after ending
    let active = get_active_recording_with_conn(&conn, "session-1").unwrap();
    assert!(active.is_none());
}

#[test]
fn test_record_recording_event() {
    let (temp, _store) = create_test_store();
    let conn = Connection::open(temp.path().join("cas.db")).unwrap();

    // Create an active recording
    let recording = Recording::for_session("session-1".to_string(), "/path/rec.bin".to_string());
    conn.execute(
        "INSERT INTO recordings (id, session_id, started_at, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            recording.id,
            recording.session_id,
            recording.started_at.to_rfc3339(),
            recording.file_path,
            recording.created_at.to_rfc3339(),
        ],
    )
    .unwrap();

    // Record an event
    let result = record_task_event_with_conn(
        &conn,
        "session-1",
        RecordingEventType::TaskCreated,
        "cas-1234",
        None,
    )
    .unwrap();
    assert!(result.is_some());

    // Verify event was recorded
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recording_events WHERE recording_id = ?",
            params![recording.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);

    // Record another event type
    let result = record_agent_event_with_conn(
        &conn,
        "session-1",
        RecordingEventType::AgentJoined,
        "agent-1",
        Some(r#"{"name": "swift-fox"}"#),
    )
    .unwrap();
    assert!(result.is_some());

    // Verify total events
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM recording_events WHERE recording_id = ?",
            params![recording.id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_record_event_no_active_recording() {
    let (temp, _store) = create_test_store();
    let conn = Connection::open(temp.path().join("cas.db")).unwrap();

    // Try to record event without an active recording
    let result = record_task_event_with_conn(
        &conn,
        "session-1",
        RecordingEventType::TaskCreated,
        "cas-1234",
        None,
    )
    .unwrap();

    // Should return None (no active recording)
    assert!(result.is_none());
}

#[test]
fn test_record_message_event() {
    let (temp, _store) = create_test_store();
    let conn = Connection::open(temp.path().join("cas.db")).unwrap();

    // Create an active recording
    let recording = Recording::for_session("session-1".to_string(), "/path/rec.bin".to_string());
    conn.execute(
        "INSERT INTO recordings (id, session_id, started_at, file_path, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        params![
            recording.id,
            recording.session_id,
            recording.started_at.to_rfc3339(),
            recording.file_path,
            recording.created_at.to_rfc3339(),
        ],
    )
    .unwrap();

    // Record a message event
    let result =
        record_message_event_with_conn(&conn, "session-1", "swift-fox", "proud-finch").unwrap();
    assert!(result.is_some());

    // Verify event and metadata
    let (event_type, metadata): (String, Option<String>) = conn
        .query_row(
            "SELECT event_type, metadata FROM recording_events WHERE id = ?",
            params![result.unwrap()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(event_type, "message_sent");
    assert!(metadata.is_some());
    let meta: serde_json::Value = serde_json::from_str(&metadata.unwrap()).unwrap();
    assert_eq!(meta["from"], "swift-fox");
    assert_eq!(meta["to"], "proud-finch");
}
