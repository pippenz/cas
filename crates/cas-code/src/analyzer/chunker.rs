//! Code chunking for embedding generation.
//!
//! This module breaks code symbols into chunks optimized for
//! semantic search via embeddings.

use crate::types::{CodeSymbol, SymbolKind};

/// Type of chunk representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkType {
    /// Full source code of the symbol
    FullSource,
    /// Just the signature (for functions/methods)
    Signature,
    /// Documentation + signature
    DocSignature,
    /// Summary: name + kind + doc (first line)
    Summary,
}

/// Configuration for chunking behavior.
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// Maximum chunk size in characters
    pub max_chunk_size: usize,
    /// Whether to include documentation
    pub include_docs: bool,
    /// Whether to include full source for small symbols
    pub include_source: bool,
    /// Chunk types to generate per symbol
    pub chunk_types: Vec<ChunkType>,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chunk_size: 2000,
            include_docs: true,
            include_source: true,
            chunk_types: vec![ChunkType::FullSource],
        }
    }
}

/// A chunk of code ready for embedding.
#[derive(Debug, Clone)]
pub struct CodeChunk {
    /// The symbol this chunk belongs to
    pub symbol_id: String,
    /// Type of this chunk
    pub chunk_type: ChunkType,
    /// Text content to embed
    pub content: String,
    /// Relative importance (0.0 - 1.0)
    pub importance: f32,
}

/// Generates embeddable chunks from code symbols.
pub struct CodeChunker {
    config: ChunkConfig,
}

impl CodeChunker {
    /// Create a new chunker with default config.
    pub fn new() -> Self {
        Self {
            config: ChunkConfig::default(),
        }
    }

    /// Create a new chunker with custom config.
    pub fn with_config(config: ChunkConfig) -> Self {
        Self { config }
    }

    /// Generate chunks for a single symbol.
    pub fn chunk_symbol(&self, symbol: &CodeSymbol) -> Vec<CodeChunk> {
        let mut chunks = Vec::new();

        for chunk_type in &self.config.chunk_types {
            if let Some(chunk) = self.create_chunk(symbol, *chunk_type) {
                chunks.push(chunk);
            }
        }

        chunks
    }

    /// Generate chunks for multiple symbols.
    pub fn chunk_symbols(&self, symbols: &[CodeSymbol]) -> Vec<CodeChunk> {
        symbols.iter().flat_map(|s| self.chunk_symbol(s)).collect()
    }

    /// Create a specific chunk type for a symbol.
    fn create_chunk(&self, symbol: &CodeSymbol, chunk_type: ChunkType) -> Option<CodeChunk> {
        let content = match chunk_type {
            ChunkType::FullSource => {
                if symbol.source.len() <= self.config.max_chunk_size {
                    symbol.source.clone()
                } else {
                    // Truncate large sources at a valid UTF-8 char boundary
                    let mut end = self.config.max_chunk_size;
                    while end > 0 && !symbol.source.is_char_boundary(end) {
                        end -= 1;
                    }
                    let truncated = &symbol.source[..end];
                    format!("{truncated}...")
                }
            }
            ChunkType::Signature => {
                // Only meaningful for functions/methods
                match symbol.kind {
                    SymbolKind::Function | SymbolKind::Method => symbol
                        .signature
                        .clone()
                        .unwrap_or_else(|| symbol.name.clone()),
                    SymbolKind::Struct | SymbolKind::Enum | SymbolKind::Trait => symbol
                        .signature
                        .clone()
                        .unwrap_or_else(|| format!("{} {}", symbol.kind, symbol.name)),
                    _ => return None,
                }
            }
            ChunkType::DocSignature => {
                let mut parts = Vec::new();

                // Add documentation if present
                if let Some(doc) = &symbol.documentation {
                    parts.push(doc.clone());
                }

                // Add signature or name
                if let Some(sig) = &symbol.signature {
                    parts.push(sig.clone());
                } else {
                    parts.push(format!("{} {}", symbol.kind, symbol.name));
                }

                parts.join("\n\n")
            }
            ChunkType::Summary => {
                let doc_line = symbol
                    .documentation
                    .as_ref()
                    .and_then(|d| d.lines().next())
                    .unwrap_or("");

                format!(
                    "{} {} in {}: {}",
                    symbol.kind, symbol.qualified_name, symbol.file_path, doc_line
                )
            }
        };

        if content.is_empty() {
            return None;
        }

        let importance = self.calculate_importance(symbol, chunk_type);

        Some(CodeChunk {
            symbol_id: symbol.id.clone(),
            chunk_type,
            content,
            importance,
        })
    }

    /// Calculate importance score for a chunk.
    fn calculate_importance(&self, symbol: &CodeSymbol, chunk_type: ChunkType) -> f32 {
        let base_importance: f32 = match symbol.kind {
            // Public API items are most important
            SymbolKind::Trait | SymbolKind::Interface => 1.0,
            SymbolKind::Struct | SymbolKind::Class | SymbolKind::Enum => 0.9,
            SymbolKind::Function => 0.8,
            SymbolKind::Macro => 0.75, // Elixir/Rust macros are important
            SymbolKind::Method => 0.7,
            SymbolKind::Impl => 0.6,
            SymbolKind::Module => 0.5,
            SymbolKind::Type => 0.5,
            SymbolKind::Constant => 0.4,
            SymbolKind::Variable => 0.3,
            SymbolKind::Import => 0.2,
        };

        // Adjust based on chunk type
        let type_modifier: f32 = match chunk_type {
            ChunkType::DocSignature => 1.0, // Best for search
            ChunkType::Signature => 0.9,
            ChunkType::Summary => 0.8,
            ChunkType::FullSource => 0.7, // Lower priority, larger
        };

        // Boost items with documentation
        let doc_boost: f32 = if symbol.documentation.is_some() {
            1.1
        } else {
            1.0
        };

        (base_importance * type_modifier * doc_boost).min(1.0)
    }
}

impl Default for CodeChunker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use crate::analyzer::chunker::*;
    use crate::types::Language;
    use chrono::Utc;

    fn make_symbol(
        name: &str,
        kind: SymbolKind,
        doc: Option<&str>,
        sig: Option<&str>,
    ) -> CodeSymbol {
        let now = Utc::now();
        CodeSymbol {
            id: format!("sym-{name}"),
            qualified_name: format!("test::{name}"),
            name: name.to_string(),
            kind,
            language: Language::Rust,
            file_path: "test.rs".to_string(),
            file_id: "file-test".to_string(),
            line_start: 1,
            line_end: 10,
            source: format!("fn {name}() {{}}"),
            documentation: doc.map(String::from),
            signature: sig.map(String::from),
            parent_id: None,
            repository: "test-repo".to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: "abc123".to_string(),
            scope: "project".to_string(),
        }
    }

    #[test]
    fn test_chunk_function() {
        let chunker = CodeChunker::new();
        let symbol = make_symbol(
            "hello",
            SymbolKind::Function,
            Some("Says hello"),
            Some("fn hello(name: &str) -> String"),
        );

        let chunks = chunker.chunk_symbol(&symbol);

        // Default config: FullSource only (1 chunk per symbol for performance)
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::FullSource);
    }

    #[test]
    fn test_chunk_function_dual_mode() {
        // Test the old dual-chunk mode explicitly
        let config = ChunkConfig {
            chunk_types: vec![ChunkType::DocSignature, ChunkType::FullSource],
            ..Default::default()
        };
        let chunker = CodeChunker::with_config(config);
        let symbol = make_symbol(
            "hello",
            SymbolKind::Function,
            Some("Says hello"),
            Some("fn hello(name: &str) -> String"),
        );

        let chunks = chunker.chunk_symbol(&symbol);

        // Should have DocSignature and FullSource
        assert_eq!(chunks.len(), 2);

        let doc_sig = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::DocSignature)
            .unwrap();
        assert!(doc_sig.content.contains("Says hello"));
        assert!(doc_sig.content.contains("fn hello"));
    }

    #[test]
    fn test_chunk_without_docs() {
        // Use dual mode to test DocSignature behavior
        let config = ChunkConfig {
            chunk_types: vec![ChunkType::DocSignature, ChunkType::FullSource],
            ..Default::default()
        };
        let chunker = CodeChunker::with_config(config);
        let symbol = make_symbol(
            "process",
            SymbolKind::Function,
            None,
            Some("fn process(data: &[u8])"),
        );

        let chunks = chunker.chunk_symbol(&symbol);

        let doc_sig = chunks
            .iter()
            .find(|c| c.chunk_type == ChunkType::DocSignature)
            .unwrap();
        // Should just have signature when no docs
        assert_eq!(doc_sig.content, "fn process(data: &[u8])");
    }

    #[test]
    fn test_importance_scores() {
        let chunker = CodeChunker::new();

        let trait_sym = make_symbol("MyTrait", SymbolKind::Trait, Some("A trait"), None);
        let fn_sym = make_symbol("my_fn", SymbolKind::Function, None, None);

        let trait_chunks = chunker.chunk_symbol(&trait_sym);
        let fn_chunks = chunker.chunk_symbol(&fn_sym);

        // Trait with docs should have higher importance than fn without docs
        assert!(trait_chunks[0].importance > fn_chunks[0].importance);
    }

    #[test]
    fn test_custom_config() {
        let config = ChunkConfig {
            max_chunk_size: 100,
            include_docs: true,
            include_source: false,
            chunk_types: vec![ChunkType::Summary],
        };

        let chunker = CodeChunker::with_config(config);
        let symbol = make_symbol("test", SymbolKind::Function, Some("Test function"), None);

        let chunks = chunker.chunk_symbol(&symbol);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_type, ChunkType::Summary);
        assert!(chunks[0].content.contains("test::test"));
    }

    #[test]
    fn test_truncate_large_source() {
        let config = ChunkConfig {
            max_chunk_size: 20,
            include_docs: false,
            include_source: true,
            chunk_types: vec![ChunkType::FullSource],
        };

        let chunker = CodeChunker::with_config(config);
        let mut symbol = make_symbol("big", SymbolKind::Function, None, None);
        symbol.source = "fn big() { let x = 1; let y = 2; let z = 3; }".to_string();

        let chunks = chunker.chunk_symbol(&symbol);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.ends_with("..."));
        assert!(chunks[0].content.len() <= 24); // 20 + "..."
    }
}
