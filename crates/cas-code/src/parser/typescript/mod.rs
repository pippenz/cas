//! TypeScript/JavaScript language parser using tree-sitter.
//!
//! Supports both TypeScript (.ts, .tsx) and JavaScript (.js, .jsx, .mjs, .cjs) files.
#![allow(
    clippy::needless_borrow,
    clippy::to_string_in_format_args,
    clippy::too_many_arguments
)]

mod imports;
mod symbol_extract;
mod tests;
mod walker;

use std::path::Path;

use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser, Tree};

use crate::error::{CodeError, Result};
use crate::parser::LanguageParser;
use crate::types::{Import, Language, ParseResult};

/// TypeScript/JavaScript language parser.
pub struct TypeScriptParser {
    ts_parser: Parser,
    tsx_parser: Parser,
    /// Whether we're parsing TSX (for current file)
    is_tsx: bool,
}

impl TypeScriptParser {
    /// Create a new TypeScript parser.
    pub fn new() -> Result<Self> {
        let mut ts_parser = Parser::new();
        ts_parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .map_err(|e| {
                CodeError::TreeSitter(format!("failed to set TypeScript language: {e}"))
            })?;

        let mut tsx_parser = Parser::new();
        tsx_parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
            .map_err(|e| CodeError::TreeSitter(format!("failed to set TSX language: {e}")))?;

        Ok(Self {
            ts_parser,
            tsx_parser,
            is_tsx: false,
        })
    }

    /// Generate a content hash for a code snippet.
    fn content_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Generate a unique symbol ID.
    fn generate_id() -> String {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let count = COUNTER.fetch_add(1, Ordering::SeqCst);
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|dur| dur.as_millis() as u32)
            .unwrap_or(0);

        format!("sym-{:04x}{:04x}", timestamp & 0xFFFF, count & 0xFFFF)
    }

    /// Extract text from a node.
    fn node_text<'a>(&self, node: &Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    /// Extract JSDoc comment above a node.
    fn extract_jsdoc(&self, node: &Node, source: &[u8]) -> Option<String> {
        let mut sibling = node.prev_sibling();

        while let Some(prev) = sibling {
            match prev.kind() {
                "comment" => {
                    let text = self.node_text(&prev, source);
                    if text.starts_with("/**") {
                        return Some(
                            text.trim_start_matches("/**")
                                .trim_end_matches("*/")
                                .lines()
                                .map(|l| l.trim().trim_start_matches('*').trim())
                                .collect::<Vec<_>>()
                                .join("\n")
                                .trim()
                                .to_string(),
                        );
                    }
                    if text.starts_with("//") {
                        let mut lines = vec![text.trim_start_matches("//").trim().to_string()];
                        let mut current = prev.prev_sibling();
                        while let Some(curr) = current {
                            if curr.kind() == "comment" {
                                let t = self.node_text(&curr, source);
                                if t.starts_with("//") {
                                    lines.push(t.trim_start_matches("//").trim().to_string());
                                    current = curr.prev_sibling();
                                    continue;
                                }
                            }
                            break;
                        }
                        lines.reverse();
                        return Some(lines.join("\n"));
                    }
                    break;
                }
                "decorator" => {
                    sibling = prev.prev_sibling();
                    continue;
                }
                _ => break,
            }
        }

        None
    }

    /// Determine if this is a JavaScript file (not TypeScript).
    fn is_javascript(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "js" | "jsx" | "mjs" | "cjs"))
            .unwrap_or(false)
    }

    /// Determine if this is a TSX/JSX file.
    fn is_jsx(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "tsx" | "jsx"))
            .unwrap_or(false)
    }

    /// Parse source code into an AST.
    fn parse(&mut self, source: &str) -> Result<Tree> {
        let parser = if self.is_tsx {
            &mut self.tsx_parser
        } else {
            &mut self.ts_parser
        };

        parser.parse(source, None).ok_or_else(|| {
            CodeError::TreeSitter("failed to parse TypeScript/JavaScript source".to_string())
        })
    }
}

impl LanguageParser for TypeScriptParser {
    fn parse_file(&mut self, path: &Path, content: &str, repository: &str) -> Result<ParseResult> {
        self.is_tsx = Self::is_jsx(path);
        let is_js = Self::is_javascript(path);

        let tree = self.parse(content)?;
        let root = tree.root_node();

        let file_path = path.to_string_lossy().to_string();
        let file_id = format!("file-{}", Self::content_hash(&file_path)[..8].to_string());

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        self.walk_tree(
            &root,
            content.as_bytes(),
            &file_path,
            &file_id,
            repository,
            "",
            &mut symbols,
            &mut imports,
            is_js,
        );

        let mut errors = Vec::new();
        if root.has_error() {
            errors.push("source contains parse errors".to_string());
        }

        Ok(ParseResult {
            symbols,
            imports,
            errors,
        })
    }

    fn extract_imports(&mut self, content: &str) -> Result<Vec<Import>> {
        let tree = self.parse(content)?;
        let root = tree.root_node();

        let mut imports = Vec::new();
        let mut cursor = root.walk();

        for child in root.children(&mut cursor) {
            if child.kind() == "import_statement" {
                if let Some(import) = self.extract_import(&child, content.as_bytes()) {
                    imports.push(import);
                }
            }
        }

        Ok(imports)
    }

    fn extensions(&self) -> &[&str] {
        &["ts", "tsx", "js", "jsx", "mjs", "cjs"]
    }

    fn language(&self) -> Language {
        Language::TypeScript
    }
}
