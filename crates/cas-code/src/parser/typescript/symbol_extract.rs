use chrono::Utc;
use tree_sitter::Node;

use crate::parser::typescript::TypeScriptParser;
use crate::types::{CodeSymbol, Language, SymbolKind};

impl TypeScriptParser {
    pub(crate) fn extract_function(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        parent_id: Option<&str>,
        qualified_prefix: &str,
        is_js: bool,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("()");

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let signature = if return_type.is_empty() {
            format!("function {name}{type_params}{params}")
        } else {
            format!("function {name}{type_params}{params}{return_type}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_jsdoc(node, source);
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
            language: if is_js {
                Language::JavaScript
            } else {
                Language::TypeScript
            },
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

    pub(crate) fn extract_arrow_function(
        &self,
        name: &str,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        is_js: bool,
    ) -> Option<CodeSymbol> {
        let qualified_name = if qualified_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source).to_string())
            .or_else(|| {
                node.child_by_field_name("parameter")
                    .map(|n| format!("({})", self.node_text(&n, source)))
            })
            .unwrap_or_else(|| "()".to_string());

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let signature = if return_type.is_empty() {
            format!("const {name} = {type_params}{params} => ...")
        } else {
            format!("const {name} = {type_params}{params}{return_type} => ...")
        };

        let full_source = self.node_text(node, source).to_string();
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name: name.to_string(),
            kind: SymbolKind::Function,
            language: if is_js {
                Language::JavaScript
            } else {
                Language::TypeScript
            },
            file_path: file_path.to_string(),
            file_id: file_id.to_string(),
            line_start: node.start_position().row + 1,
            line_end: node.end_position().row + 1,
            source: full_source,
            documentation: None,
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

    pub(crate) fn extract_class(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        is_js: bool,
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
        let doc = self.extract_jsdoc(node, source);
        let now = Utc::now();

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let mut heritage = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_heritage" {
                heritage = format!(" {}", self.node_text(&child, source));
                break;
            }
        }

        let is_abstract = node.kind() == "abstract_class_declaration";
        let prefix = if is_abstract {
            "abstract class"
        } else {
            "class"
        };
        let signature = format!("{prefix} {name}{type_params}{heritage}");

        let class_symbol = Some(CodeSymbol {
            id: class_id.clone(),
            qualified_name: qualified_name.clone(),
            name: name.clone(),
            kind: SymbolKind::Class,
            language: if is_js {
                Language::JavaScript
            } else {
                Language::TypeScript
            },
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

        if let Some(body) = node.child_by_field_name("body") {
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                match child.kind() {
                    "method_definition" | "public_field_definition" => {
                        if let Some(method) = self.extract_method(
                            &child,
                            source,
                            file_path,
                            file_id,
                            repository,
                            Some(&class_id),
                            &qualified_name,
                            is_js,
                        ) {
                            methods.push(method);
                        }
                    }
                    _ => {}
                }
            }
        }

        (class_symbol, methods)
    }

    pub(crate) fn extract_method(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        parent_id: Option<&str>,
        qualified_prefix: &str,
        is_js: bool,
    ) -> Option<CodeSymbol> {
        let name_node = node.child_by_field_name("name")?;
        let name = self.node_text(&name_node, source).to_string();

        let qualified_name = if qualified_prefix.is_empty() {
            name.clone()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        let params = node
            .child_by_field_name("parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("()");

        let return_type = node
            .child_by_field_name("return_type")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let mut modifiers = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            match child.kind() {
                "accessibility_modifier" | "override_modifier" => {
                    modifiers.push(self.node_text(&child, source));
                }
                _ => {}
            }
        }

        let is_getter = node.child(0).map(|c| c.kind() == "get").unwrap_or(false);
        let is_setter = node.child(0).map(|c| c.kind() == "set").unwrap_or(false);

        let prefix = if is_getter {
            "get "
        } else if is_setter {
            "set "
        } else {
            ""
        };

        let modifier_str = if modifiers.is_empty() {
            String::new()
        } else {
            format!("{} ", modifiers.join(" "))
        };

        let signature = if return_type.is_empty() {
            format!("{modifier_str}{prefix}{name}{type_params}{params}")
        } else {
            format!("{modifier_str}{prefix}{name}{type_params}{params}{return_type}")
        };

        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_jsdoc(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Method,
            language: if is_js {
                Language::JavaScript
            } else {
                Language::TypeScript
            },
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

    pub(crate) fn extract_interface(
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
            format!("{qualified_prefix}.{name}")
        };

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let mut extends = String::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "extends_type_clause" {
                extends = format!(" {}", self.node_text(&child, source));
                break;
            }
        }

        let signature = format!("interface {name}{type_params}{extends}");
        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_jsdoc(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Interface,
            language: Language::TypeScript,
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

    pub(crate) fn extract_type_alias(
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
            format!("{qualified_prefix}.{name}")
        };

        let type_params = node
            .child_by_field_name("type_parameters")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("");

        let value = node
            .child_by_field_name("value")
            .map(|n| self.node_text(&n, source))
            .unwrap_or("unknown");

        let signature = format!("type {name}{type_params} = {value}");
        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_jsdoc(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Type,
            language: Language::TypeScript,
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
            format!("{qualified_prefix}.{name}")
        };

        let is_const = node.child(0).map(|c| c.kind() == "const").unwrap_or(false);
        let prefix = if is_const { "const enum" } else { "enum" };

        let signature = format!("{prefix} {name}");
        let full_source = self.node_text(node, source).to_string();
        let doc = self.extract_jsdoc(node, source);
        let now = Utc::now();

        Some(CodeSymbol {
            id: Self::generate_id(),
            qualified_name,
            name,
            kind: SymbolKind::Enum,
            language: Language::TypeScript,
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
}
