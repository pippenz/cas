//! Query caching for search operations
//!
//! Provides LRU caching with TTL expiration for:
//! - Query embeddings (most expensive to compute)
//! - Semantic search results
//! - Hybrid search results
//!
//! Cache invalidation is triggered when entries are modified/deleted.
//!
//! Implementation uses `LinkedHashMap` from `hashlink` for O(1) LRU operations.

use hashlink::LinkedHashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::hybrid_search::hybrid::HybridSearchResult;

type SemanticResultsCache = Arc<Mutex<LruCache<u64, Vec<(String, f32)>>>>;

/// Compute a hash of the search options for cache keys
pub fn hash_options<H: Hash>(value: &H) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

/// A cached item with expiration time
#[derive(Clone)]
struct CacheEntry<V> {
    value: V,
    expires_at: Instant,
    /// Entry IDs this result depends on (for invalidation)
    depends_on: Vec<String>,
}

/// Simple LRU cache with TTL expiration
///
/// Uses `LinkedHashMap` for O(1) lookup, insertion, removal, and LRU reordering.
/// The map maintains insertion order, with front = oldest, back = newest.
pub struct LruCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Maximum number of entries
    capacity: usize,
    /// Time-to-live for entries
    ttl: Duration,
    /// Main storage with LRU ordering (front = oldest, back = newest)
    entries: LinkedHashMap<K, CacheEntry<V>>,
}

impl<K, V> LruCache<K, V>
where
    K: Eq + Hash + Clone,
    V: Clone,
{
    /// Create a new LRU cache with specified capacity and TTL
    pub fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            capacity,
            ttl,
            entries: LinkedHashMap::with_capacity(capacity),
        }
    }

    /// Get a value from the cache, updating LRU order - O(1)
    pub fn get(&mut self, key: &K) -> Option<V> {
        // Check if entry exists and is valid
        let is_expired = self
            .entries
            .get(key)
            .map(|e| e.expires_at <= Instant::now())
            .unwrap_or(true);

        if is_expired {
            // Entry doesn't exist or expired, remove if it exists
            self.entries.remove(key);
            return None;
        }

        // Move to back of LRU order - O(1)
        self.entries.to_back(key);

        // Return cloned value
        self.entries.get(key).map(|e| e.value.clone())
    }

    /// Insert a value into the cache
    pub fn insert(&mut self, key: K, value: V) {
        self.insert_with_deps(key, value, Vec::new());
    }

    /// Insert a value with dependency tracking for invalidation - O(1) amortized
    pub fn insert_with_deps(&mut self, key: K, value: V, depends_on: Vec<String>) {
        // Remove if already exists - O(1)
        self.entries.remove(&key);

        // Evict oldest if at capacity - O(1)
        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }

        // Insert new entry at back - O(1)
        let entry = CacheEntry {
            value,
            expires_at: Instant::now() + self.ttl,
            depends_on,
        };
        self.entries.insert(key, entry);
    }

    /// Remove an entry from the cache - O(1)
    pub fn remove(&mut self, key: &K) {
        self.entries.remove(key);
    }

    /// Invalidate all entries that depend on a given entry ID - O(n) where n = matching entries
    pub fn invalidate_by_dependency(&mut self, entry_id: &str) {
        let keys_to_remove: Vec<K> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.depends_on.iter().any(|id| id == entry_id))
            .map(|(k, _)| k.clone())
            .collect();

        for key in keys_to_remove {
            self.entries.remove(&key);
        }
    }

    /// Clear all entries - O(n)
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Get cache statistics
    pub fn stats(&self) -> CacheStats {
        let now = Instant::now();
        let valid_count = self.entries.values().filter(|e| e.expires_at > now).count();

        CacheStats {
            capacity: self.capacity,
            size: self.entries.len(),
            valid_entries: valid_count,
            expired_entries: self.entries.len() - valid_count,
        }
    }

    /// Remove expired entries (can be called periodically for cleanup)
    pub fn evict_expired(&mut self) {
        let now = Instant::now();
        let expired_keys: Vec<K> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.expires_at <= now)
            .map(|(k, _)| k.clone())
            .collect();

        for key in expired_keys {
            self.entries.remove(&key);
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub capacity: usize,
    pub size: usize,
    pub valid_entries: usize,
    pub expired_entries: usize,
}

/// Unified search cache for all search operations
///
/// Thread-safe wrapper around individual LRU caches.
pub struct SearchCache {
    /// Query text -> embedding vector (most expensive to compute: ~100ms)
    query_embeddings: Arc<Mutex<LruCache<String, Vec<f32>>>>,
    /// (embedding_hash, k) -> semantic results
    semantic_results: SemanticResultsCache,
    /// options_hash -> hybrid results
    hybrid_results: Arc<Mutex<LruCache<u64, Vec<HybridSearchResult>>>>,
    /// Track cache hits/misses
    hits: Arc<Mutex<u64>>,
    misses: Arc<Mutex<u64>>,
}

impl Default for SearchCache {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchCache {
    /// Create a new search cache with default settings
    ///
    /// Default capacities and TTLs:
    /// - Query embeddings: 1000 entries, 24h TTL
    /// - Semantic results: 500 entries, 30min TTL
    /// - Hybrid results: 100 entries, 5min TTL
    pub fn new() -> Self {
        Self {
            query_embeddings: Arc::new(Mutex::new(LruCache::new(
                1000,
                Duration::from_secs(24 * 60 * 60), // 24 hours
            ))),
            semantic_results: Arc::new(Mutex::new(LruCache::new(
                500,
                Duration::from_secs(30 * 60), // 30 minutes
            ))),
            hybrid_results: Arc::new(Mutex::new(LruCache::new(
                100,
                Duration::from_secs(5 * 60), // 5 minutes
            ))),
            hits: Arc::new(Mutex::new(0)),
            misses: Arc::new(Mutex::new(0)),
        }
    }

    /// Create a search cache with custom capacities
    pub fn with_capacities(
        embedding_capacity: usize,
        semantic_capacity: usize,
        hybrid_capacity: usize,
    ) -> Self {
        Self {
            query_embeddings: Arc::new(Mutex::new(LruCache::new(
                embedding_capacity,
                Duration::from_secs(24 * 60 * 60),
            ))),
            semantic_results: Arc::new(Mutex::new(LruCache::new(
                semantic_capacity,
                Duration::from_secs(30 * 60),
            ))),
            hybrid_results: Arc::new(Mutex::new(LruCache::new(
                hybrid_capacity,
                Duration::from_secs(5 * 60),
            ))),
            hits: Arc::new(Mutex::new(0)),
            misses: Arc::new(Mutex::new(0)),
        }
    }

    // ========== Query Embedding Cache ==========

    /// Get a cached query embedding
    pub fn get_query_embedding(&self, query: &str) -> Option<Vec<f32>> {
        let mut cache = self.query_embeddings.lock().unwrap();
        if let Some(embedding) = cache.get(&query.to_string()) {
            *self.hits.lock().unwrap() += 1;
            Some(embedding)
        } else {
            *self.misses.lock().unwrap() += 1;
            None
        }
    }

    /// Cache a query embedding
    pub fn put_query_embedding(&self, query: &str, embedding: Vec<f32>) {
        let mut cache = self.query_embeddings.lock().unwrap();
        cache.insert(query.to_string(), embedding);
    }

    // ========== Semantic Results Cache ==========

    /// Generate cache key for semantic search
    pub fn semantic_cache_key(embedding: &[f32], k: usize) -> u64 {
        // Hash the embedding and k parameter
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        // Hash first/last few floats and length for speed
        embedding.len().hash(&mut hasher);
        k.hash(&mut hasher);
        if !embedding.is_empty() {
            // Use bits representation for deterministic hashing
            embedding[0].to_bits().hash(&mut hasher);
            embedding[embedding.len() / 2].to_bits().hash(&mut hasher);
            embedding[embedding.len() - 1].to_bits().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Get cached semantic search results
    pub fn get_semantic_results(&self, embedding: &[f32], k: usize) -> Option<Vec<(String, f32)>> {
        let key = Self::semantic_cache_key(embedding, k);
        let mut cache = self.semantic_results.lock().unwrap();
        if let Some(results) = cache.get(&key) {
            *self.hits.lock().unwrap() += 1;
            Some(results)
        } else {
            *self.misses.lock().unwrap() += 1;
            None
        }
    }

    /// Cache semantic search results
    pub fn put_semantic_results(&self, embedding: &[f32], k: usize, results: Vec<(String, f32)>) {
        let key = Self::semantic_cache_key(embedding, k);
        // Extract entry IDs for invalidation tracking
        let depends_on: Vec<String> = results.iter().map(|(id, _)| id.clone()).collect();
        let mut cache = self.semantic_results.lock().unwrap();
        cache.insert_with_deps(key, results, depends_on);
    }

    // ========== Hybrid Results Cache ==========

    /// Get cached hybrid search results
    pub fn get_hybrid_results(&self, options_hash: u64) -> Option<Vec<HybridSearchResult>> {
        let mut cache = self.hybrid_results.lock().unwrap();
        if let Some(results) = cache.get(&options_hash) {
            *self.hits.lock().unwrap() += 1;
            Some(results)
        } else {
            *self.misses.lock().unwrap() += 1;
            None
        }
    }

    /// Cache hybrid search results
    pub fn put_hybrid_results(&self, options_hash: u64, results: Vec<HybridSearchResult>) {
        // Extract entry IDs for invalidation tracking
        let depends_on: Vec<String> = results.iter().map(|r| r.id.clone()).collect();
        let mut cache = self.hybrid_results.lock().unwrap();
        cache.insert_with_deps(options_hash, results, depends_on);
    }

    // ========== Invalidation ==========

    /// Invalidate all cached results that depend on the given entry ID
    ///
    /// Should be called when an entry is:
    /// - Created/updated (index_entry)
    /// - Deleted
    /// - Archived
    pub fn invalidate_entry(&self, entry_id: &str) {
        // Semantic results cache
        {
            let mut cache = self.semantic_results.lock().unwrap();
            cache.invalidate_by_dependency(entry_id);
        }

        // Hybrid results cache
        {
            let mut cache = self.hybrid_results.lock().unwrap();
            cache.invalidate_by_dependency(entry_id);
        }

        // Note: Query embeddings are NOT invalidated because they
        // only depend on the query text, not on entry content
    }

    /// Clear all caches
    pub fn clear(&self) {
        self.query_embeddings.lock().unwrap().clear();
        self.semantic_results.lock().unwrap().clear();
        self.hybrid_results.lock().unwrap().clear();
        *self.hits.lock().unwrap() = 0;
        *self.misses.lock().unwrap() = 0;
    }

    /// Get cache statistics
    pub fn stats(&self) -> SearchCacheStats {
        let embedding_stats = self.query_embeddings.lock().unwrap().stats();
        let semantic_stats = self.semantic_results.lock().unwrap().stats();
        let hybrid_stats = self.hybrid_results.lock().unwrap().stats();
        let hits = *self.hits.lock().unwrap();
        let misses = *self.misses.lock().unwrap();

        SearchCacheStats {
            embedding_cache: embedding_stats,
            semantic_cache: semantic_stats,
            hybrid_cache: hybrid_stats,
            total_hits: hits,
            total_misses: misses,
            hit_rate: if hits + misses > 0 {
                hits as f64 / (hits + misses) as f64
            } else {
                0.0
            },
        }
    }

    /// Evict expired entries from all caches
    pub fn evict_expired(&self) {
        self.query_embeddings.lock().unwrap().evict_expired();
        self.semantic_results.lock().unwrap().evict_expired();
        self.hybrid_results.lock().unwrap().evict_expired();
    }
}

/// Statistics for the search cache
#[derive(Debug, Clone)]
pub struct SearchCacheStats {
    pub embedding_cache: CacheStats,
    pub semantic_cache: CacheStats,
    pub hybrid_cache: CacheStats,
    pub total_hits: u64,
    pub total_misses: u64,
    pub hit_rate: f64,
}

#[cfg(test)]
#[path = "cache_tests/tests.rs"]
mod tests;
