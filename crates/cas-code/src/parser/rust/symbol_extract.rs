use chrono::Utc;
use tree_sitter::Node;

use crate::parser::rust::RustParser;
use crate::types::{CodeSymbol, Language, SymbolKind};

impl RustParser {
    pub(crate) fn extract_function(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        parent_id: Option<&str>,
        qualified_prefix: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}::{name}")
        };

        let signature = if let Some(params) = node.child_by_field_name("parameters") {
            let return_type = node
                .child_by_field_name("return_type")
                .map(|n| self.node_text(&n, source))
                .unwrap_or("");

            let params_text = self.node_text(&params, source);
            if return_type.is_empty() {
                format!("fn {name}{params_text}")
            } else {
                format!("fn {name}{params_text} {return_type}")
            }
        } else {
            format!("fn {name}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: if parent_id.is_some() {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            },
            language: Language::Rust,
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

    pub(crate) fn extract_struct(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}::{name}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let signature = format!("struct {name}{type_params}");

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Struct,
            language: Language::Rust,
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

    pub(crate) fn extract_enum(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}::{name}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Enum,
            language: Language::Rust,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: None,
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    pub(crate) fn extract_trait(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}::{name}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_doc_comment(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Trait,
            language: Language::Rust,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: doc,
            signature: None,
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        })
    }

    pub(crate) fn extract_impl(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
    ) -> (Option<CodeSymbol>, Vec<CodeSymbol>) {
        let mut methods = Vec::new();

        let type_node = node.child_by_field_name("type");
        let type_name = type_node
            .map(|n| self.node_text(&n, source).to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let trait_node = node.child_by_field_name("trait");
        let impl_name = if let Some(trait_n) = trait_node {
            let trait_name = self.node_text(&trait_n, source);
            format!("{trait_name} for {type_name}")
        } else {
            type_name.clone()
        };

        let qualified_impl = if qualified_prefix.is_empty() {
            impl_name.clone()
        } else {
            format!("{qualified_prefix}::{impl_name}")
        };

        let impl_id = Self::generate_id();
        let now = Utc::now();

        let impl_symbol = Some(CodeSymbol {
            id: impl_id.clone(),
            qualified_name: qualified_impl.clone(),
            name: impl_name,
            kind: SymbolKind::Impl,
            language: Language::Rust,
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: self.node_text(node, source).to_string(),
            documentation: self.extract_doc_comment(node, source),
            signature: None,
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        });

        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "function_item" {
                    if let Some(method) = self.extract_function(
                        &child,
                        source,
                        file_path,
                        file_id,
                        repository,
                        Some(&impl_id),
                        &qualified_impl,
                    ) {
                        methods.push(method);
                    }
                }
            }
        }

        (impl_symbol, methods)
    }
}
