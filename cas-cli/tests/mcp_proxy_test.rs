//! Integration tests for the cas-mcp-proxy (code-mode-mcp) crate.
//!
//! These tests verify the config API, catalog serialization format,
//! and compatibility with the proxy_catalog.json cache consumed by
//! SessionStart context injection.

#![cfg(feature = "mcp-proxy")]

use std::collections::HashMap;
use std::path::Path;

use cmcp_core::config::{Config, Scope, ServerConfig};
use cmcp_core::CatalogEntry;

// ── Config round-trip ────────────────────────────────────────────────

#[test]
fn config_round_trip_all_transports() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proxy.toml");

    let mut config = Config::default();

    config.add_server(
        "my-stdio".to_string(),
        ServerConfig::Stdio {
            command: "npx".to_string(),
            args: vec!["mcp-server-git".to_string()],
            env: HashMap::from([("HOME".to_string(), "/tmp".to_string())]),
        },
    );

    config.add_server(
        "my-http".to_string(),
        ServerConfig::Http {
            url: "https://mcp.example.com/api".to_string(),
            auth: Some("secret-token".to_string()),
            headers: HashMap::new(),
            oauth: false,
        },
    );

    config.add_server(
        "my-sse".to_string(),
        ServerConfig::Sse {
            url: "https://mcp.example.com/sse".to_string(),
            auth: None,
            headers: HashMap::from([("X-Custom".to_string(), "value".to_string())]),
            oauth: true,
        },
    );

    config.save_to(&path).unwrap();
    let loaded = Config::load_from(&path).unwrap();
    assert_eq!(config, loaded);
    assert_eq!(loaded.servers.len(), 3);
}

#[test]
fn config_add_remove_overwrite() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("proxy.toml");

    let mut config = Config::default();
    config.add_server(
        "srv".to_string(),
        ServerConfig::Stdio {
            command: "old".to_string(),
            args: vec![],
            env: HashMap::new(),
        },
    );
    config.save_to(&path).unwrap();

    // Overwrite with new config
    config.add_server(
        "srv".to_string(),
        ServerConfig::Stdio {
            command: "new".to_string(),
            args: vec!["--flag".to_string()],
            env: HashMap::new(),
        },
    );
    config.save_to(&path).unwrap();

    let loaded = Config::load_from(&path).unwrap();
    match &loaded.servers["srv"] {
        ServerConfig::Stdio { command, args, .. } => {
            assert_eq!(command, "new");
            assert_eq!(args, &["--flag"]);
        }
        _ => panic!("expected Stdio"),
    }

    // Remove
    let mut loaded = loaded;
    assert!(loaded.remove_server("srv"));
    assert!(!loaded.remove_server("srv")); // Already gone
    loaded.save_to(&path).unwrap();

    let final_config = Config::load_from(&path).unwrap();
    assert!(final_config.servers.is_empty());
}

#[test]
fn config_load_missing_returns_empty() {
    let config = Config::load_from(Path::new("/tmp/nonexistent-cas-test/proxy.toml")).unwrap();
    assert!(config.servers.is_empty());
}

#[test]
fn config_merge_project_over_user() {
    let dir = tempfile::tempdir().unwrap();

    // Simulate project config
    let project_path = dir.path().join("project.toml");
    let mut project = Config::default();
    project.add_server(
        "shared".to_string(),
        ServerConfig::Http {
            url: "https://project.example.com".to_string(),
            auth: None,
            headers: HashMap::new(),
            oauth: false,
        },
    );
    project.save_to(&project_path).unwrap();

    // load_merged with project path
    let merged = Config::load_merged(Some(&project_path)).unwrap();
    assert!(merged.servers.contains_key("shared"));
}

#[test]
fn scope_user_config_path_valid() {
    let path = Scope::User.config_path().unwrap();
    assert!(path.to_string_lossy().contains("code-mode-mcp"));
    assert!(path.to_string_lossy().ends_with("config.toml"));
}

// ── Catalog serialization ────────────────────────────────────────────

#[test]
fn catalog_entry_serializes_to_json() {
    let entry = CatalogEntry {
        name: "take_screenshot".to_string(),
        description: Some("Captures a screenshot of the page".to_string()),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "url": { "type": "string" }
            }
        }),
    };

    let json = serde_json::to_value(&entry).unwrap();
    assert_eq!(json["name"], "take_screenshot");
    assert_eq!(
        json["description"],
        "Captures a screenshot of the page"
    );
    assert!(json["input_schema"]["properties"]["url"].is_object());
}

#[test]
fn catalog_entries_by_server_format_compatible_with_cache() {
    // The proxy_catalog.json cache format expected by build_mcp_tools_section
    // is: { "server_name": ["tool1", "tool2"] }
    // write_proxy_catalog_cache converts CatalogEntry → just names.
    // Verify that our CatalogEntry.name is what gets written.

    let entries = vec![
        CatalogEntry {
            name: "navigate_page".to_string(),
            description: Some("Navigate to URL".to_string()),
            input_schema: serde_json::json!({}),
        },
        CatalogEntry {
            name: "take_screenshot".to_string(),
            description: None,
            input_schema: serde_json::json!({}),
        },
    ];

    // Simulate the conversion done in write_proxy_catalog_cache
    let mut catalog: HashMap<String, Vec<String>> = HashMap::new();
    catalog.insert(
        "chrome-devtools".to_string(),
        entries.iter().map(|e| e.name.clone()).collect(),
    );

    let json = serde_json::to_string(&catalog).unwrap();

    // Verify it can be deserialized as BTreeMap<String, Vec<String>>
    // (the format build_mcp_tools_section expects)
    let parsed: std::collections::BTreeMap<String, Vec<String>> =
        serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["chrome-devtools"].len(), 2);
    assert!(parsed["chrome-devtools"].contains(&"navigate_page".to_string()));
    assert!(parsed["chrome-devtools"].contains(&"take_screenshot".to_string()));
}
