//! Filter query grammar for the memory search index.
//!
//! Agents issue queries like:
//!
//! ```text
//! module:cas-mcp severity:critical worker timeout
//! ```
//!
//! which is parsed as:
//! - keyword residual: `"worker timeout"`
//! - filters: `[("module", "cas-mcp"), ("severity", "critical")]`
//!
//! Only a fixed set of keys is recognized as filters — tokens whose prefix is
//! not in the allow-list are passed through as raw keyword text so that
//! legacy queries (and URLs, namespaces, etc.) continue to work unchanged.

/// Filter keys recognized by the search index. Must match the Tantivy field
/// names added in `search_index_impl::build_schema`.
pub const FILTER_KEYS: &[&str] = &[
    "module",
    "track",
    "problem_type",
    "severity",
    "root_cause",
    "date",
];

/// Parsed query: the text portion (post-filter-strip) and the list of
/// `(key, value)` filter pairs.
#[derive(Debug, Default, Clone)]
pub struct ParsedQuery {
    /// Residual keyword text after filter tokens have been removed. May be
    /// empty when the query is pure filters.
    pub residual: String,
    /// Recognized filter tokens in query order.
    pub filters: Vec<(String, String)>,
}

/// Split a query string into a keyword residual and a list of recognized
/// `key:value` filters.
///
/// Rules:
/// - Whitespace-delimited tokens.
/// - A token matching `key:value` where `key` is in [`FILTER_KEYS`] becomes a
///   filter; key is lowercased, value is kept as-is (trimmed).
/// - A token with a colon but an unknown key is treated as raw text (returned
///   unchanged in the residual). This preserves back-compat for queries like
///   `cas:task` or URLs.
/// - An empty value (`module:`) is ignored — passed through as raw text.
pub fn parse_filter_query(query: &str) -> ParsedQuery {
    let mut residual_tokens: Vec<&str> = Vec::new();
    let mut filters: Vec<(String, String)> = Vec::new();

    for token in query.split_whitespace() {
        if let Some((k, v)) = token.split_once(':') {
            let key = k.to_ascii_lowercase();
            if !v.is_empty() && FILTER_KEYS.contains(&key.as_str()) {
                filters.push((key, v.to_string()));
                continue;
            }
        }
        residual_tokens.push(token);
    }

    ParsedQuery {
        residual: residual_tokens.join(" "),
        filters,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_keyword_query() {
        let p = parse_filter_query("worker timeout deadlock");
        assert_eq!(p.residual, "worker timeout deadlock");
        assert!(p.filters.is_empty());
    }

    #[test]
    fn pure_filter_query() {
        let p = parse_filter_query("module:cas-mcp severity:critical");
        assert_eq!(p.residual, "");
        assert_eq!(
            p.filters,
            vec![
                ("module".to_string(), "cas-mcp".to_string()),
                ("severity".to_string(), "critical".to_string()),
            ]
        );
    }

    #[test]
    fn mixed_keyword_and_filter() {
        let p = parse_filter_query("worker module:cas-mcp timeout");
        assert_eq!(p.residual, "worker timeout");
        assert_eq!(p.filters.len(), 1);
        assert_eq!(p.filters[0], ("module".to_string(), "cas-mcp".to_string()));
    }

    #[test]
    fn unknown_key_passes_through_as_text() {
        let p = parse_filter_query("foo:bar module:cas-tui baz");
        assert_eq!(p.residual, "foo:bar baz");
        assert_eq!(p.filters.len(), 1);
    }

    #[test]
    fn empty_value_passes_through() {
        let p = parse_filter_query("module: critical");
        assert_eq!(p.residual, "module: critical");
        assert!(p.filters.is_empty());
    }

    #[test]
    fn key_case_is_normalized() {
        let p = parse_filter_query("Module:cas-mcp");
        assert_eq!(p.filters.len(), 1);
        assert_eq!(p.filters[0].0, "module");
    }

    #[test]
    fn all_known_keys_parsed() {
        let q =
            "track:bug module:cas-mcp problem_type:runtime_error severity:critical root_cause:environment date:2026-03-30";
        let p = parse_filter_query(q);
        assert_eq!(p.filters.len(), 6);
        assert_eq!(p.residual, "");
    }
}
