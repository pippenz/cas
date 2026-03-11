//! Python language parser using tree-sitter.
//!
//! Supports Python source files (.py, .pyi).
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

/// Python language parser.
pub struct PythonParser {
    parser: Parser,
}

impl PythonParser {
    /// Create a new Python parser.
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| CodeError::TreeSitter(format!("failed to set Python language: {e}")))?;

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

    /// Extract docstring from a function or class body.
    fn extract_docstring(&self, node: &Node, source: &[u8]) -> Option<String> {
        // Look for the body and check if first statement is a string
        let body = node.child_by_field_name("body")?;

        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            match child.kind() {
                "expression_statement" => {
                    // Check if expression is a string
                    let mut expr_cursor = child.walk();
                    for expr_child in child.children(&mut expr_cursor) {
                        if expr_child.kind() == "string" {
                            let text = self.node_text(&expr_child, source);
                            // Remove string delimiters
                            let doc = text
                                .trim_start_matches("\"\"\"")
                                .trim_start_matches("'''")
                                .trim_start_matches('"')
                                .trim_start_matches('\'')
                                .trim_end_matches("\"\"\"")
                                .trim_end_matches("'''")
                                .trim_end_matches('"')
                                .trim_end_matches('\'')
                                .trim();
                            return Some(doc.to_string());
                        }
                    }
                    break;
                }
                // Skip comments but stop at other statements
                "comment" => continue,
                _ => break,
            }
        }

        None
    }

    /// Extract decorators from a decorated definition.
    fn extract_decorators(&self, node: &Node, source: &[u8]) -> Vec<String> {
        let mut decorators = Vec::new();
        let mut cursor = node.walk();

        for child in node.children(&mut cursor) {
            if child.kind() == "decorator" {
                decorators.push(self.node_text(&child, source).to_string());
            }
        }

        decorators
    }

    /// Extract a function definition.
    fn extract_function(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        parent_id: Option<&str>,
        qualified_prefix: &str,
        decorators: &[String],
        is_async: bool,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        // Build signature
        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("()");

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| format!(" -> {}", self.node_text(&n, source)))
            .unwrap_or_default();

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let async_prefix = if is_async { "async " } else { "" };
        let decorator_str = if decorators.is_empty() {
            String::new()
        } else {
            format!("{}\n", decorators.join("\n"))
        };

        let signature =
            format!("{decorator_str}{async_prefix}def {name}{type_params}{params}{return_type}");

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_docstring(node, source);
        let now = Utc::now();

        // Determine if this is a method or function
        let kind = if parent_id.is_some() {
            // Check if it's a static/class method
            if decorators.iter().any(|d| d.contains("staticmethod")) {
                SymbolKind::Function
            } else {
                SymbolKind::Method
            }
        } else {
            SymbolKind::Function
        };

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind,
            language: Language::Python,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: Some(signature),
            parent_id: parent_id.map(String::from),
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    /// Extract a class definition.
    fn extract_class(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        decorators: &[String],
    ) -> (Option<CodeSymbol>, Vec<CodeSymbol>) {
        let mut methods = Vec::new();

        let name_node = match node.child_by_field_name("name") {
            Some(n) => n,
            None => return (None, methods),
        };
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        let class_id = Self::generate_id();
        let doc = self.extract_docstring(node, source);
        let now = Utc::now();

        // Build signature
        let superclasses = node
            .child_by_field_name("superclasses")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let decorator_str = if decorators.is_empty() {
            String::new()
        } else {
            format!("{}\n", decorators.join("\n"))
        };

        let signature = format!("{decorator_str}class {name}{type_params}{superclasses}");

        let class_symbol = Some(CodeSymbol {
            id: class_id.clone(),
            qualified_name: qualified_name.clone(),
            name: name.clone(),
            kind: SymbolKind::Class,
            language: Language::Python,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: self.node_text(node, source).to_string(),
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        });

        // Extract methods from class body
        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "function_definition" => {
                        let is_async = self.is_async_function(&child, source);
                        if let Some(method) = self.extract_function(
                            &child,
                            source,
                            file_path,
                            file_id,
                            repository,
                            Some(&class_id),
                            &qualified_name,
                            &[],
                            is_async,
                        ) {
                            methods.push(method);
                        }
                    }
                    "decorated_definition" => {
                        let decorators = self.extract_decorators(&child, source);
                        if let Some(def) = child.child_by_field_name("definition") {
                            if def.kind() == "function_definition" {
                                let is_async = self.is_async_function(&def, source);
                                if let Some(method) = self.extract_function(
                                    &def,
                                    source,
                                    file_path,
                                    file_id,
                                    repository,
                                    Some(&class_id),
                                    &qualified_name,
                                    &decorators,
                                    is_async,
                                ) {
                                    methods.push(method);
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        (class_symbol, methods)
    }

    /// Extract import statements.
    fn extract_import(&self, node: &Node, source: &[u8]) -> Vec<Import> {
        let mut imports = Vec::new();

        match node.kind() {
            "import_statement" => {
                // import foo, bar, baz
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
                        let module_path = if child.kind() == "aliased_import" {
                            child
                                .child_by_field_name("name")
                                .map(|n| self.node_text(&n, source))
                                .unwrap_or("")
                        } else {
                            self.node_text(&child, source)
                        };

                        imports.push(Import {
                            module_path: module_path.to_string(),
                            items: Vec::new(),
                            line: node.start_position().row + 1,
                            is_reexport: false,
                        });
                    }
                }
            }
            "import_from_statement" => {
                // from foo import bar, baz
                let module_name = node
                    .child_by_field_name("module_name")
                    .map(|n| self.node_text(&n, source).to_string())
                    .unwrap_or_else(|| ".".to_string()); // relative import

                let mut items = Vec::new();
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    match child.kind() {
                        "dotted_name" | "aliased_import" => {
                            let name = if child.kind() == "aliased_import" {
                                child
                                    .child_by_field_name("name")
                                    .map(|n| self.node_text(&n, source))
                                    .unwrap_or("")
                            } else {
                                self.node_text(&child, source)
                            };
                            // Skip the module name itself
                            if name != module_name {
                                items.push(name.to_string());
                            }
                        }
                        "wildcard_import" => {
                            items.push("*".to_string());
                        }
                        _ => {}
                    }
                }

                imports.push(Import {
                    module_path: module_name,
                    items,
                    line: node.start_position().row + 1,
                    is_reexport: false,
                });
            }
            _ => {}
        }

        imports
    }

    /// Check if a function definition is async by looking for "async" keyword.
    fn is_async_function(&self, node: &Node, source: &[u8]) -> bool {
        // In tree-sitter-python 0.25, async is the first optional child of function_definition
        node.child(0)
            .map(|c| self.node_text(&c, source) == "async")
            .unwrap_or(false)
    }

    /// Recursively walk the AST and extract symbols.
    fn walk_tree(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        symbols: &mut Vec<CodeSymbol>,
        imports: &mut Vec<Import>,
    ) {
        match node.kind() {
            "function_definition" => {
                let is_async = self.is_async_function(node, source);
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    None,
                    qualified_prefix,
                    &[],
                    is_async,
                ) {
                    symbols.push(sym);
                }
            }
            // Keep this for backwards compatibility with older tree-sitter-python versions
            "async_function_definition" => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    None,
                    qualified_prefix,
                    &[],
                    true,
                ) {
                    symbols.push(sym);
                }
            }
            "class_definition" => {
                let (class_sym, methods) = self.extract_class(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                    &[],
                );
                if let Some(sym) = class_sym {
                    symbols.push(sym);
                }
                symbols.extend(methods);
                return; // Don't recurse - methods already extracted
            }
            "decorated_definition" => {
                let decorators = self.extract_decorators(node, source);
                if let Some(def) = node.child_by_field_name("definition") {
                    match def.kind() {
                        "function_definition" => {
                            let is_async = self.is_async_function(&def, source);
                            if let Some(sym) = self.extract_function(
                                &def,
                                source,
                                file_path,
                                file_id,
                                repository,
                                None,
                                qualified_prefix,
                                &decorators,
                                is_async,
                            ) {
                                symbols.push(sym);
                            }
                        }
                        "class_definition" => {
                            let (class_sym, methods) = self.extract_class(
                                &def,
                                source,
                                file_path,
                                file_id,
                                repository,
                                qualified_prefix,
                                &decorators,
                            );
                            if let Some(sym) = class_sym {
                                symbols.push(sym);
                            }
                            symbols.extend(methods);
                        }
                        _ => {}
                    }
                }
                return; // Don't recurse
            }
            "import_statement" | "import_from_statement" => {
                imports.extend(self.extract_import(node, source));
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
                qualified_prefix,
                symbols,
                imports,
            );
        }
    }

    /// Parse source code into an AST.
    fn parse(&mut self, source: &str) -> Result<Tree> {
        self.parser
            .parse(source, None)
            .ok_or_else(|| CodeError::TreeSitter("failed to parse Python source".to_string()))
    }
}

impl LanguageParser for PythonParser {
    fn parse_file(&mut self, path: &Path, content: &str, repository: &str) -> Result<ParseResult> {
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
            if child.kind() == "import_statement" || child.kind() == "import_from_statement" {
                imports.extend(self.extract_import(&child, content.as_bytes()));
            }
        }

        Ok(imports)
    }

    fn extensions(&self) -> &[&str] {
        &["py", "pyi"]
    }

    fn language(&self) -> Language {
        Language::Python
    }
}

#[cfg(test)]
mod tests;
