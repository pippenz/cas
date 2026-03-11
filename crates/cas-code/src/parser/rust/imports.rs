use tree_sitter::Node;

use crate::parser::rust::RustParser;
use crate::types::Import;

impl RustParser {
    /// Extract use/import statements.
    pub(crate) fn extract_use_statement(&self, node: &Node, source: &[u8]) -> Option<Import> {
        let use_tree = node.child_by_field_name("argument")?;
        let path = self.node_text(&use_tree, source);

        let is_reexport = node
            .child(0)
            .map(|c| c.kind() == "visibility_modifier")
            .unwrap_or(false);

        Some(Import {
            module_path: path.to_string(),
            items: Vec::new(),
            line: node.start_position().row + 1,
            is_reexport,
        })
    }
}
