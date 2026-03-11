use tree_sitter::Node;

use crate::parser::rust::RustParser;
use crate::types::{CodeSymbol, Import};

impl RustParser {
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
    ) {
        match node.kind() {
            "function_item" => {
                if let Some(sym) = self.extract_function(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    None,
                    qualified_prefix,
                ) {
                    symbols.push(sym);
                }
            }
            "struct_item" => {
                if let Some(sym) = self.extract_struct(
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
            "enum_item" => {
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
            "trait_item" => {
                if let Some(sym) = self.extract_trait(
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
            "impl_item" => {
                let (impl_sym, methods) = self.extract_impl(
                    node,
                    source,
                    file_path,
                    file_id,
                    repository,
                    qualified_prefix,
                );
                if let Some(sym) = impl_sym {
                    symbols.push(sym);
                }
                symbols.extend(methods);
                return;
            }
            "use_declaration" => {
                if let Some(import) = self.extract_use_statement(node, source) {
                    imports.push(import);
                }
            }
            "mod_item" => {
                if let Some(name_node) = node.child_by_field_name("name") {
                    let mod_name = self.node_text(&name_node, source);
                    let new_prefix = if qualified_prefix.is_empty() {
                        mod_name.to_string()
                    } else {
                        format!("{qualified_prefix}::{mod_name}")
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
            );
        }
    }
}
