//! Integration tests for Code Search
//!
//! Tests cover:
//! - Symbol search by name
//! - Semantic search returns similar code
//! - Language/kind filters work
//! - No N+1 queries (batch fetching verification)
//!
//! Note: Tests use MockVectorStore since MockVectorStore has been removed.
//! Semantic search is now available via cloud API (premium feature).

use cas_code::{
    CodeFile, CodeMemoryLink, CodeMemoryLinkType, CodeRelationship, CodeSymbol, Language,
    SymbolKind,
};
use cas_search::{
    Bm25Index, CodeSearch, CodeSearchOptions, CodeSearchResult, Embedder as CodeEmbedder,
    SearchDocument, TextIndex, VectorStore,
};
use cas_store::CodeStore;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

// =============================================================================
// Mock implementations
// =============================================================================

/// Mock VectorStore for testing (semantic search is now cloud-only)
struct MockVectorStore {
    vectors: RwLock<HashMap<String, Vec<f32>>>,
    dimension: usize,
}

impl MockVectorStore {
    fn new(dimension: usize) -> Self {
        Self {
            vectors: RwLock::new(HashMap::new()),
            dimension,
        }
    }
}

impl VectorStore for MockVectorStore {
    fn store(&self, doc_id: &str, embedding: &[f32]) -> cas_search::Result<()> {
        if embedding.len() != self.dimension {
            return Err(cas_search::SearchError::storage("dimension mismatch"));
        }
        self.vectors
            .write()
            .unwrap()
            .insert(doc_id.to_string(), embedding.to_vec());
        Ok(())
    }

    fn get(&self, doc_id: &str) -> cas_search::Result<Option<Vec<f32>>> {
        Ok(self.vectors.read().unwrap().get(doc_id).cloned())
    }

    fn delete(&self, doc_id: &str) -> cas_search::Result<()> {
        self.vectors.write().unwrap().remove(doc_id);
        Ok(())
    }

    fn search(&self, _query: &[f32], k: usize) -> cas_search::Result<Vec<(String, f32)>> {
        // Simple mock: return first k vectors with dummy scores
        let vectors = self.vectors.read().unwrap();
        Ok(vectors
            .keys()
            .take(k)
            .map(|id| (id.clone(), 0.5f32))
            .collect())
    }

    fn exists(&self, doc_id: &str) -> cas_search::Result<bool> {
        Ok(self.vectors.read().unwrap().contains_key(doc_id))
    }

    fn count(&self) -> cas_search::Result<usize> {
        Ok(self.vectors.read().unwrap().len())
    }

    fn list_ids(&self) -> cas_search::Result<Vec<String>> {
        Ok(self.vectors.read().unwrap().keys().cloned().collect())
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Mock CodeStore that tracks batch vs single symbol fetches
struct MockCodeStore {
    symbols: std::sync::RwLock<HashMap<String, CodeSymbol>>,
    single_fetch_count: AtomicUsize,
    batch_fetch_count: AtomicUsize,
}

impl MockCodeStore {
    fn new() -> Self {
        Self {
            symbols: std::sync::RwLock::new(HashMap::new()),
            single_fetch_count: AtomicUsize::new(0),
            batch_fetch_count: AtomicUsize::new(0),
        }
    }

    fn insert_symbol(&self, symbol: CodeSymbol) {
        self.symbols
            .write()
            .unwrap()
            .insert(symbol.id.clone(), symbol);
    }

    fn single_fetches(&self) -> usize {
        self.single_fetch_count.load(Ordering::SeqCst)
    }

    fn batch_fetches(&self) -> usize {
        self.batch_fetch_count.load(Ordering::SeqCst)
    }

    fn reset_counters(&self) {
        self.single_fetch_count.store(0, Ordering::SeqCst);
        self.batch_fetch_count.store(0, Ordering::SeqCst);
    }
}

impl CodeStore for MockCodeStore {
    fn init(&self) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn generate_file_id(&self) -> cas_store::error::Result<String> {
        Ok("f-1".to_string())
    }

    fn generate_file_id_for(&self, _path: &str, _project: &str) -> String {
        "f-1".to_string()
    }

    fn add_file(&self, _file: &CodeFile) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn get_file(&self, _id: &str) -> cas_store::error::Result<CodeFile> {
        Err(cas_store::error::StoreError::NotFound(
            "file not found".to_string(),
        ))
    }

    fn get_file_by_path(
        &self,
        _path: &str,
        _project: &str,
    ) -> cas_store::error::Result<Option<CodeFile>> {
        Ok(None)
    }

    fn list_files(
        &self,
        _project: &str,
        _language: Option<Language>,
    ) -> cas_store::error::Result<Vec<CodeFile>> {
        Ok(vec![])
    }

    fn delete_file(&self, _id: &str) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn generate_symbol_id(&self) -> cas_store::error::Result<String> {
        Ok("s-1".to_string())
    }

    fn generate_symbol_id_for(&self, _file_id: &str, _name: &str, _kind: &str) -> String {
        "s-1".to_string()
    }

    fn add_symbol(&self, _symbol: &CodeSymbol) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn get_symbol(&self, id: &str) -> cas_store::error::Result<CodeSymbol> {
        self.single_fetch_count.fetch_add(1, Ordering::SeqCst);
        self.symbols
            .read()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| cas_store::error::StoreError::NotFound(id.to_string()))
    }

    fn get_symbols_by_name(&self, name: &str) -> cas_store::error::Result<Vec<CodeSymbol>> {
        Ok(self
            .symbols
            .read()
            .unwrap()
            .values()
            .filter(|s| s.qualified_name == name || s.name == name)
            .cloned()
            .collect())
    }

    fn get_symbols_in_file(&self, file_id: &str) -> cas_store::error::Result<Vec<CodeSymbol>> {
        Ok(self
            .symbols
            .read()
            .unwrap()
            .values()
            .filter(|s| s.file_id == file_id)
            .cloned()
            .collect())
    }

    fn search_symbols(
        &self,
        query: &str,
        kind: Option<SymbolKind>,
        language: Option<Language>,
        limit: usize,
    ) -> cas_store::error::Result<Vec<CodeSymbol>> {
        let query_lower = query.to_lowercase();
        Ok(self
            .symbols
            .read()
            .unwrap()
            .values()
            .filter(|s| {
                let name_match = s.qualified_name.to_lowercase().contains(&query_lower);
                let kind_match = kind.is_none_or(|k| s.kind == k);
                let lang_match = language.is_none_or(|l| s.language == l);
                name_match && kind_match && lang_match
            })
            .take(limit)
            .cloned()
            .collect())
    }

    fn search_symbols_paginated(
        &self,
        query: &str,
        kind: Option<SymbolKind>,
        language: Option<Language>,
        limit: usize,
        offset: usize,
    ) -> cas_store::error::Result<Vec<CodeSymbol>> {
        let query_lower = query.to_lowercase();
        Ok(self
            .symbols
            .read()
            .unwrap()
            .values()
            .filter(|s| {
                let name_match = s.qualified_name.to_lowercase().contains(&query_lower);
                let kind_match = kind.is_none_or(|k| s.kind == k);
                let lang_match = language.is_none_or(|l| s.language == l);
                name_match && kind_match && lang_match
            })
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }

    fn delete_symbol(&self, _id: &str) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn delete_symbols_in_file(&self, _file_id: &str) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn generate_relationship_id(&self) -> cas_store::error::Result<String> {
        Ok("r-1".to_string())
    }

    fn add_relationship(&self, _rel: &CodeRelationship) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn get_callers(&self, _symbol_id: &str) -> cas_store::error::Result<Vec<CodeSymbol>> {
        Ok(vec![])
    }

    fn get_callees(&self, _symbol_id: &str) -> cas_store::error::Result<Vec<CodeSymbol>> {
        Ok(vec![])
    }

    fn get_relationships_from(
        &self,
        _symbol_id: &str,
    ) -> cas_store::error::Result<Vec<CodeRelationship>> {
        Ok(vec![])
    }

    fn get_relationships_to(
        &self,
        _symbol_id: &str,
    ) -> cas_store::error::Result<Vec<CodeRelationship>> {
        Ok(vec![])
    }

    fn delete_relationships_for_symbol(&self, _symbol_id: &str) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn link_to_memory(&self, _link: &CodeMemoryLink) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn get_linked_memories(&self, _symbol_id: &str) -> cas_store::error::Result<Vec<String>> {
        Ok(vec![])
    }

    fn get_linked_code(&self, _memory_id: &str) -> cas_store::error::Result<Vec<String>> {
        Ok(vec![])
    }

    fn get_memory_links(&self, _symbol_id: &str) -> cas_store::error::Result<Vec<CodeMemoryLink>> {
        Ok(vec![])
    }

    fn delete_memory_link(
        &self,
        _symbol_id: &str,
        _memory_id: &str,
        _link_type: CodeMemoryLinkType,
    ) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn delete_memory_links_for_code(&self, _symbol_id: &str) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn add_symbols_batch(&self, symbols: &[CodeSymbol]) -> cas_store::error::Result<()> {
        let mut store = self.symbols.write().unwrap();
        for symbol in symbols {
            store.insert(symbol.id.clone(), symbol.clone());
        }
        Ok(())
    }

    fn add_relationships_batch(&self, _rels: &[CodeRelationship]) -> cas_store::error::Result<()> {
        Ok(())
    }

    fn get_symbols_batch(&self, ids: &[&str]) -> cas_store::error::Result<Vec<CodeSymbol>> {
        self.batch_fetch_count.fetch_add(1, Ordering::SeqCst);
        let store = self.symbols.read().unwrap();
        Ok(ids
            .iter()
            .filter_map(|id| store.get(*id).cloned())
            .collect())
    }

    fn count_files(&self) -> cas_store::error::Result<usize> {
        Ok(1)
    }

    fn count_symbols(&self) -> cas_store::error::Result<usize> {
        Ok(self.symbols.read().unwrap().len())
    }

    fn count_files_by_language(&self) -> cas_store::error::Result<HashMap<Language, usize>> {
        Ok(HashMap::new())
    }

    fn close(&self) -> cas_store::error::Result<()> {
        Ok(())
    }
}

/// Mock embedder for testing
struct MockEmbedder {
    dimension: usize,
}

impl MockEmbedder {
    fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Generate deterministic embedding from text
    fn embed_deterministic(&self, text: &str) -> Vec<f32> {
        let mut emb = vec![0.0f32; self.dimension];
        for (i, byte) in text.bytes().enumerate() {
            emb[i % self.dimension] += (byte as f32) / 255.0;
        }

        // Normalize
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in emb.iter_mut() {
                *x /= norm;
            }
        }
        emb
    }
}

impl CodeEmbedder for MockEmbedder {
    fn embed(&self, text: &str) -> cas_search::Result<Vec<f32>> {
        Ok(self.embed_deterministic(text))
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

/// Helper to create a test symbol
fn create_symbol(id: &str, name: &str, kind: SymbolKind, language: Language) -> CodeSymbol {
    CodeSymbol {
        id: id.to_string(),
        qualified_name: format!("module::{name}"),
        name: name.to_string(),
        kind,
        language,
        file_path: format!("src/{}.rs", name.to_lowercase()),
        file_id: format!("file-{id}"),
        line_start: 1,
        line_end: 10,
        source: format!("fn {name}() {{ /* body */ }}"),
        documentation: Some(format!("Documentation for {name}")),
        ..Default::default()
    }
}

/// BM25 document wrapper for code symbols
struct SymbolDoc<'a> {
    symbol: &'a CodeSymbol,
}

impl<'a> SearchDocument for SymbolDoc<'a> {
    fn doc_id(&self) -> &str {
        &self.symbol.id
    }

    fn doc_content(&self) -> &str {
        &self.symbol.source
    }

    fn doc_type(&self) -> &str {
        "code_symbol"
    }

    fn doc_tags(&self) -> Vec<&str> {
        vec![]
    }

    fn doc_metadata(&self) -> HashMap<String, String> {
        HashMap::new()
    }

    fn doc_title(&self) -> Option<&str> {
        Some(&self.symbol.qualified_name)
    }
}

// =============================================================================
// Code Search Tests
// =============================================================================

/// Test: Symbol search by name works
#[path = "code_search_integration_cases/tests.rs"]
mod tests;
