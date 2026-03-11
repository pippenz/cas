use tree_sitter::Node;

use crate::parser::typescript::TypeScriptParser;
use crate::types::Import;

impl TypeScriptParser {
    /// Extract import statements.
    pub(crate) fn extract_import(&self, node: &Node, source: &[u8]) -> Option<Import> {
        let source_node = node.child_by_field_name("source")?;
        let module_path = self
            .node_text(&source_node, source)
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();

        let mut items = Vec::new();
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "import_clause" {
                let mut clause_cursor = child.walk();
                for clause_child in child.children(&mut clause_cursor) {
                    match clause_child.kind() {
                        "identifier" => {
                            items.push(self.node_text(&clause_child, source).to_string());
                        }
                        "named_imports" => {
                            let mut import_cursor = clause_child.walk();
                            for import in clause_child.children(&mut import_cursor) {
                                if import.kind() == "import_specifier" {
                                    if let Some(name) = import.child_by_field_name("name") {
                                        items.push(self.node_text(&name, source).to_string());
                                    }
                                }
                            }
                        }
                        "namespace_import" => {
                            items.push(self.node_text(&clause_child, source).to_string());
                        }
                        _ => {}
                    }
                }
            }
        }

        Some(Import {
            module_path,
            items,
            line: node.start_position().row + 1,
            is_reexport: false,
        })
    }

    /// Extract export statements (for re-exports).
    pub(crate) fn extract_export(&self, node: &Node, source: &[u8]) -> Option<Import> {
        let source_node = node.child_by_field_name("source")?;
        let module_path = self
            .node_text(&source_node, source)
            .trim_matches(|c| c == '"' || c == '\'')
            .to_string();

        Some(Import {
            module_path,
            items: Vec::new(),
            line: node.start_position().row + 1,
            is_reexport: true,
        })
    }
}
