use std::path::Path;

use toml::map::Map;

/// Get the CAS hooks configuration JSON
///
/// Note: Claude Code 2.1.0+ supports `once: true` for hooks that should only run once
/// per session, even if resumed. CAS hooks intentionally do NOT use `once: true` because:
/// - SessionStart should inject context on every session start/resume
/// - PostToolUse and Stop should run on every matching event
///
/// Users can manually add `"once": true` to specific hooks if desired.
pub(crate) fn get_cas_hooks_config(config: &crate::config::HookConfig) -> serde_json::Value {
    // Build hooks config, only including enabled hooks
    let mut hooks = serde_json::Map::new();

    if config.session_start.enabled {
        hooks.insert(
            "SessionStart".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook SessionStart",
                            "timeout": config.session_start.timeout
                        }
                    ]
                },
                {
                    // Factory worktree staleness check - warns workers if behind remote
                    // Silent when up-to-date, so safe to run for all agents
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas factory check-staleness",
                            "timeout": 5000
                        }
                    ]
                }
            ]),
        );
    }

    // SessionEnd always uses session_start timeout (no separate config needed)
    // async: true - pure cleanup, no context injection
    if config.session_start.enabled {
        hooks.insert(
            "SessionEnd".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook SessionEnd",
                            "timeout": config.session_start.timeout,
                            "async": true
                        }
                    ]
                }
            ]),
        );
    }

    if config.stop.enabled {
        hooks.insert(
            "Stop".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook Stop",
                            "timeout": config.stop.timeout
                        }
                    ]
                }
            ]),
        );
    }

    // SubagentStart for verification jail unjailing (matcher: task-verifier)
    // async: true - database update only, no blocking decision
    if config.stop.enabled {
        hooks.insert(
            "SubagentStart".to_string(),
            serde_json::json!([
                {
                    "matcher": "task-verifier",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook SubagentStart",
                            "timeout": 2000,
                            "async": true
                        }
                    ]
                }
            ]),
        );
    }

    // SubagentStop uses stop timeout
    // async: true - marker file cleanup only
    if config.stop.enabled {
        hooks.insert(
            "SubagentStop".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook SubagentStop",
                            "timeout": config.stop.timeout / 2, // Subagent cleanup is quicker
                            "async": true
                        }
                    ]
                }
            ]),
        );
    }

    // async: true - observation recording, doesn't affect execution
    if config.post_tool_use.enabled {
        let matcher = config.post_tool_use.matcher.join("|");
        hooks.insert(
            "PostToolUse".to_string(),
            serde_json::json!([
                {
                    "matcher": matcher,
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook PostToolUse",
                            "timeout": config.post_tool_use.timeout,
                            "async": true
                        }
                    ]
                }
            ]),
        );
    }

    if config.pre_tool_use.enabled {
        let matcher = config.pre_tool_use.matcher.join("|");
        hooks.insert(
            "PreToolUse".to_string(),
            serde_json::json!([
                {
                    "matcher": matcher,
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook PreToolUse",
                            "timeout": config.pre_tool_use.timeout
                        }
                    ]
                }
            ]),
        );
    }

    if config.user_prompt_submit.enabled {
        hooks.insert(
            "UserPromptSubmit".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook UserPromptSubmit",
                            "timeout": config.user_prompt_submit.timeout
                        }
                    ]
                }
            ]),
        );
    }

    if config.permission_request.enabled {
        hooks.insert(
            "PermissionRequest".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook PermissionRequest",
                            "timeout": config.permission_request.timeout
                        }
                    ]
                }
            ]),
        );
    }

    // async: true - external notifications, already spawns threads for webhooks
    if config.notification.enabled {
        let matcher = config.notification.matcher.join("|");
        hooks.insert(
            "Notification".to_string(),
            serde_json::json!([
                {
                    "matcher": matcher,
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook Notification",
                            "timeout": config.notification.timeout,
                            "async": true
                        }
                    ]
                }
            ]),
        );
    }

    if config.pre_compact.enabled {
        hooks.insert(
            "PreCompact".to_string(),
            serde_json::json!([
                {
                    "hooks": [
                        {
                            "type": "command",
                            "command": "cas hook PreCompact",
                            "timeout": config.pre_compact.timeout
                        }
                    ]
                }
            ]),
        );
    }

    let mut allow_permissions = get_cas_bash_permissions();
    allow_permissions.extend(get_cas_mcp_permissions());

    serde_json::json!({
        "permissions": {
            "allow": allow_permissions
        },
        "hooks": hooks,
        "statusLine": {
            "type": "command",
            "command": "cas statusline"
        }
    })
}

/// Get suggested Bash permission patterns for CAS commands
///
/// Claude Code 2.1.0+ supports wildcard patterns like `Bash(cas :*)` to allow
/// all CAS CLI commands without individual prompts.
pub fn get_cas_bash_permissions() -> Vec<String> {
    vec![
        "Bash(cas :*)".to_string(),       // All CAS commands
        "Bash(cas task:*)".to_string(),   // Task operations
        "Bash(cas search:*)".to_string(), // Search operations
        "Bash(cas add:*)".to_string(),    // Memory operations
    ]
}

/// Get MCP tool permission patterns for CAS tools
///
/// Workers need these permissions to call mcp__cas__* tools without prompts.
pub fn get_cas_mcp_permissions() -> Vec<String> {
    vec![
        "mcp__cas__task".to_string(),
        "mcp__cas__coordination".to_string(),
        "mcp__cas__memory".to_string(),
        "mcp__cas__search".to_string(),
        "mcp__cas__rule".to_string(),
        "mcp__cas__skill".to_string(),
        "mcp__cas__spec".to_string(),
        "mcp__cas__verification".to_string(),
        "mcp__cas__system".to_string(),
        "mcp__cas__pattern".to_string(),
    ]
}

/// Configure CAS as an MCP server via .mcp.json
///
/// Creates or updates .mcp.json in the project root to register CAS.
/// This follows the Claude Code convention for project-level MCP configuration.
/// Returns Ok(true) if file was modified, Ok(false) if no changes needed.
pub fn configure_mcp_server(project_root: &Path) -> anyhow::Result<bool> {
    let mcp_json_path = project_root.join(".mcp.json");

    // Read existing content for comparison
    let existing_content = if mcp_json_path.exists() {
        std::fs::read_to_string(&mcp_json_path).ok()
    } else {
        None
    };

    // Read existing config or create new
    let mut config: serde_json::Value = existing_content
        .as_ref()
        .and_then(|c| serde_json::from_str(c).ok())
        .unwrap_or_else(|| serde_json::json!({}));

    // Ensure mcpServers object exists
    if config.get("mcpServers").is_none() {
        config["mcpServers"] = serde_json::json!({});
    }

    // Add or update CAS server config
    config["mcpServers"]["cas"] = serde_json::json!({
        "command": "cas",
        "args": ["serve"]
    });

    // Write back with pretty formatting
    let formatted = serde_json::to_string_pretty(&config)?;

    // Check if content actually changed
    if existing_content.as_ref() == Some(&formatted) {
        return Ok(false);
    }

    std::fs::write(&mcp_json_path, formatted)?;
    Ok(true)
}

/// Configure CAS as an MCP server for Codex via .codex/config.toml
///
/// Creates or updates .codex/config.toml in the project root to register CAS.
/// Returns Ok(true) if file was modified, Ok(false) if no changes needed.
pub fn configure_codex_mcp_server(project_root: &Path) -> anyhow::Result<bool> {
    let codex_dir = project_root.join(".codex");
    let config_path = codex_dir.join("config.toml");

    if !codex_dir.exists() {
        std::fs::create_dir_all(&codex_dir)?;
    }

    let existing_content = if config_path.exists() {
        Some(std::fs::read_to_string(&config_path)?)
    } else {
        None
    };

    let mut config: toml::Value = match existing_content.as_ref() {
        Some(content) => toml::from_str(content)
            .map_err(|e| anyhow::anyhow!("Failed to parse .codex/config.toml: {e}"))?,
        None => toml::Value::Table(Map::new()),
    };

    let root = config
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("config.toml is not a table"))?;

    let mcp_servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(Map::new()));
    let mcp_servers = mcp_servers
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("mcp_servers is not a table"))?;

    let mut target_key = None;
    if mcp_servers.contains_key("cas") {
        target_key = Some("cas".to_string());
    } else {
        for (key, value) in mcp_servers.iter() {
            if let Some(entry) = value.as_table() {
                if entry.get("command") == Some(&toml::Value::String("cas".to_string())) {
                    target_key = Some(key.clone());
                    break;
                }
            }
        }
    }

    let key = target_key.unwrap_or_else(|| "cas".to_string());
    let mut changed = false;

    match mcp_servers.get_mut(&key) {
        Some(entry) => {
            let entry = entry
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("mcp_servers.{key} is not a table"))?;

            let desired_command = toml::Value::String("cas".to_string());
            if entry.get("command") != Some(&desired_command) {
                entry.insert("command".to_string(), desired_command);
                changed = true;
            }

            let desired_args = toml::Value::Array(vec![toml::Value::String("serve".to_string())]);
            if entry.get("args") != Some(&desired_args) {
                entry.insert("args".to_string(), desired_args);
                changed = true;
            }

            // Ensure Codex fallback session is enabled by default
            let env = entry
                .entry("env")
                .or_insert_with(|| toml::Value::Table(Map::new()));
            let env = env
                .as_table_mut()
                .ok_or_else(|| anyhow::anyhow!("mcp_servers.{key}.env is not a table"))?;
            if !env.contains_key("CAS_CODEX_FALLBACK_SESSION") {
                env.insert(
                    "CAS_CODEX_FALLBACK_SESSION".to_string(),
                    toml::Value::String("1".to_string()),
                );
                changed = true;
            }
        }
        None => {
            let mut entry = Map::new();
            entry.insert(
                "command".to_string(),
                toml::Value::String("cas".to_string()),
            );
            entry.insert(
                "args".to_string(),
                toml::Value::Array(vec![toml::Value::String("serve".to_string())]),
            );
            let mut env = Map::new();
            env.insert(
                "CAS_CODEX_FALLBACK_SESSION".to_string(),
                toml::Value::String("1".to_string()),
            );
            entry.insert("env".to_string(), toml::Value::Table(env));
            mcp_servers.insert(key, toml::Value::Table(entry));
            changed = true;
        }
    }

    if !changed {
        return Ok(false);
    }

    let formatted = toml::to_string_pretty(&config)?;
    if existing_content.as_ref() == Some(&formatted) {
        return Ok(false);
    }

    std::fs::write(&config_path, formatted)?;
    Ok(true)
}
