//! Go language parser using tree-sitter.
//!
//! Supports Go source files (.go).
#![allow(
    clippy::needless_borrow,
    clippy::to_string_in_format_args,
    clippy::too_many_arguments
)]

use std::path::Path;

use chrono::Utc;
use sha2::{Digest, Sha256};
use tree_sitter::{Node, Parser, Tree};

use crate::error::{CodeError, Result};
use crate::types::{CodeSymbol, Import, Language, ParseResult, SymbolKind};

use crate::parser::LanguageParser;

/// Go language parser.
pub struct GoParser {
    parser: Parser,
}

impl GoParser {
    /// Create a new Go parser.
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .map_err(|e| CodeError::TreeSitter(format!("failed to set Go language: {e}")))?;

        Ok(Self { parser })
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
            .unwrap()
            .as_millis() as u32;

        format!("sym-{:04x}{:04x}", timestamp & 0xFFFF, count & 0xFFFF)
    }

    /// Extract text from a node.
    fn node_text<'a>(&self, node: &Node, source: &'a [u8]) -> &'a str {
        node.utf8_text(source).unwrap_or("")
    }

    /// Extract Go doc comment above a node.
    fn extract_doc_comment(&self, node: &Node, source: &[u8]) -> Option<String> {
        let mut doc_lines = Vec::new();
        let mut sibling = node.prev_sibling();

        while let Some(prev) = sibling {
            if prev.kind() == "comment" {
                let text = self.node_text(&prev, source);
                if text.starts_with("//") {
                    // Single-line comment
                    doc_lines.push(text.trim_start_matches("//").trim().to_string());
                } else if text.starts_with("/*") {
                    // Block comment
                    let cleaned = text
                        .trim_start_matches("/*")
                        .trim_end_matches("*/")
                        .lines()
                        .map(|l| l.trim().trim_start_matches('*').trim())
                        .collect::<Vec<_>>()
                        .join("\n");
                    doc_lines.push(cleaned);
                    break;
                }
                sibling = prev.prev_sibling();
            } else {
                break;
            }
        }

        if doc_lines.is_empty() {
            None
        } else {
            doc_lines.reverse();
            Some(doc_lines.join("\n"))
        }
    }

    /// Extract a function declaration.
    fn extract_function(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        package_name: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if package_name.is_empty() {
            name.clone()
        } else {
            format!("{package_name}.{name}")
        };

        // Build signature
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("()");

        let result = node
            .child_by_field_name("result")
            .map(|n| format!(" {}", self.node_text(&n, source)))
            .unwrap_or_default();

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let signature = format!("func {name}{type_params}{params}{result}");

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Function,
            language: Language::Go,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    /// Extract a method declaration.
    fn extract_method(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        package_name: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        // Get receiver type
        let receiver = node.child_by_field_name("receiver")?;
        let receiver_text = self.node_text(&receiver, source);

        // Extract the receiver type name (e.g., "(s *Server)" -> "Server")
        let receiver_type = receiver_text
            .trim_matches(|c| c == '(' || c == ')')
            .split_whitespace()
            .last()
            .unwrap_or("")
            .trim_start_matches('*');

        let qualified_name = if package_name.is_empty() {
            format!("{receiver_type}.{name}")
        } else {
            format!("{package_name}.{receiver_type}.{name}")
        };

        // Build signature
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("()");

        let result = node
            .child_by_field_name("result")
            .map(|n| format!(" {}", self.node_text(&n, source)))
            .unwrap_or_default();

        let signature = format!("func {receiver_text} {name}{params}{result}");

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Method,
            language: Language::Go,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: Some(signature),
            parent_id: None, // In Go, methods don't nest inside types
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    /// Extract a type specification (struct, interface, type alias).
    fn extract_type_spec(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        package_name: &str,
        doc: Option<String>,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if package_name.is_empty() {
            name.clone()
        } else {
            format!("{package_name}.{name}")
        };

        let type_node = node.child_by_field_name("type")?;
        let type_kind = type_node.kind();

        let (kind, signature) = match type_kind {
            "struct_type" => (SymbolKind::Struct, format!("type {name} struct")),
            "interface_type" => (SymbolKind::Interface, format!("type {name} interface")),
            _ => {
                // Type alias
                let type_text = self.node_text(&type_node, source);
                (SymbolKind::Type, format!("type {name} {type_text}"))
            }
        };

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let full_signature = if type_params.is_empty() {
            signature
        } else {
            signature.replace(&name, &format!("{name}{type_params}"))
        };

        let full_source = self.node_text(node, source).to_string();
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind,
            language: Language::Go,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: Some(full_signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    /// Extract import specifications from an import declaration node.
    fn extract_import_decl(&self, node: &Node, source: &[u8]) -> Vec<Import> {
        let mut imports = Vec::new();

        // Handle import declaration (import "path" or import (...))
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "import_spec" => {
                    if let Some(import) = self.extract_import_spec(&child, source) {
                        imports.push(import);
                    }
                }
                "import_spec_list" => {
                    let mut list_cursor = child.walk();
                    for spec in child.children(&mut list_cursor) {
                        if spec.kind() == "import_spec" {
                            if let Some(import) = self.extract_import_spec(&spec, source) {
                                imports.push(import);
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        imports
    }

    /// Extract a single import spec.
    fn extract_import_spec(&self, node: &Node, source: &[u8]) -> Option<Import> {
        let path_node = node.child_by_field_name("path")?;
        let path = self
            .node_text(&path_node, source)
            .trim_matches('"')
            .to_string();

        // Check for alias
        let alias = node
            .child_by_field_name("name")
            .map(|n| self.node_text(&n, source).to_string());

        let items = alias.map(|a| vec![a]).unwrap_or_default();

        Some(Import {
            module_path: path,
            items,
            line: node.start_position().row + 1,
            is_reexport: false,
        })
    }

    /// Extract package name from package clause.
    fn extract_package_name(&self, node: &Node, source: &[u8]) -> Option<String> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "package_identifier" {
                return Some(self.node_text(&child, source).to_string());
            }
        }
        None
    }

    /// Recursively walk the AST and extract symbols.
    fn walk_tree(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        package_name: &str,
        symbols: &mut Vec<CodeSymbol>,
        imports: &mut Vec<Import>,
    ) {
        match node.kind() {
            "function_declaration" => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    package_name,
                ) {
                    symbols.push(sym);
                }
            }
            "method_declaration" => {
                if let Some(sym) =
                    self.extract_method(node, source, file_path, file_id, repository, package_name)
                {
                    symbols.push(sym);
                }
            }
            "type_declaration" => {
                // Extract doc comment for the type declaration
                let doc = self.extract_doc_comment(node, source);

                // Process type specs within the declaration
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "type_spec" {
                        if let Some(sym) = self.extract_type_spec(
                            &child,
                            source,
                            file_path,
                            file_id,
                            repository,
                            package_name,
                            doc.clone(),
                        ) {
                            symbols.push(sym);
                        }
                    }
                }
                return; // Don't recurse
            }
            "import_declaration" => {
                imports.extend(self.extract_import_decl(node, source));
            }
            _ => {}
        }

        // Recurse into children
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk_tree(
                &child,
                source,
                file_path,
                file_id,
                repository,
                package_name,
                symbols,
                imports,
            );
        }
    }

    /// Parse source code into an AST.
    fn parse(&mut self, source: &str) -> Result<Tree> {
        self.parser
            .parse(source, None)
            .ok_or_else(|| CodeError::TreeSitter("failed to parse Go source".to_string()))
    }
}

impl LanguageParser for GoParser {
    fn parse_file(&mut self, path: &Path, content: &str, repository: &str) -> Result<ParseResult> {
        let tree = self.parse(content)?;
        let root = tree.root_node();

        let file_path = path.to_string_lossy().to_string();
        let file_id = format!("file-{}", Self::content_hash(&file_path)[..8].to_string());

        // Extract package name first
        let mut package_name = String::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if child.kind() == "package_clause" {
                if let Some(name) = self.extract_package_name(&child, content.as_bytes()) {
                    package_name = name;
                    break;
                }
            }
        }

        let mut symbols = Vec::new();
        let mut imports = Vec::new();

        self.walk_tree(
            &root,
            content.as_bytes(),
            &file_path,
            &file_id,
            repository,
            &package_name,
            &mut symbols,
            &mut imports,
        );

        // Check for parse errors
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
            if child.kind() == "import_declaration" {
                imports.extend(self.extract_import_decl(&child, content.as_bytes()));
            }
        }

        Ok(imports)
    }

    fn extensions(&self) -> &[&str] {
        &["go"]
    }

    fn language(&self) -> Language {
        Language::Go
    }
}

#[cfg(test)]
mod tests {
    use crate::parser::go::*;

    #[test]
    fn test_parse_function() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

// Add adds two integers.
func Add(a, b int) int {
    return a + b
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Add");
        assert_eq!(sym.qualified_name, "main.Add");
        assert_eq!(sym.kind, SymbolKind::Function);
        assert!(sym.signature.as_ref().unwrap().contains("func Add"));
        assert!(sym.documentation.is_some());
    }

    #[test]
    fn test_parse_method() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package http

// ServeHTTP handles HTTP requests.
func (s *Server) ServeHTTP(w ResponseWriter, r *Request) {
    // implementation
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "ServeHTTP");
        assert_eq!(sym.qualified_name, "http.Server.ServeHTTP");
        assert_eq!(sym.kind, SymbolKind::Method);
    }

    #[test]
    fn test_parse_struct() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

// Config holds configuration values.
type Config struct {
    Name    string
    Port    int
    Enabled bool
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Config");
        assert_eq!(sym.kind, SymbolKind::Struct);
        assert!(
            sym.signature
                .as_ref()
                .unwrap()
                .contains("type Config struct")
        );
    }

    #[test]
    fn test_parse_interface() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package io

// Reader is the interface for reading.
type Reader interface {
    Read(p []byte) (n int, err error)
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Reader");
        assert_eq!(sym.kind, SymbolKind::Interface);
        assert!(
            sym.signature
                .as_ref()
                .unwrap()
                .contains("type Reader interface")
        );
    }

    #[test]
    fn test_parse_type_alias() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

type StringSlice []string
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "StringSlice");
        assert_eq!(sym.kind, SymbolKind::Type);
    }

    #[test]
    fn test_parse_generic_function() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

func Map[T, U any](items []T, fn func(T) U) []U {
    result := make([]U, len(items))
    for i, item := range items {
        result[i] = fn(item)
    }
    return result
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.symbols.len(), 1);
        let sym = &result.symbols[0];
        assert_eq!(sym.name, "Map");
        assert!(sym.signature.as_ref().unwrap().contains("[T, U any]"));
    }

    #[test]
    fn test_parse_imports() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

import (
    "fmt"
    "os"
    log "github.com/sirupsen/logrus"
)
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.imports.len(), 3);
        assert!(result.imports.iter().any(|i| i.module_path == "fmt"));
        assert!(result.imports.iter().any(|i| i.module_path == "os"));
        assert!(
            result
                .imports
                .iter()
                .any(|i| i.module_path == "github.com/sirupsen/logrus")
        );
    }

    #[test]
    fn test_parse_single_import() {
        let mut parser = GoParser::new().unwrap();
        let source = r#"
package main

import "fmt"

func main() {
    fmt.Println("hello")
}
"#;
        let result = parser
            .parse_file(Path::new("test.go"), source, "test-repo")
            .unwrap();

        assert_eq!(result.imports.len(), 1);
        assert_eq!(result.imports[0].module_path, "fmt");
    }
}
