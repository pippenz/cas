//! Elixir language parser using tree-sitter.
//!
//! Supports Elixir source files (.ex, .exs).
#![allow(
    clippy::manual_find,
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

/// Elixir language parser.
pub struct ElixirParser {
    parser: Parser,
}

impl ElixirParser {
    /// Create a new Elixir parser.
    pub fn new() -> Result<Self> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_elixir::LANGUAGE.into())
            .map_err(|e| CodeError::TreeSitter(format!("failed to set Elixir language: {e}")))?;

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

    /// Extract @doc or @moduledoc comment above a node.
    fn extract_doc_attribute(&self, node: &Node, source: &[u8]) -> Option<String> {
        // In Elixir, doc comments are module attributes like @doc or @moduledoc
        // They appear as sibling nodes before the function/module definition
        let mut sibling = node.prev_sibling();

        while let Some(prev) = sibling {
            let text = self.node_text(&prev, source);

            // Check for @doc or @moduledoc
            if text.starts_with("@doc") || text.starts_with("@moduledoc") {
                // Extract the string content from the attribute
                // Format is typically: @doc "some doc" or @doc """multi-line"""
                let doc_content = self.extract_string_from_attribute(&prev, source);
                if doc_content.is_some() {
                    return doc_content;
                }
            }

            // Skip other attributes, but stop at real code
            if !self.is_attribute(&prev, source) && prev.kind() != "comment" {
                break;
            }

            sibling = prev.prev_sibling();
        }

        None
    }

    /// Check if a node is a module attribute.
    fn is_attribute(&self, node: &Node, source: &[u8]) -> bool {
        let text = self.node_text(node, source);
        text.starts_with('@')
    }

    /// Extract string content from a @doc/@moduledoc attribute.
    fn extract_string_from_attribute(&self, node: &Node, source: &[u8]) -> Option<String> {
        let text = self.node_text(node, source);

        // Handle heredoc strings (""" or ''')
        if let Some(start) = text.find("\"\"\"") {
            if let Some(end) = text[start + 3..].find("\"\"\"") {
                let content = &text[start + 3..start + 3 + end];
                return Some(content.trim().to_string());
            }
        }

        // Handle regular strings
        if let Some(start) = text.find('"') {
            let rest = &text[start + 1..];
            if let Some(end) = rest.rfind('"') {
                let content = &rest[..end];
                return Some(content.to_string());
            }
        }

        None
    }

    /// Extract a function definition (def/defp).
    fn extract_function(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        module_name: &str,
        is_private: bool,
    ) -> Option<CodeSymbol> {
        // In Elixir tree-sitter, function definitions are call nodes
        // with the target being "def" or "defp"
        let text = self.node_text(node, source);

        // Parse the function name and arguments
        let (name, params) = self.extract_function_signature(node, source)?;

        let qualified_name = if module_name.is_empty() {
            name.clone()
        } else {
            format!("{module_name}.{name}")
        };

        let visibility = if is_private { "defp" } else { "def" };
        let signature = format!("{visibility} {name}{params}");

        let doc = self.extract_doc_attribute(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Function,
            language: Language::Elixir,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: text.to_string(),
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(text),
            scope: "project".to_string(),
        })
    }

    /// Get child node by kind name.
    fn child_by_kind<'a>(&self, node: &'a Node, kind: &str) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == kind {
                return Some(child);
            }
        }
        None
    }

    /// Extract function name and parameters from a call node.
    fn extract_function_signature(&self, node: &Node, source: &[u8]) -> Option<(String, String)> {
        // Structure: call -> [identifier(def), arguments([call or identifier]), do_block]
        // The arguments node contains the function signature

        let arguments = self.child_by_kind(node, "arguments")?;
        let mut cursor = arguments.walk();

        for child in arguments.children(&mut cursor) {
            match child.kind() {
                "call" => {
                    // def foo(a, b) - function with args
                    let target = child.child_by_field_name("target")?;
                    let name = self.node_text(&target, source).to_string();

                    let args = self
                        .child_by_kind(&child, "arguments")
                        .map(|n| self.node_text(&n, source))
                        .unwrap_or("()");

                    return Some((name, args.to_string()));
                }
                "identifier" => {
                    // def foo - function without args
                    let name = self.node_text(&child, source).to_string();
                    return Some((name, "()".to_string()));
                }
                "binary_operator" => {
                    // def foo(x) when guard - function with guard clause
                    let left = child.child_by_field_name("left")?;
                    if left.kind() == "call" {
                        let target = left.child_by_field_name("target")?;
                        let name = self.node_text(&target, source).to_string();
                        let args = self
                            .child_by_kind(&left, "arguments")
                            .map(|n| self.node_text(&n, source))
                            .unwrap_or("()");
                        return Some((name, args.to_string()));
                    } else if left.kind() == "identifier" {
                        let name = self.node_text(&left, source).to_string();
                        return Some((name, "()".to_string()));
                    }
                }
                _ => {}
            }
        }

        None
    }

    /// Extract a module definition.
    fn extract_module(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
    ) -> Option<(CodeSymbol, String)> {
        // defmodule ModuleName do ... end
        // Structure: call -> [identifier(defmodule), arguments([alias]), do_block]
        let arguments = self.child_by_kind(node, "arguments")?;
        let mut cursor = arguments.walk();

        let mut module_name = String::new();

        for child in arguments.children(&mut cursor) {
            match child.kind() {
                "alias" => {
                    module_name = self.node_text(&child, source).to_string();
                    break;
                }
                "dot" => {
                    // Nested module like Foo.Bar.Baz
                    module_name = self.node_text(&child, source).to_string();
                    break;
                }
                _ => {}
            }
        }

        if module_name.is_empty() {
            return None;
        }

        let text = self.node_text(node, source);
        let signature = format!("defmodule {module_name}");
        let doc = self.extract_doc_attribute(node, source);
        let now = Utc::now();

        let symbol = CodeSymbol {
            id: Self::generate_id(),
            qualified_name: module_name.clone(),
            name: module_name.clone(),
            kind: SymbolKind::Module,
            language: Language::Elixir,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: text.to_string(),
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(text),
            scope: "project".to_string(),
        };

        Some((symbol, module_name))
    }

    /// Extract a struct definition.
    fn extract_struct(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        module_name: &str,
    ) -> Option<CodeSymbol> {
        let text = self.node_text(node, source);
        let signature = format!("{module_name}.defstruct");
        let now = Utc::now();

        // For structs, the name is the module name
        let name = module_name.rsplit('.').next().unwrap_or(module_name);

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name: format!("{module_name}.__struct__"),
            name: name.to_string(),
            kind: SymbolKind::Struct,
            language: Language::Elixir,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: text.to_string(),
            documentation: None,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(text),
            scope: "project".to_string(),
        })
    }

    /// Extract a protocol definition.
    fn extract_protocol(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
    ) -> Option<(CodeSymbol, String)> {
        // defprotocol ProtocolName do ... end
        let arguments = self.child_by_kind(node, "arguments")?;
        let mut cursor = arguments.walk();

        let mut protocol_name = String::new();

        for child in arguments.children(&mut cursor) {
            match child.kind() {
                "alias" | "dot" => {
                    protocol_name = self.node_text(&child, source).to_string();
                    break;
                }
                _ => {}
            }
        }

        if protocol_name.is_empty() {
            return None;
        }

        let text = self.node_text(node, source);
        let signature = format!("defprotocol {protocol_name}");
        let doc = self.extract_doc_attribute(node, source);
        let now = Utc::now();

        let symbol = CodeSymbol {
            id: Self::generate_id(),
            qualified_name: protocol_name.clone(),
            name: protocol_name.clone(),
            kind: SymbolKind::Interface,
            language: Language::Elixir,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: text.to_string(),
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(text),
            scope: "project".to_string(),
        };

        Some((symbol, protocol_name))
    }

    /// Extract a macro definition (defmacro/defmacrop).
    fn extract_macro(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        module_name: &str,
        is_private: bool,
    ) -> Option<CodeSymbol> {
        let text = self.node_text(node, source);
        let (name, params) = self.extract_function_signature(node, source)?;

        let qualified_name = if module_name.is_empty() {
            name.clone()
        } else {
            format!("{module_name}.{name}")
        };

        let visibility = if is_private { "defmacrop" } else { "defmacro" };
        let signature = format!("{visibility} {name}{params}");

        let doc = self.extract_doc_attribute(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Macro,
            language: Language::Elixir,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: text.to_string(),
            documentation: doc,
            signature: Some(signature),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(text),
            scope: "project".to_string(),
        })
    }

    /// Extract import statements (alias, import, require, use).
    fn extract_import_statement(&self, node: &Node, source: &[u8], kind: &str) -> Option<Import> {
        let arguments = self.child_by_kind(node, "arguments")?;
        let mut cursor = arguments.walk();

        for child in arguments.children(&mut cursor) {
            match child.kind() {
                "alias" | "dot" => {
                    let module_path = self.node_text(&child, source).to_string();
                    return Some(Import {
                        module_path,
                        items: vec![kind.to_string()],
                        line: node.start_position().row + 1,
                        is_reexport: false,
                    });
                }
                _ => {}
            }
        }

        None
    }

    /// Check if a call node is a specific definition type.
    fn get_definition_type(&self, node: &Node, source: &[u8]) -> Option<&'static str> {
        if node.kind() != "call" {
            return None;
        }

        let target = node.child_by_field_name("target")?;
        let target_text = self.node_text(&target, source);

        match target_text {
            "def" => Some("def"),
            "defp" => Some("defp"),
            "defmodule" => Some("defmodule"),
            "defmacro" => Some("defmacro"),
            "defmacrop" => Some("defmacrop"),
            "defstruct" => Some("defstruct"),
            "defprotocol" => Some("defprotocol"),
            "defimpl" => Some("defimpl"),
            "alias" => Some("alias"),
            "import" => Some("import"),
            "require" => Some("require"),
            "use" => Some("use"),
            _ => None,
        }
    }

    /// Recursively walk the AST and extract symbols.
    fn walk_tree(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        current_module: &str,
        symbols: &mut Vec<CodeSymbol>,
        imports: &mut Vec<Import>,
    ) {
        let def_type = self.get_definition_type(node, source);

        match def_type {
            Some("defmodule") => {
                if let Some((sym, module_name)) =
                    self.extract_module(node, source, file_path, file_id, repository)
                {
                    symbols.push(sym);

                    // Process module body with new module context
                    if let Some(do_block) = self.find_do_block(node) {
                        self.walk_tree(
                            &do_block,
                            source,
                            file_path,
                            file_id,
                            repository,
                            &module_name,
                            symbols,
                            imports,
                        );
                    }
                    return; // Don't recurse normally
                }
            }
            Some("defprotocol") => {
                if let Some((sym, protocol_name)) =
                    self.extract_protocol(node, source, file_path, file_id, repository)
                {
                    symbols.push(sym);

                    // Process protocol body
                    if let Some(do_block) = self.find_do_block(node) {
                        self.walk_tree(
                            &do_block,
                            source,
                            file_path,
                            file_id,
                            repository,
                            &protocol_name,
                            symbols,
                            imports,
                        );
                    }
                    return;
                }
            }
            Some("def") => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    current_module,
                    false,
                ) {
                    symbols.push(sym);
                }
                return;
            }
            Some("defp") => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    current_module,
                    true,
                ) {
                    symbols.push(sym);
                }
                return;
            }
            Some("defmacro") => {
                if let Some(sym) = self.extract_macro(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    current_module,
                    false,
                ) {
                    symbols.push(sym);
                }
                return;
            }
            Some("defmacrop") => {
                if let Some(sym) = self.extract_macro(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    current_module,
                    true,
                ) {
                    symbols.push(sym);
                }
                return;
            }
            Some("defstruct") => {
                if !current_module.is_empty() {
                    if let Some(sym) = self.extract_struct(
                        node,
                        source,
                        file_path,
                        file_id,
                        repository,
                        current_module,
                    ) {
                        symbols.push(sym);
                    }
                }
                return;
            }
            Some(import_kind @ ("alias" | "import" | "require" | "use")) => {
                if let Some(import) = self.extract_import_statement(node, source, import_kind) {
                    imports.push(import);
                }
                return;
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
                current_module,
                symbols,
                imports,
            );
        }
    }

    /// Find the do block in a definition.
    fn find_do_block<'a>(&self, node: &'a Node) -> Option<Node<'a>> {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "do_block" {
                return Some(child);
            }
        }
        None
    }

    /// Parse source code into an AST.
    fn parse(&mut self, source: &str) -> Result<Tree> {
        self.parser
            .parse(source, None)
            .ok_or_else(|| CodeError::TreeSitter("failed to parse Elixir source".to_string()))
    }
}

impl LanguageParser for ElixirParser {
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
        let mut symbols = Vec::new(); // We need this for walk_tree but won't use it

        self.walk_tree(
            &root,
            content.as_bytes(),
            "",
            "",
            "",
            "",
            &mut symbols,
            &mut imports,
        );

        Ok(imports)
    }

    fn extensions(&self) -> &[&str] {
        &["ex", "exs"]
    }

    fn language(&self) -> Language {
        Language::Elixir
    }
}

#[cfg(test)]
mod tests;
