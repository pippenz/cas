use crate::Result;
use crate::error::StoreError;
use crate::event_store::record_event_with_conn;
use crate::recording_store::capture_memory_event;
use crate::sqlite::{SCHEMA, SqliteStore};
use crate::tracing::{DevTracer, TraceTimer};
use cas_types::{Entry, EntryType, Event, EventEntityType, EventType, Scope};
use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use std::str::FromStr;

impl SqliteStore {
    pub(crate) fn store_init(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(SCHEMA)?;

        // Sessions table for enterprise analytics
        // NOTE: Column migrations are now handled by `cas update --schema-only`
        // See cas-cli/src/migration/migrations.rs for the migration definitions
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                cwd TEXT NOT NULL,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                duration_secs INTEGER,
                permission_mode TEXT,
                entries_created INTEGER NOT NULL DEFAULT 0,
                tasks_closed INTEGER NOT NULL DEFAULT 0,
                tool_uses INTEGER NOT NULL DEFAULT 0,
                team_id TEXT,
                title TEXT
            )",
            [],
        )?;
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_started ON sessions(started_at DESC)",
            [],
        );
        let _ = conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_sessions_team ON sessions(team_id)",
            [],
        );

        Ok(())
    }
    pub(crate) fn store_generate_id(&self) -> Result<String> {
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().unwrap();

        // Use MAX instead of COUNT to handle gaps from deleted entries
        let max_num: Option<i32> = conn
            .query_row(
                "SELECT MAX(CAST(SUBSTR(id, 12) AS INTEGER)) FROM entries WHERE id LIKE ?",
                params![format!("{}-%", today)],
                |row| row.get(0),
            )
            .ok();

        let next_num = max_num.unwrap_or(0) + 1;
        Ok(format!("{today}-{next_num}"))
    }
    pub(crate) fn store_add(&self, entry: &Entry) -> Result<()> {
        let timer = TraceTimer::new();
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            let result = conn.execute(
            "INSERT INTO entries (id, type, tags, created, content, title,
             helpful_count, harmful_count, last_accessed, archived,
             session_id, source_tool, pending_extraction, observation_type,
             stability, access_count, raw_content, compressed, memory_tier, importance,
             valid_from, valid_until, review_after, last_reviewed, pending_embedding, belief_type, confidence, domain, branch, scope, team_id, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30, ?31, ?32)",
            params![
                entry.id,
                entry.entry_type.to_string(),
                Self::tags_to_string(&entry.tags),
                entry.created.to_rfc3339(),
                entry.content,
                entry.title,
                entry.helpful_count,
                entry.harmful_count,
                entry.last_accessed.map(|t| t.to_rfc3339()),
                entry.archived as i32,
                entry.session_id,
                entry.source_tool,
                entry.pending_extraction as i32,
                entry.observation_type.map(|t| t.to_string()),
                entry.stability,
                entry.access_count,
                entry.raw_content,
                entry.compressed as i32,
                entry.memory_tier.to_string(),
                entry.importance,
                entry.valid_from.map(|t| t.to_rfc3339()),
                entry.valid_until.map(|t| t.to_rfc3339()),
                entry.review_after.map(|t| t.to_rfc3339()),
                entry.last_reviewed.map(|t| t.to_rfc3339()),
                entry.pending_embedding as i32,
                entry.belief_type.to_string(),
                entry.confidence,
                entry.domain,
                entry.branch,
                entry.scope.to_string(),
                entry.team_id,
                now, // updated_at = created time for new entries
            ],
        );

            // Record trace (only if store ops tracing is enabled)
            if let Some(tracer) = DevTracer::get() {
                if tracer.should_trace_store_ops() {
                    let (success, error) = match &result {
                        Ok(_) => (true, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tracer.record_store_op(
                        "add",
                        "sqlite",
                        &[entry.id.as_str()],
                        if success { 1 } else { 0 },
                        timer.elapsed_ms(),
                        success,
                        error.as_deref(),
                    );
                }
            }

            result?;

            // Record event for sidecar activity feed
            let summary = entry.title.as_deref().unwrap_or_else(|| {
                // Truncate content for summary
                if entry.content.len() > 50 {
                    &entry.content[..50]
                } else {
                    &entry.content
                }
            });
            let event = Event::new(
                EventType::MemoryStored,
                EventEntityType::Entry,
                &entry.id,
                format!("Memory stored: {summary}"),
            )
            .with_session(entry.session_id.as_deref().unwrap_or(""));
            let _ = record_event_with_conn(&conn, &event);

            // Capture event for recording playback
            let _ = capture_memory_event(&conn, &entry.id, None);

            Ok(())
        }) // with_write_retry
    }
    pub(crate) fn store_get(&self, id: &str) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let entry = conn
            .query_row(
                "SELECT id, type, tags, created, content, title, helpful_count,
                 harmful_count, last_accessed, archived, session_id, source_tool,
                 pending_extraction, observation_type, stability, access_count,
                 raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
                 belief_type, confidence, domain, branch, scope, team_id
                 FROM entries WHERE id = ? AND archived = 0",
                params![id],
                |row| {
                    Ok(Entry {
                        id: row.get(0)?,
                        entry_type: row
                            .get::<_, String>(1)?
                            .parse()
                            .unwrap_or(EntryType::Learning),
                        observation_type: Self::parse_observation_type(row.get(13)?),
                        tags: Self::parse_tags(row.get(2)?),
                        created: Self::parse_datetime(&row.get::<_, String>(3)?)
                            .unwrap_or_else(Utc::now),
                        content: row.get(4)?,
                        raw_content: row.get(16)?,
                        compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                        memory_tier: Self::parse_memory_tier(row.get(18)?),
                        title: row.get(5)?,
                        helpful_count: row.get(6)?,
                        harmful_count: row.get(7)?,
                        last_accessed: row
                            .get::<_, Option<String>>(8)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        archived: row.get::<_, i32>(9)? != 0,
                        session_id: row.get(10)?,
                        source_tool: row.get(11)?,
                        pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                        stability: row.get::<_, f32>(14).unwrap_or(0.5),
                        access_count: row.get::<_, i32>(15).unwrap_or(0),
                        importance: row.get::<_, f32>(19).unwrap_or(0.5),
                        valid_from: row
                            .get::<_, Option<String>>(20)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        valid_until: row
                            .get::<_, Option<String>>(21)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        review_after: row
                            .get::<_, Option<String>>(22)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        last_reviewed: row
                            .get::<_, Option<String>>(23)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                        belief_type: Self::parse_belief_type(row.get(25)?),
                        confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                        domain: row.get(27)?,
                        branch: row.get(28)?,
                        scope: row
                            .get::<_, Option<String>>(29)?
                            .map(|s| Scope::from_str(&s).unwrap_or_default())
                            .unwrap_or_default(),
                        team_id: row.get(30)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::EntryNotFound(id.to_string()))?;
        Ok(entry)
    }
    pub(crate) fn store_get_archived(&self, id: &str) -> Result<Entry> {
        let conn = self.conn.lock().unwrap();
        let entry = conn
            .query_row(
                "SELECT id, type, tags, created, content, title, helpful_count,
                 harmful_count, last_accessed, archived, session_id, source_tool,
                 pending_extraction, observation_type, stability, access_count,
                 raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
                 belief_type, confidence, domain, branch, scope, team_id
                 FROM entries WHERE id = ? AND archived = 1",
                params![id],
                |row| {
                    Ok(Entry {
                        id: row.get(0)?,
                        entry_type: row
                            .get::<_, String>(1)?
                            .parse()
                            .unwrap_or(EntryType::Learning),
                        observation_type: Self::parse_observation_type(row.get(13)?),
                        tags: Self::parse_tags(row.get(2)?),
                        created: Self::parse_datetime(&row.get::<_, String>(3)?)
                            .unwrap_or_else(Utc::now),
                        content: row.get(4)?,
                        raw_content: row.get(16)?,
                        compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                        memory_tier: Self::parse_memory_tier(row.get(18)?),
                        title: row.get(5)?,
                        helpful_count: row.get(6)?,
                        harmful_count: row.get(7)?,
                        last_accessed: row
                            .get::<_, Option<String>>(8)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        archived: row.get::<_, i32>(9)? != 0,
                        session_id: row.get(10)?,
                        source_tool: row.get(11)?,
                        pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                        stability: row.get::<_, f32>(14).unwrap_or(0.5),
                        access_count: row.get::<_, i32>(15).unwrap_or(0),
                        importance: row.get::<_, f32>(19).unwrap_or(0.5),
                        valid_from: row
                            .get::<_, Option<String>>(20)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        valid_until: row
                            .get::<_, Option<String>>(21)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        review_after: row
                            .get::<_, Option<String>>(22)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        last_reviewed: row
                            .get::<_, Option<String>>(23)?
                            .and_then(|s| Self::parse_datetime(&s)),
                        pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                        belief_type: Self::parse_belief_type(row.get(25)?),
                        confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                        domain: row.get(27)?,
                        branch: row.get(28)?,
                        scope: row
                            .get::<_, Option<String>>(29)?
                            .map(|s| Scope::from_str(&s).unwrap_or_default())
                            .unwrap_or_default(),
                        team_id: row.get(30)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| StoreError::EntryNotFound(id.to_string()))?;
        Ok(entry)
    }
    pub(crate) fn store_update(&self, entry: &Entry) -> Result<()> {
        let timer = TraceTimer::new();
        crate::shared_db::with_write_retry(|| {
            let conn = self.conn.lock().unwrap();
            let now = Utc::now().to_rfc3339();
            let result = conn.execute(
            "UPDATE entries SET type = ?1, tags = ?2, content = ?3, title = ?4,
             helpful_count = ?5, harmful_count = ?6, last_accessed = ?7, archived = ?8,
             session_id = ?9, source_tool = ?10, pending_extraction = ?11, observation_type = ?12,
             stability = ?13, access_count = ?14, raw_content = ?15, compressed = ?16,
             memory_tier = ?17, importance = ?18, valid_from = ?19, valid_until = ?20, review_after = ?21,
             last_reviewed = ?22, pending_embedding = ?23, belief_type = ?24, confidence = ?25, domain = ?26, branch = ?27,
             updated_at = ?28, scope = ?29
             WHERE id = ?30",
            params![
                entry.entry_type.to_string(),
                Self::tags_to_string(&entry.tags),
                entry.content,
                entry.title,
                entry.helpful_count,
                entry.harmful_count,
                entry.last_accessed.map(|t| t.to_rfc3339()),
                entry.archived as i32,
                entry.session_id,
                entry.source_tool,
                entry.pending_extraction as i32,
                entry.observation_type.map(|t| t.to_string()),
                entry.stability,
                entry.access_count,
                entry.raw_content,
                entry.compressed as i32,
                entry.memory_tier.to_string(),
                entry.importance,
                entry.valid_from.map(|t| t.to_rfc3339()),
                entry.valid_until.map(|t| t.to_rfc3339()),
                entry.review_after.map(|t| t.to_rfc3339()),
                entry.last_reviewed.map(|t| t.to_rfc3339()),
                entry.pending_embedding as i32,
                entry.belief_type.to_string(),
                entry.confidence,
                entry.domain,
                entry.branch,
                now, // updated_at = current time on update
                entry.scope.to_string(),
                entry.id,
            ],
        );

            // Record trace (only if store ops tracing is enabled)
            if let Some(tracer) = DevTracer::get() {
                if tracer.should_trace_store_ops() {
                    let (success, error) = match &result {
                        Ok(rows) => (*rows > 0, None),
                        Err(e) => (false, Some(e.to_string())),
                    };
                    let _ = tracer.record_store_op(
                        "update",
                        "sqlite",
                        &[entry.id.as_str()],
                        result.as_ref().copied().unwrap_or(0),
                        timer.elapsed_ms(),
                        success,
                        error.as_deref(),
                    );
                }
            }

            let rows = result?;
            if rows == 0 {
                return Err(StoreError::EntryNotFound(entry.id.clone()));
            }
            Ok(())
        }) // with_write_retry
    }
    pub(crate) fn store_delete(&self, id: &str) -> Result<()> {
        let timer = TraceTimer::new();
        let conn = self.conn.lock().unwrap();
        let result = conn.execute("DELETE FROM entries WHERE id = ?", params![id]);

        // Record trace (only if store ops tracing is enabled)
        if let Some(tracer) = DevTracer::get() {
            if tracer.should_trace_store_ops() {
                let (success, error) = match &result {
                    Ok(rows) => (*rows > 0, None),
                    Err(e) => (false, Some(e.to_string())),
                };
                let _ = tracer.record_store_op(
                    "delete",
                    "sqlite",
                    &[id],
                    result.as_ref().copied().unwrap_or(0),
                    timer.elapsed_ms(),
                    success,
                    error.as_deref(),
                );
            }
        }

        let rows = result?;
        if rows == 0 {
            return Err(StoreError::EntryNotFound(id.to_string()));
        }
        Ok(())
    }
    pub(crate) fn store_list(&self) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE archived = 0 ORDER BY created DESC",
        )?;

        let entries = stmt
            .query_map([], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    entry_type: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or(EntryType::Learning),
                    observation_type: Self::parse_observation_type(row.get(13)?),
                    tags: Self::parse_tags(row.get(2)?),
                    created: Self::parse_datetime(&row.get::<_, String>(3)?)
                        .unwrap_or_else(Utc::now),
                    content: row.get(4)?,
                    raw_content: row.get(16)?,
                    compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                    memory_tier: Self::parse_memory_tier(row.get(18)?),
                    title: row.get(5)?,
                    helpful_count: row.get(6)?,
                    harmful_count: row.get(7)?,
                    last_accessed: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    archived: row.get::<_, i32>(9)? != 0,
                    session_id: row.get(10)?,
                    source_tool: row.get(11)?,
                    pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                    stability: row.get::<_, f32>(14).unwrap_or(0.5),
                    access_count: row.get::<_, i32>(15).unwrap_or(0),
                    importance: row.get::<_, f32>(19).unwrap_or(0.5),
                    valid_from: row
                        .get::<_, Option<String>>(20)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    valid_until: row
                        .get::<_, Option<String>>(21)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    review_after: row
                        .get::<_, Option<String>>(22)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    last_reviewed: row
                        .get::<_, Option<String>>(23)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                    belief_type: Self::parse_belief_type(row.get(25)?),
                    confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                    domain: row.get(27)?,
                    branch: row.get(28)?,
                    scope: row
                        .get::<_, Option<String>>(29)?
                        .map(|s| Scope::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    team_id: row.get(30)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }
    pub(crate) fn store_recent(&self, n: usize) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE archived = 0 ORDER BY created DESC LIMIT ?",
        )?;

        let entries = stmt
            .query_map(params![n as i64], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    entry_type: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or(EntryType::Learning),
                    observation_type: Self::parse_observation_type(row.get(13)?),
                    tags: Self::parse_tags(row.get(2)?),
                    created: Self::parse_datetime(&row.get::<_, String>(3)?)
                        .unwrap_or_else(Utc::now),
                    content: row.get(4)?,
                    raw_content: row.get(16)?,
                    compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                    memory_tier: Self::parse_memory_tier(row.get(18)?),
                    title: row.get(5)?,
                    helpful_count: row.get(6)?,
                    harmful_count: row.get(7)?,
                    last_accessed: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    archived: row.get::<_, i32>(9)? != 0,
                    session_id: row.get(10)?,
                    source_tool: row.get(11)?,
                    pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                    stability: row.get::<_, f32>(14).unwrap_or(0.5),
                    access_count: row.get::<_, i32>(15).unwrap_or(0),
                    importance: row.get::<_, f32>(19).unwrap_or(0.5),
                    valid_from: row
                        .get::<_, Option<String>>(20)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    valid_until: row
                        .get::<_, Option<String>>(21)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    review_after: row
                        .get::<_, Option<String>>(22)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    last_reviewed: row
                        .get::<_, Option<String>>(23)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                    belief_type: Self::parse_belief_type(row.get(25)?),
                    confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                    domain: row.get(27)?,
                    branch: row.get(28)?,
                    scope: row
                        .get::<_, Option<String>>(29)?
                        .map(|s| Scope::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    team_id: row.get(30)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }
    pub(crate) fn store_archive(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE entries SET archived = 1 WHERE id = ? AND archived = 0",
            params![id],
        )?;
        if rows == 0 {
            return Err(StoreError::EntryNotFound(id.to_string()));
        }
        Ok(())
    }
    pub(crate) fn store_unarchive(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE entries SET archived = 0 WHERE id = ? AND archived = 1",
            params![id],
        )?;
        if rows == 0 {
            return Err(StoreError::EntryNotFound(id.to_string()));
        }
        Ok(())
    }
    pub(crate) fn store_list_archived(&self) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE archived = 1 ORDER BY created DESC",
        )?;

        let entries = stmt
            .query_map([], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    entry_type: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or(EntryType::Learning),
                    observation_type: Self::parse_observation_type(row.get(13)?),
                    tags: Self::parse_tags(row.get(2)?),
                    created: Self::parse_datetime(&row.get::<_, String>(3)?)
                        .unwrap_or_else(Utc::now),
                    content: row.get(4)?,
                    raw_content: row.get(16)?,
                    compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                    memory_tier: Self::parse_memory_tier(row.get(18)?),
                    title: row.get(5)?,
                    helpful_count: row.get(6)?,
                    harmful_count: row.get(7)?,
                    last_accessed: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    archived: row.get::<_, i32>(9)? != 0,
                    session_id: row.get(10)?,
                    source_tool: row.get(11)?,
                    pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                    stability: row.get::<_, f32>(14).unwrap_or(0.5),
                    access_count: row.get::<_, i32>(15).unwrap_or(0),
                    importance: row.get::<_, f32>(19).unwrap_or(0.5),
                    valid_from: row
                        .get::<_, Option<String>>(20)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    valid_until: row
                        .get::<_, Option<String>>(21)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    review_after: row
                        .get::<_, Option<String>>(22)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    last_reviewed: row
                        .get::<_, Option<String>>(23)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                    belief_type: Self::parse_belief_type(row.get(25)?),
                    confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                    domain: row.get(27)?,
                    branch: row.get(28)?,
                    scope: row
                        .get::<_, Option<String>>(29)?
                        .map(|s| Scope::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    team_id: row.get(30)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }

    pub(crate) fn store_list_by_branch(&self, branch: &str) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE branch = ? AND archived = 0 ORDER BY created DESC",
        )?;

        let entries = stmt
            .query_map(params![branch], |row| {
                Ok(Entry {
                    id: row.get(0)?,
                    entry_type: row
                        .get::<_, String>(1)?
                        .parse()
                        .unwrap_or(EntryType::Learning),
                    observation_type: Self::parse_observation_type(row.get(13)?),
                    tags: Self::parse_tags(row.get(2)?),
                    created: Self::parse_datetime(&row.get::<_, String>(3)?)
                        .unwrap_or_else(Utc::now),
                    content: row.get(4)?,
                    raw_content: row.get(16)?,
                    compressed: row.get::<_, i32>(17).unwrap_or(0) != 0,
                    memory_tier: Self::parse_memory_tier(row.get(18)?),
                    title: row.get(5)?,
                    helpful_count: row.get(6)?,
                    harmful_count: row.get(7)?,
                    last_accessed: row
                        .get::<_, Option<String>>(8)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    archived: row.get::<_, i32>(9)? != 0,
                    session_id: row.get(10)?,
                    source_tool: row.get(11)?,
                    pending_extraction: row.get::<_, i32>(12).unwrap_or(0) != 0,
                    stability: row.get::<_, f32>(14).unwrap_or(0.5),
                    access_count: row.get::<_, i32>(15).unwrap_or(0),
                    importance: row.get::<_, f32>(19).unwrap_or(0.5),
                    valid_from: row
                        .get::<_, Option<String>>(20)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    valid_until: row
                        .get::<_, Option<String>>(21)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    review_after: row
                        .get::<_, Option<String>>(22)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    last_reviewed: row
                        .get::<_, Option<String>>(23)?
                        .and_then(|s| Self::parse_datetime(&s)),
                    pending_embedding: row.get::<_, i32>(24).unwrap_or(1) != 0,
                    belief_type: Self::parse_belief_type(row.get(25)?),
                    confidence: row.get::<_, f32>(26).unwrap_or(1.0),
                    domain: row.get(27)?,
                    branch: row.get(28)?,
                    scope: row
                        .get::<_, Option<String>>(29)?
                        .map(|s| Scope::from_str(&s).unwrap_or_default())
                        .unwrap_or_default(),
                    team_id: row.get(30)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(entries)
    }
}
