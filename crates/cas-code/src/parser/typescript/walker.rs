use chrono::Utc;
use tree_sitter::Node;

use crate::parser::typescript::TypeScriptParser;
use crate::types::{CodeSymbol, Import, Language, SymbolKind};

impl TypeScriptParser {
    /// Recursively walk the AST and extract symbols.
    pub(crate) fn walk_tree(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        symbols: &mut Vec<CodeSymbol>,
        imports: &mut Vec<Import>,
        is_js: bool,
    ) {
        match node.kind() {
            "function_declaration" | "generator_function_declaration" => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    None,
                    qualified_prefix,
                    is_js,
                ) {
                    symbols.push(sym);
                }
            }
            "class_declaration" | "abstract_class_declaration" => {
                let (class_sym, methods) = self.extract_class(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                    is_js,
                );
                if let Some(sym) = class_sym {
                    symbols.push(sym);
                }
                symbols.extend(methods);
                return;
            }
            "interface_declaration" => {
                if let Some(sym) = self.extract_interface(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                ) {
                    symbols.push(sym);
                }
            }
            "type_alias_declaration" => {
                if let Some(sym) = self.extract_type_alias(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                ) {
                    symbols.push(sym);
                }
            }
            "enum_declaration" => {
                if let Some(sym) = self.extract_enum(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                ) {
                    symbols.push(sym);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.extract_variable_functions(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                    symbols,
                    is_js,
                );
            }
            "import_statement" => {
                if let Some(import) = self.extract_import(node, source) {
                    imports.push(import);
                }
            }
            "export_statement" => {
                if let Some(import) = self.extract_export(node, source) {
                    imports.push(import);
                }

                if let Some(decl) = node.child_by_field_name("declaration") {
                    self.walk_tree(
                        &decl,
                        source,
                        file_path,
                        file_id,
                        repository,
                        qualified_prefix,
                        symbols,
                        imports,
                        is_js,
                    );
                    return;
                }
            }
            "internal_module" | "module" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let mod_name = self.node_text(&name_node, source);
                    let new_prefix = if qualified_prefix.is_empty() {
                        mod_name.to_string()
                    } else {
                        format!("{qualified_prefix}.{mod_name}")
                    };

                    if let Some(body) = node.child_by_field_name("body") {
                        let mut cursor = body.walk();
                        for child in body.children(&mut cursor) {
                            self.walk_tree(
                                &child,
                                source,
                                file_path,
                                file_id,
                                repository,
                                &new_prefix,
                                symbols,
                                imports,
                                is_js,
                            );
                        }
                    }
                }
                return;
            }
            _ => {}
        }

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
                is_js,
            );
        }
    }

    fn extract_variable_functions(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        symbols: &mut Vec<CodeSymbol>,
        is_js: bool,
    ) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() != "variable_declarator" {
                continue;
            }

            let Some(name_node) = child.child_by_field_name("name") else {
                continue;
            };
            let Some(value_node) = child.child_by_field_name("value") else {
                continue;
            };

            let name = self.node_text(&name_node, source);
            match value_node.kind() {
                "arrow_function" => {
                    if let Some(sym) = self.extract_arrow_function(
                        name,
                        &value_node,
                        source,
                        file_path,
                        file_id,
                        repository,
                        qualified_prefix,
                        is_js,
                    ) {
                        symbols.push(sym);
                    }
                }
                "function_expression" | "generator_function" => {
                    symbols.push(self.build_variable_function_symbol(
                        &child,
                        source,
                        file_path,
                        file_id,
                        repository,
                        qualified_prefix,
                        name,
                        is_js,
                    ));
                }
                _ => {}
            }
        }
    }

    fn build_variable_function_symbol(
        &self,
        node: &Node,
        source: &[u8],
        file_path: &str,
        file_id: &str,
        repository: &str,
        qualified_prefix: &str,
        name: &str,
        is_js: bool,
    ) -> CodeSymbol {
        let qualified_name = if qualified_prefix.is_empty() {
            name.to_string()
        } else {
            format!("{qualified_prefix}.{name}")
        };

        let now = Utc::now();
        CodeSymbol {
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
            source: self.node_text(node, source).to_string(),
            documentation: self.extract_jsdoc(node, source),
            signature: Some(format!("const {name} = function(...)")),
            parent_id: None,
            repository: repository.to_string(),
            commit_hash: None,
            created: now,
            updated: now,
            content_hash: Self::content_hash(&self.node_text(node, source)),
            scope: "project".to_string(),
        }
    }
}
