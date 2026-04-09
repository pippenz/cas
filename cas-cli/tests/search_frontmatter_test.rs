//! Tests for cas-7b1e: search index frontmatter parsing + filter grammar +
//! module-scoped candidate retrieval API.
//!
//! Scenarios:
//! 1. Parse a memory with full structured frontmatter — all fields indexed
//! 2. Parse a legacy memory (no frontmatter or legacy-only) — keyword search
//!    still works, filter queries return empty
//! 3. `module:cas-mcp` returns only memories with that module
//! 4. `severity:critical track:bug` returns intersection
//! 5. `search_module_candidates` returns top-N scoped to a module
//! 6. Index rebuild on mixed legacy + structured memories succeeds

use cas::hybrid_search::{SearchIndex, SearchOptions};
use cas::types::Entry;

fn mem(id: &str, content: &str) -> Entry {
    Entry {
        id: id.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

const STRUCTURED_MCP_CRITICAL: &str = "\
---
name: wal_timeout
description: NTFS3 WAL deadlock
type: bugfix
track: bug
module: cas-mcp
problem_type: runtime_error
severity: critical
root_cause: environment
date: 2026-03-30
---
Every MCP tool call times out after 60s on NTFS3 due to WAL file locking.
";

const STRUCTURED_MCP_HIGH: &str = "\
---
name: mcp_worker_missing
description: Worker worktree missing mcp__cas
type: bugfix
track: bug
module: cas-mcp
problem_type: config_error
severity: high
root_cause: misconfiguration
date: 2026-04-01
---
Factory workers sometimes spawn without mcp__cas connection.
";

const STRUCTURED_SEARCH_CRITICAL: &str = "\
---
name: search_frontmatter
description: Extend search index with frontmatter filters
type: learning
track: knowledge
module: cas-search
problem_type: design_decision
severity: critical
root_cause: feature_gap
date: 2026-04-09
---
Search index must parse memory frontmatter and support module filters.
";

const STRUCTURED_TUI_LOW: &str = "\
---
name: panel_flash
description: TUI panel flashing empty
type: bugfix
track: bug
module: cas-tui
problem_type: ui_glitch
severity: low
root_cause: race_condition
date: 2026-03-28
---
Task panel flashes empty because of a read race between list and deps.
";

const LEGACY_MEMORY: &str = "\
---
name: legacy_memory
description: A legacy memory
type: learning
---
This is a legacy memory about WAL behavior. It mentions MCP and cas-mcp in
the body but has no structured frontmatter fields.
";

const NO_FRONTMATTER: &str = "Just some text about WAL and cas-mcp with no frontmatter at all.";

fn load_all(index: &SearchIndex) {
    let entries = vec![
        mem("001", STRUCTURED_MCP_CRITICAL),
        mem("002", STRUCTURED_MCP_HIGH),
        mem("003", STRUCTURED_SEARCH_CRITICAL),
        mem("004", STRUCTURED_TUI_LOW),
        mem("005", LEGACY_MEMORY),
        mem("006", NO_FRONTMATTER),
    ];
    index.index_entries_batch(&entries).unwrap();
}

#[test]
fn frontmatter_fields_are_indexed_for_structured_memory() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // A pure filter query (no text terms) on module should return the
    // structured memories tagged cas-mcp.
    let opts = SearchOptions {
        query: "module:cas-mcp".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = index.search_unified(&opts).unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"001"), "001 should match module:cas-mcp");
    assert!(ids.contains(&"002"), "002 should match module:cas-mcp");
    assert!(!ids.contains(&"003"), "003 is cas-search module");
    assert!(!ids.contains(&"004"), "004 is cas-tui module");
    assert!(
        !ids.contains(&"005"),
        "005 is legacy, no module — filter must not match"
    );
    assert!(!ids.contains(&"006"), "006 has no frontmatter");
}

#[test]
fn legacy_memory_still_matches_keyword_search() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // Keyword-only query (no filter) should match the legacy memory body.
    let opts = SearchOptions {
        query: "legacy".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = index.search_unified(&opts).unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(
        ids.contains(&"005"),
        "legacy memory should match keyword search"
    );
}

#[test]
fn legacy_memory_excluded_by_filter_query() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // Even though the legacy body mentions "cas-mcp", it has no structured
    // module field — the filter must not match it.
    let opts = SearchOptions {
        query: "module:cas-mcp".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = index.search_unified(&opts).unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(!ids.contains(&"005"));
    assert!(!ids.contains(&"006"));
}

#[test]
fn filter_intersection_severity_and_track() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    let opts = SearchOptions {
        query: "severity:critical track:bug".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = index.search_unified(&opts).unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    // 001: critical + bug -> match
    // 002: high + bug -> no (severity)
    // 003: critical + knowledge -> no (track)
    // 004: low + bug -> no (severity)
    assert_eq!(
        ids,
        vec!["001"],
        "only 001 matches critical+bug, got {:?}",
        ids
    );
}

#[test]
fn filter_with_keyword_and_key_value_mixed() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // Keyword 'WAL' present in 001 and 005. Filter module:cas-mcp picks only
    // structured memories. Combined should return 001 only.
    let opts = SearchOptions {
        query: "WAL module:cas-mcp".to_string(),
        limit: 10,
        ..Default::default()
    };
    let results = index.search_unified(&opts).unwrap();
    let ids: Vec<&str> = results.iter().map(|r| r.id.as_str()).collect();
    assert!(ids.contains(&"001"));
    assert!(!ids.contains(&"005"));
}

#[test]
fn module_scoped_candidate_retrieval_api() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // Unit 3's consumer API: given a query and a module, return top-N same-
    // module candidates.
    let hits = index
        .search_module_candidates("worker deadlock timeout", "cas-mcp", 5)
        .unwrap();
    assert!(!hits.is_empty(), "expected at least one cas-mcp candidate");
    for h in &hits {
        assert!(
            h.id == "001" || h.id == "002",
            "candidate {} is not a cas-mcp memory",
            h.id
        );
    }
}

#[test]
fn module_scoped_candidate_excludes_legacy() {
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    let hits = index
        .search_module_candidates("legacy WAL mcp", "cas-mcp", 5)
        .unwrap();
    for h in &hits {
        assert_ne!(h.id, "005", "legacy memory must not be returned");
        assert_ne!(h.id, "006", "no-frontmatter memory must not be returned");
    }
}

#[test]
fn mixed_index_rebuild_succeeds() {
    // Rebuild index on a mixed legacy + structured set — must not panic or err.
    let index = SearchIndex::in_memory().unwrap();
    let entries = vec![
        mem("001", STRUCTURED_MCP_CRITICAL),
        mem("005", LEGACY_MEMORY),
        mem("006", NO_FRONTMATTER),
    ];
    let count = index.index_entries_batch(&entries).unwrap();
    assert_eq!(count, 3);

    // Re-run indexing to confirm delete+reinsert path also handles frontmatter.
    let count2 = index.index_entries_batch(&entries).unwrap();
    assert_eq!(count2, 3);

    // Both keyword and filter queries should work post-rebuild.
    let opts = SearchOptions {
        query: "module:cas-mcp".to_string(),
        limit: 10,
        ..Default::default()
    };
    let r = index.search_unified(&opts).unwrap();
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].id, "001");
}

#[test]
fn filter_query_unknown_key_treated_as_keyword() {
    // Backwards compat: a `key:value` pair for an unknown key should not
    // blow up filter parsing; it is passed through as raw text.
    let index = SearchIndex::in_memory().unwrap();
    load_all(&index);

    // 'foo:bar' is not a known filter key. Query should not error.
    let opts = SearchOptions {
        query: "foo:bar legacy".to_string(),
        limit: 10,
        ..Default::default()
    };
    let _ = index.search_unified(&opts).unwrap();
}
