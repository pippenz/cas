//! Language parser trait and implementations.
//!
//! This module provides a unified interface for parsing source code
//! across different programming languages using tree-sitter.

use std::path::Path;

use crate::error::Result;
use crate::types::{Import, Language, ParseResult};

mod elixir;
mod go;
mod python;
mod rust;
mod typescript;

pub use elixir::ElixirParser;
pub use go::GoParser;
pub use python::PythonParser;
pub use rust::RustParser;
pub use typescript::TypeScriptParser;

/// Trait for language-specific parsers.
///
/// Each parser implementation extracts symbols and imports from
/// source code using tree-sitter for AST parsing.
pub trait LanguageParser: Send + Sync {
    /// Parse a file and extract symbols.
    ///
    /// # Arguments
    /// * `path` - Path to the source file (for metadata)
    /// * `content` - Source code content
    /// * `repository` - Repository identifier
    ///
    /// # Returns
    /// ParseResult containing symbols, imports, and any errors
    fn parse_file(&mut self, path: &Path, content: &str, repository: &str) -> Result<ParseResult>;

    /// Extract import/dependency information only.
    ///
    /// This is a lighter-weight operation than full parsing when
    /// only import information is needed.
    fn extract_imports(&mut self, content: &str) -> Result<Vec<Import>>;

    /// Get supported file extensions for this language.
    fn extensions(&self) -> &[&str];

    /// Get the language this parser handles.
    fn language(&self) -> Language;
}

/// Multi-language parser that delegates to language-specific parsers.
pub struct MultiLanguageParser {
    rust: RustParser,
    typescript: TypeScriptParser,
    python: PythonParser,
    go: GoParser,
    elixir: ElixirParser,
}

impl MultiLanguageParser {
    /// Create a new multi-language parser.
    pub fn new() -> Result<Self> {
        Ok(Self {
            rust: RustParser::new()?,
            typescript: TypeScriptParser::new()?,
            python: PythonParser::new()?,
            go: GoParser::new()?,
            elixir: ElixirParser::new()?,
        })
    }

    /// Get the appropriate parser for a language.
    pub fn parser_for(&mut self, language: Language) -> Option<&mut dyn LanguageParser> {
        match language {
            Language::Rust => Some(&mut self.rust),
            Language::TypeScript | Language::JavaScript => Some(&mut self.typescript),
            Language::Python => Some(&mut self.python),
            Language::Go => Some(&mut self.go),
            Language::Elixir => Some(&mut self.elixir),
            _ => None,
        }
    }

    /// Parse a file, auto-detecting the language from the extension.
    pub fn parse_file(
        &mut self,
        path: &Path,
        content: &str,
        repository: &str,
    ) -> Result<ParseResult> {
        let language = path
            .extension()
            .and_then(|e| e.to_str())
            .map(Language::from_extension)
            .unwrap_or(Language::Unknown);

        if let Some(parser) = self.parser_for(language) {
            parser.parse_file(path, content, repository)
        } else {
            // Return empty result for unsupported languages
            Ok(ParseResult::default())
        }
    }

    /// Check if a language is supported.
    pub fn supports(&self, language: Language) -> bool {
        matches!(
            language,
            Language::Rust
                | Language::TypeScript
                | Language::JavaScript
                | Language::Python
                | Language::Go
                | Language::Elixir
        )
    }

    /// Get all supported languages.
    pub fn supported_languages(&self) -> Vec<Language> {
        vec![
            Language::Rust,
            Language::TypeScript,
            Language::JavaScript,
            Language::Python,
            Language::Go,
            Language::Elixir,
        ]
    }
}

impl Default for MultiLanguageParser {
    fn default() -> Self {
        Self::new().expect("failed to create parser")
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::*;

    #[test]
    fn test_multi_parser_supports_languages() {
        let parser = MultiLanguageParser::new().unwrap();
        assert!(parser.supports(Language::Rust));
        assert!(parser.supports(Language::TypeScript));
        assert!(parser.supports(Language::JavaScript));
        assert!(parser.supports(Language::Python));
        assert!(parser.supports(Language::Go));
        assert!(parser.supports(Language::Elixir));
    }

    #[test]
    fn test_multi_parser_supported_languages() {
        let parser = MultiLanguageParser::new().unwrap();
        let langs = parser.supported_languages();
        assert!(langs.contains(&Language::Rust));
        assert!(langs.contains(&Language::TypeScript));
        assert!(langs.contains(&Language::JavaScript));
        assert!(langs.contains(&Language::Python));
        assert!(langs.contains(&Language::Go));
        assert!(langs.contains(&Language::Elixir));
    }
}
