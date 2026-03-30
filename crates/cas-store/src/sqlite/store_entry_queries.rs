use crate::Result;
use crate::error::StoreError;
use crate::sqlite::SqliteStore;
use cas_types::{Entry, EntryType, Scope};
use chrono::Utc;
use rusqlite::params;
use std::str::FromStr;

impl SqliteStore {
    pub(crate) fn store_list_pending(&self, limit: usize) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE pending_extraction = 1 AND archived = 0
             ORDER BY created DESC LIMIT ?",
        )?;

        let entries = stmt
            .query_map(params![limit as i64], |row| {
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
    pub(crate) fn store_mark_extracted(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE entries SET pending_extraction = 0 WHERE id = ?",
            params![id],
        )?;
        if rows == 0 {
            return Err(StoreError::EntryNotFound(id.to_string()));
        }
        Ok(())
    }
    pub(crate) fn store_list_pinned(&self) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries WHERE memory_tier = 'in-context' AND archived = 0
             ORDER BY created DESC",
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
    pub(crate) fn store_list_helpful(&self, limit: usize) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries
             WHERE archived = 0 AND (helpful_count - harmful_count) > 0
             ORDER BY (helpful_count - harmful_count) DESC, last_accessed DESC
             LIMIT ?",
        )?;

        let entries = stmt
            .query_map([limit as i64], |row| {
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
    pub(crate) fn store_list_by_session(&self, session_id: &str) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries
             WHERE session_id = ? AND archived = 0
             ORDER BY created DESC",
        )?;

        let entries = stmt
            .query_map([session_id], |row| {
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
    pub(crate) fn store_list_unreviewed_learnings(&self, limit: usize) -> Result<Vec<Entry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare_cached(
            "SELECT id, type, tags, created, content, title, helpful_count,
             harmful_count, last_accessed, archived, session_id, source_tool,
             pending_extraction, observation_type, stability, access_count,
             raw_content, compressed, memory_tier, importance, valid_from, valid_until, review_after, last_reviewed, pending_embedding,
             belief_type, confidence, domain, branch, scope, team_id
             FROM entries
             WHERE type = 'learning' AND archived = 0 AND last_reviewed IS NULL
             ORDER BY created DESC
             LIMIT ?",
        )?;

        let entries = stmt
            .query_map([limit as i64], |row| {
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
    pub(crate) fn store_mark_reviewed(&self, id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();
        let rows = conn.execute(
            "UPDATE entries SET last_reviewed = ? WHERE id = ?",
            params![now, id],
        )?;
        if rows == 0 {
            return Err(StoreError::EntryNotFound(id.to_string()));
        }
        Ok(())
    }
}
