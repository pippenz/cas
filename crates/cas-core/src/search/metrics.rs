//! Search quality metrics and telemetry
//!
//! Tracks search performance, result quality, and enables A/B testing
//! between different search methods (BM25, semantic, hybrid).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

use crate::error::CoreError;

/// Search method being used
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SearchMethod {
    /// BM25 text search only
    Bm25,
    /// Semantic embedding search only
    Semantic,
    /// Hybrid BM25 + semantic
    Hybrid,
    /// Hybrid with reranking
    HybridReranked,
}

impl std::fmt::Display for SearchMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchMethod::Bm25 => write!(f, "bm25"),
            SearchMethod::Semantic => write!(f, "semantic"),
            SearchMethod::Hybrid => write!(f, "hybrid"),
            SearchMethod::HybridReranked => write!(f, "hybrid_reranked"),
        }
    }
}

/// A recorded search event with timing and results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchEvent {
    /// Unique event ID
    pub id: String,
    /// Timestamp of the search
    pub timestamp: DateTime<Utc>,
    /// The search query
    pub query: String,
    /// Search method used
    pub method: SearchMethod,
    /// Time taken for the search (milliseconds)
    pub latency_ms: u64,
    /// Number of results returned
    pub result_count: usize,
    /// IDs of results in order
    pub result_ids: Vec<String>,
    /// Session ID if available
    pub session_id: Option<String>,
}

/// Feedback on a search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultFeedback {
    /// Search event ID
    pub search_id: String,
    /// Result ID that received feedback
    pub result_id: String,
    /// Position in results (0-indexed)
    pub position: usize,
    /// Whether the result was helpful
    pub helpful: bool,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
}

/// Aggregated metrics for a search method
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MethodMetrics {
    /// Total number of searches
    pub search_count: u64,
    /// Total latency (milliseconds)
    pub total_latency_ms: u64,
    /// Number of results returned across all searches
    pub total_results: u64,
    /// Helpful feedback count
    pub helpful_count: u64,
    /// Harmful feedback count
    pub harmful_count: u64,
    /// Mean Reciprocal Rank (1/position for first helpful result)
    pub mrr_sum: f64,
    /// Number of searches with feedback
    pub mrr_count: u64,
}

impl MethodMetrics {
    /// Calculate average latency in milliseconds
    pub fn avg_latency_ms(&self) -> f64 {
        if self.search_count == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.search_count as f64
        }
    }

    /// Calculate average results per search
    pub fn avg_results(&self) -> f64 {
        if self.search_count == 0 {
            0.0
        } else {
            self.total_results as f64 / self.search_count as f64
        }
    }

    /// Calculate Mean Reciprocal Rank
    pub fn mrr(&self) -> f64 {
        if self.mrr_count == 0 {
            0.0
        } else {
            self.mrr_sum / self.mrr_count as f64
        }
    }

    /// Calculate precision (helpful / (helpful + harmful))
    pub fn precision(&self) -> f64 {
        let total = self.helpful_count + self.harmful_count;
        if total == 0 {
            0.0
        } else {
            self.helpful_count as f64 / total as f64
        }
    }
}

/// Search metrics store
pub struct MetricsStore {
    db: rusqlite::Connection,
}

impl MetricsStore {
    /// Open or create metrics store
    pub fn open(path: &Path) -> Result<Self, CoreError> {
        let db = rusqlite::Connection::open(path)?;

        db.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS search_events (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                query TEXT NOT NULL,
                method TEXT NOT NULL,
                latency_ms INTEGER NOT NULL,
                result_count INTEGER NOT NULL,
                result_ids TEXT NOT NULL,
                session_id TEXT
            );

            CREATE TABLE IF NOT EXISTS result_feedback (
                search_id TEXT NOT NULL,
                result_id TEXT NOT NULL,
                position INTEGER NOT NULL,
                helpful INTEGER NOT NULL,
                timestamp TEXT NOT NULL,
                PRIMARY KEY (search_id, result_id)
            );

            CREATE INDEX IF NOT EXISTS idx_events_method ON search_events(method);
            CREATE INDEX IF NOT EXISTS idx_events_timestamp ON search_events(timestamp);
            CREATE INDEX IF NOT EXISTS idx_feedback_search ON result_feedback(search_id);
            "#,
        )?;

        Ok(Self { db })
    }

    /// Record a search event
    pub fn record_search(&self, event: &SearchEvent) -> Result<(), CoreError> {
        self.db.execute(
            r#"
            INSERT OR REPLACE INTO search_events
            (id, timestamp, query, method, latency_ms, result_count, result_ids, session_id)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            "#,
            rusqlite::params![
                event.id,
                event.timestamp.to_rfc3339(),
                event.query,
                event.method.to_string(),
                event.latency_ms as i64,
                event.result_count as i64,
                serde_json::to_string(&event.result_ids).unwrap_or_default(),
                event.session_id,
            ],
        )?;

        Ok(())
    }

    /// Record feedback on a search result
    pub fn record_feedback(&self, feedback: &ResultFeedback) -> Result<(), CoreError> {
        self.db.execute(
            r#"
            INSERT OR REPLACE INTO result_feedback
            (search_id, result_id, position, helpful, timestamp)
            VALUES (?1, ?2, ?3, ?4, ?5)
            "#,
            rusqlite::params![
                feedback.search_id,
                feedback.result_id,
                feedback.position as i64,
                feedback.helpful as i32,
                feedback.timestamp.to_rfc3339(),
            ],
        )?;

        Ok(())
    }

    /// Get aggregated metrics for each search method
    pub fn get_method_metrics(&self) -> Result<HashMap<SearchMethod, MethodMetrics>, CoreError> {
        let mut metrics = HashMap::new();

        // Initialize all methods
        metrics.insert(SearchMethod::Bm25, MethodMetrics::default());
        metrics.insert(SearchMethod::Semantic, MethodMetrics::default());
        metrics.insert(SearchMethod::Hybrid, MethodMetrics::default());
        metrics.insert(SearchMethod::HybridReranked, MethodMetrics::default());

        // Aggregate search events
        let mut stmt = self.db.prepare(
            r#"
            SELECT method, COUNT(*), SUM(latency_ms), SUM(result_count)
            FROM search_events
            GROUP BY method
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        for row in rows {
            let (method_str, count, latency, results) = row?;
            let (count, latency, results) = (count as u64, latency as u64, results as u64);
            let method = match method_str.as_str() {
                "bm25" => SearchMethod::Bm25,
                "semantic" => SearchMethod::Semantic,
                "hybrid" => SearchMethod::Hybrid,
                "hybrid_reranked" => SearchMethod::HybridReranked,
                _ => continue,
            };

            if let Some(m) = metrics.get_mut(&method) {
                m.search_count = count;
                m.total_latency_ms = latency;
                m.total_results = results;
            }
        }

        // Aggregate feedback
        let mut stmt = self.db.prepare(
            r#"
            SELECT e.method, f.helpful, f.position, COUNT(*)
            FROM result_feedback f
            JOIN search_events e ON f.search_id = e.id
            GROUP BY e.method, f.helpful
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i32>(1)?,
                row.get::<_, i32>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })?;

        for row in rows {
            let (method_str, helpful, _position, count) = row?;
            let count = count as u64;
            let method = match method_str.as_str() {
                "bm25" => SearchMethod::Bm25,
                "semantic" => SearchMethod::Semantic,
                "hybrid" => SearchMethod::Hybrid,
                "hybrid_reranked" => SearchMethod::HybridReranked,
                _ => continue,
            };

            if let Some(m) = metrics.get_mut(&method) {
                if helpful == 1 {
                    m.helpful_count += count;
                } else {
                    m.harmful_count += count;
                }
            }
        }

        // Calculate MRR (Mean Reciprocal Rank)
        let mut stmt = self.db.prepare(
            r#"
            SELECT e.method, MIN(f.position) as first_pos
            FROM result_feedback f
            JOIN search_events e ON f.search_id = e.id
            WHERE f.helpful = 1
            GROUP BY f.search_id
            "#,
        )?;

        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i32>(1)?))
        })?;

        for row in rows {
            let (method_str, first_pos) = row?;
            let method = match method_str.as_str() {
                "bm25" => SearchMethod::Bm25,
                "semantic" => SearchMethod::Semantic,
                "hybrid" => SearchMethod::Hybrid,
                "hybrid_reranked" => SearchMethod::HybridReranked,
                _ => continue,
            };

            if let Some(m) = metrics.get_mut(&method) {
                // MRR = 1 / (position + 1) since position is 0-indexed
                m.mrr_sum += 1.0 / (first_pos as f64 + 1.0);
                m.mrr_count += 1;
            }
        }

        Ok(metrics)
    }

    /// Get recent search events
    pub fn get_recent_events(&self, limit: usize) -> Result<Vec<SearchEvent>, CoreError> {
        let mut stmt = self.db.prepare(
            r#"
            SELECT id, timestamp, query, method, latency_ms, result_count, result_ids, session_id
            FROM search_events
            ORDER BY timestamp DESC
            LIMIT ?
            "#,
        )?;

        let rows = stmt.query_map([limit as i64], |row| {
            let method_str: String = row.get(3)?;
            let method = match method_str.as_str() {
                "bm25" => SearchMethod::Bm25,
                "semantic" => SearchMethod::Semantic,
                "hybrid" => SearchMethod::Hybrid,
                "hybrid_reranked" => SearchMethod::HybridReranked,
                _ => SearchMethod::Bm25,
            };

            let result_ids_json: String = row.get(6)?;
            let result_ids: Vec<String> =
                serde_json::from_str(&result_ids_json).unwrap_or_default();

            Ok(SearchEvent {
                id: row.get(0)?,
                timestamp: DateTime::parse_from_rfc3339(&row.get::<_, String>(1)?)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
                query: row.get(2)?,
                method,
                latency_ms: row.get::<_, i64>(4)? as u64,
                result_count: row.get::<_, i64>(5)? as usize,
                result_ids,
                session_id: row.get(7)?,
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(CoreError::Database)
    }

    /// Get comparison stats between two methods
    pub fn compare_methods(
        &self,
        method_a: SearchMethod,
        method_b: SearchMethod,
    ) -> Result<MethodComparison, CoreError> {
        let metrics = self.get_method_metrics()?;

        let a = metrics.get(&method_a).cloned().unwrap_or_default();
        let b = metrics.get(&method_b).cloned().unwrap_or_default();

        Ok(MethodComparison {
            method_a,
            method_b,
            latency_diff_pct: if a.avg_latency_ms() > 0.0 {
                ((b.avg_latency_ms() - a.avg_latency_ms()) / a.avg_latency_ms()) * 100.0
            } else {
                0.0
            },
            precision_diff: b.precision() - a.precision(),
            mrr_diff: b.mrr() - a.mrr(),
            a_metrics: a,
            b_metrics: b,
        })
    }
}

/// Comparison between two search methods
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MethodComparison {
    pub method_a: SearchMethod,
    pub method_b: SearchMethod,
    /// Latency difference as percentage (positive = B is slower)
    pub latency_diff_pct: f64,
    /// Precision difference (positive = B is better)
    pub precision_diff: f64,
    /// MRR difference (positive = B is better)
    pub mrr_diff: f64,
    pub a_metrics: MethodMetrics,
    pub b_metrics: MethodMetrics,
}

impl MethodComparison {
    /// Determine if method B is significantly better
    pub fn b_is_better(&self) -> bool {
        // B is better if it has higher precision and MRR, even if slower
        self.precision_diff > 0.05 || self.mrr_diff > 0.05
    }

    /// Determine if method A is significantly better
    pub fn a_is_better(&self) -> bool {
        self.precision_diff < -0.05 || self.mrr_diff < -0.05
    }
}

/// Helper to generate unique event IDs
pub fn generate_event_id() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_micros();

    format!("evt-{timestamp:x}")
}

/// Measure search latency
pub struct LatencyTimer {
    start: std::time::Instant,
}

impl LatencyTimer {
    pub fn new() -> Self {
        Self {
            start: std::time::Instant::now(),
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }
}

impl Default for LatencyTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::search::metrics::*;
    use tempfile::TempDir;

    #[test]
    fn test_metrics_store() {
        let temp = TempDir::new().unwrap();
        let store = MetricsStore::open(&temp.path().join("metrics.db")).unwrap();

        // Record a search event
        let event = SearchEvent {
            id: "evt-001".to_string(),
            timestamp: Utc::now(),
            query: "test query".to_string(),
            method: SearchMethod::Hybrid,
            latency_ms: 150,
            result_count: 5,
            result_ids: vec!["a".to_string(), "b".to_string()],
            session_id: None,
        };

        store.record_search(&event).unwrap();

        // Record feedback
        let feedback = ResultFeedback {
            search_id: "evt-001".to_string(),
            result_id: "a".to_string(),
            position: 0,
            helpful: true,
            timestamp: Utc::now(),
        };

        store.record_feedback(&feedback).unwrap();

        // Get metrics
        let metrics = store.get_method_metrics().unwrap();
        let hybrid = metrics.get(&SearchMethod::Hybrid).unwrap();

        assert_eq!(hybrid.search_count, 1);
        assert_eq!(hybrid.total_latency_ms, 150);
        assert_eq!(hybrid.helpful_count, 1);
    }

    #[test]
    fn test_method_comparison() {
        let temp = TempDir::new().unwrap();
        let store = MetricsStore::open(&temp.path().join("metrics.db")).unwrap();

        // Record BM25 searches
        for i in 0..10 {
            store
                .record_search(&SearchEvent {
                    id: format!("bm25-{i}"),
                    timestamp: Utc::now(),
                    query: "test".to_string(),
                    method: SearchMethod::Bm25,
                    latency_ms: 50,
                    result_count: 5,
                    result_ids: vec!["a".to_string()],
                    session_id: None,
                })
                .unwrap();
        }

        // Record Hybrid searches (slower but assume better)
        for i in 0..10 {
            store
                .record_search(&SearchEvent {
                    id: format!("hybrid-{i}"),
                    timestamp: Utc::now(),
                    query: "test".to_string(),
                    method: SearchMethod::Hybrid,
                    latency_ms: 200,
                    result_count: 5,
                    result_ids: vec!["a".to_string()],
                    session_id: None,
                })
                .unwrap();
        }

        let comparison = store
            .compare_methods(SearchMethod::Bm25, SearchMethod::Hybrid)
            .unwrap();

        // Hybrid is slower
        assert!(comparison.latency_diff_pct > 0.0);
    }

    #[test]
    fn test_latency_timer() {
        let timer = LatencyTimer::new();
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(timer.elapsed_ms() >= 10);
    }
}
