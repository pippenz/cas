use crate::cli::hook::*;
use crate::cli::hook::config_gen::has_cas_hook_entries;
use tempfile::TempDir;
use toml::map::Map;

#[test]
fn test_configure_creates_settings() {
    let temp = TempDir::new().unwrap();
    let result = configure_claude_hooks(temp.path(), false).unwrap();

    assert!(result); // Created new file
    assert!(temp.path().join(".claude/settings.json").exists());

    let content = std::fs::read_to_string(temp.path().join(".claude/settings.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

    if global_has_cas_hooks() {
        // Global hooks exist — project should NOT have hooks
        assert!(
            settings.get("hooks").is_none(),
            "Hooks should be omitted when global hooks exist"
        );
    } else {
        // No global hooks — project should have hooks
        assert!(settings.pointer("/hooks/SessionStart").is_some());
        assert!(settings.pointer("/hooks/SessionEnd").is_some());
        assert!(settings.pointer("/hooks/Stop").is_some());
        assert!(settings.pointer("/hooks/SubagentStop").is_some());
        assert!(settings.pointer("/hooks/PostToolUse").is_some());
        assert!(settings.pointer("/hooks/UserPromptSubmit").is_some());
    }

    // Permissions should always be written
    let allow = settings
        .pointer("/permissions/allow")
        .expect("permissions.allow missing");
    let allow_arr = allow.as_array().expect("permissions.allow is not array");
    assert!(
        allow_arr.iter().any(|v| v.as_str() == Some("Bash(cas :*)")),
        "Bash(cas :*) permission missing"
    );
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some("mcp__cas__task")),
        "mcp__cas__task permission missing"
    );
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some("mcp__cas__coordination")),
        "mcp__cas__coordination permission missing"
    );
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some("mcp__cas__memory")),
        "mcp__cas__memory permission missing"
    );
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some("mcp__cas__search")),
        "mcp__cas__search permission missing"
    );
}

#[test]
fn test_configure_merges_existing() {
    let temp = TempDir::new().unwrap();
    let claude_dir = temp.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();

    // Create existing settings with custom content
    let existing = serde_json::json!({
        "permissions": {
            "allow": ["Read", "Write"]
        },
        "hooks": {
            "CustomHook": [{"hooks": [{"type": "command", "command": "echo custom"}]}]
        }
    });
    std::fs::write(
        claude_dir.join("settings.json"),
        serde_json::to_string_pretty(&existing).unwrap(),
    )
    .unwrap();

    // Configure CAS hooks
    let result = configure_claude_hooks(temp.path(), false).unwrap();
    assert!(!result); // Updated, not created

    let content = std::fs::read_to_string(claude_dir.join("settings.json")).unwrap();
    let settings: serde_json::Value = serde_json::from_str(&content).unwrap();

    if global_has_cas_hooks() {
        // Global hooks exist — CAS hooks should NOT be added to project
        assert!(
            settings.pointer("/hooks/SessionStart").is_none(),
            "CAS hooks should not be added when global hooks exist"
        );
        // Non-CAS custom hook should be preserved
        assert!(
            settings.pointer("/hooks/CustomHook").is_some(),
            "Non-CAS custom hooks should be preserved"
        );
    } else {
        // No global hooks — CAS hooks should be added
        assert!(settings.pointer("/hooks/SessionStart").is_some());
        assert!(settings.pointer("/hooks/Stop").is_some());
        assert!(settings.pointer("/hooks/PostToolUse").is_some());
    }

    // Existing permissions should always be preserved and CAS permissions added
    let allow = settings
        .pointer("/permissions/allow")
        .expect("permissions.allow missing");
    let allow_arr = allow.as_array().expect("permissions.allow is not array");

    assert!(
        allow_arr.iter().any(|v| v.as_str() == Some("Read")),
        "Original Read permission should be preserved"
    );
    assert!(
        allow_arr.iter().any(|v| v.as_str() == Some("Write")),
        "Original Write permission should be preserved"
    );
    assert!(
        allow_arr.iter().any(|v| v.as_str() == Some("Bash(cas :*)")),
        "Bash(cas :*) permission should be added"
    );
    assert!(
        allow_arr
            .iter()
            .any(|v| v.as_str() == Some("mcp__cas__task")),
        "mcp__cas__task permission should be added"
    );
}

#[test]
fn test_strip_cas_hooks() {
    let mut settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "cas hook PreToolUse"}]}],
            "SessionStart": [
                {"hooks": [{"type": "command", "command": "cas hook SessionStart"}]},
                {"hooks": [{"type": "command", "command": "cas factory check-staleness"}]}
            ],
            "CustomHook": [{"hooks": [{"type": "command", "command": "echo custom"}]}]
        },
        "permissions": {"allow": ["Read"]}
    });

    let modified = strip_cas_hooks(&mut settings);
    assert!(modified);

    // CAS hooks should be removed
    assert!(settings.pointer("/hooks/PreToolUse").is_none());
    assert!(settings.pointer("/hooks/SessionStart").is_none());

    // Non-CAS hook should be preserved
    assert!(settings.pointer("/hooks/CustomHook").is_some());

    // Permissions should be untouched
    assert!(settings.pointer("/permissions/allow").is_some());
}

#[test]
fn test_strip_cas_hooks_removes_empty_hooks_object() {
    let mut settings = serde_json::json!({
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "cas hook PreToolUse"}]}]
        },
        "permissions": {"allow": ["Read"]}
    });

    strip_cas_hooks(&mut settings);

    // hooks object should be completely removed when empty
    assert!(settings.get("hooks").is_none());
    assert!(settings.get("permissions").is_some());
}

#[test]
fn test_has_cas_hook_entries() {
    let with_hooks = serde_json::json!({
        "hooks": {
            "PreToolUse": [{"hooks": [{"type": "command", "command": "cas hook PreToolUse"}]}]
        }
    });
    assert!(has_cas_hook_entries(&with_hooks));

    let without_hooks = serde_json::json!({
        "hooks": {
            "Custom": [{"hooks": [{"type": "command", "command": "echo test"}]}]
        }
    });
    assert!(!has_cas_hook_entries(&without_hooks));

    let no_hooks = serde_json::json!({"permissions": {}});
    assert!(!has_cas_hook_entries(&no_hooks));
}

#[test]
fn test_configure_codex_creates_config() {
    let temp = TempDir::new().unwrap();
    let result = configure_codex_mcp_server(temp.path()).unwrap();

    assert!(result);
    let config_path = temp.path().join(".codex/config.toml");
    assert!(config_path.exists());

    let content = std::fs::read_to_string(&config_path).unwrap();
    let config: toml::Value = toml::from_str(&content).unwrap();
    let entry = config
        .get("mcp_servers")
        .and_then(|v| v.get("cas"))
        .and_then(|v| v.as_table())
        .expect("mcp_servers.cas missing");

    assert_eq!(
        entry.get("command"),
        Some(&toml::Value::String("cas".to_string()))
    );
    assert_eq!(
        entry.get("args"),
        Some(&toml::Value::Array(vec![toml::Value::String(
            "serve".to_string()
        )]))
    );
    assert_eq!(
        entry.get("env"),
        Some(&toml::Value::Table({
            let mut env = Map::new();
            env.insert(
                "CAS_CODEX_FALLBACK_SESSION".to_string(),
                toml::Value::String("1".to_string()),
            );
            env
        }))
    );
}

#[test]
fn test_configure_codex_updates_existing_entry() {
    let temp = TempDir::new().unwrap();
    let codex_dir = temp.path().join(".codex");
    std::fs::create_dir_all(&codex_dir).unwrap();

    let content = r#"
[mcp_servers.context7]
command = "cas"
args = ["old"]
env = { CAS_LOG = "debug" }
"#;
    std::fs::write(codex_dir.join("config.toml"), content).unwrap();

    let result = configure_codex_mcp_server(temp.path()).unwrap();
    assert!(result);

    let updated = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
    let config: toml::Value = toml::from_str(&updated).unwrap();
    let entry = config
        .get("mcp_servers")
        .and_then(|v| v.get("context7"))
        .and_then(|v| v.as_table())
        .expect("mcp_servers.context7 missing");

    assert_eq!(
        entry.get("command"),
        Some(&toml::Value::String("cas".to_string()))
    );
    assert_eq!(
        entry.get("args"),
        Some(&toml::Value::Array(vec![toml::Value::String(
            "serve".to_string()
        )]))
    );
    assert_eq!(
        entry.get("env"),
        Some(&toml::Value::Table({
            let mut env = Map::new();
            env.insert(
                "CAS_LOG".to_string(),
                toml::Value::String("debug".to_string()),
            );
            env.insert(
                "CAS_CODEX_FALLBACK_SESSION".to_string(),
                toml::Value::String("1".to_string()),
            );
            env
        }))
    );
}

// Note: configure_mcp_server tests removed because they require the claude CLI
// which isn't available in test environments. The function now uses `claude mcp add`.
