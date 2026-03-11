//! Hook command - handle Claude Code hook events
//!
//! Reads JSON from stdin, processes the hook event, and outputs JSON to stdout.

use clap::{Parser, Subcommand};
use std::io::{self, Read};
use std::path::Path;

use crate::config::Config;
use crate::hooks::{HookInput, handle_hook};
use crate::store;
use crate::ui::components::{Formatter, Header, KeyValue, Renderable, StatusLine};
use crate::ui::theme::ActiveTheme;

use crate::cli::Cli;
use crate::cli::hook::config_gen::get_cas_hooks_config;

mod config_gen;
pub use crate::cli::hook::config_gen::{configure_codex_mcp_server, configure_mcp_server};

/// Arguments for the hook command
#[derive(Parser)]
pub struct HookArgs {
    /// Hook subcommand or event name
    #[command(subcommand)]
    pub command: HookCommand,
}

#[derive(Subcommand)]
#[command(rename_all = "verbatim")]
pub enum HookCommand {
    /// Configure Claude Code hooks in .claude/settings.json
    #[command(name = "configure")]
    Configure {
        /// Force overwrite existing hooks configuration
        #[arg(short, long)]
        force: bool,
    },
    /// Show current hooks configuration status
    #[command(name = "status")]
    Status,
    /// Handle SessionStart hook event
    SessionStart,
    /// Handle SessionEnd hook event
    SessionEnd,
    /// Handle Stop hook event
    Stop,
    /// Handle SubagentStart hook event (verification jail unjailing)
    SubagentStart,
    /// Handle SubagentStop hook event
    SubagentStop,
    /// Handle PostToolUse hook event
    PostToolUse,
    /// Handle PreToolUse hook event (auto-approval)
    PreToolUse,
    /// Handle UserPromptSubmit hook event
    UserPromptSubmit,
    /// Handle PermissionRequest hook event (smart auto-approve)
    PermissionRequest,
    /// Handle Notification hook event (external alerts)
    Notification,
    /// Handle PreCompact hook event (context preservation)
    PreCompact,
}

/// Execute the hook command
pub fn execute(args: &HookArgs, cli: &Cli) -> anyhow::Result<()> {
    match &args.command {
        HookCommand::Configure { force } => execute_configure(*force, cli),
        HookCommand::Status => execute_status(cli),
        HookCommand::SessionStart => execute_event("SessionStart", cli),
        HookCommand::SessionEnd => execute_event("SessionEnd", cli),
        HookCommand::Stop => execute_event("Stop", cli),
        HookCommand::SubagentStart => execute_event("SubagentStart", cli),
        HookCommand::SubagentStop => execute_event("SubagentStop", cli),
        HookCommand::PostToolUse => execute_event("PostToolUse", cli),
        HookCommand::PreToolUse => execute_event("PreToolUse", cli),
        HookCommand::UserPromptSubmit => execute_event("UserPromptSubmit", cli),
        HookCommand::PermissionRequest => execute_event("PermissionRequest", cli),
        HookCommand::Notification => execute_event("Notification", cli),
        HookCommand::PreCompact => execute_event("PreCompact", cli),
    }
}

/// Handle a hook event (reads JSON from stdin)
fn execute_event(event: &str, cli: &Cli) -> anyhow::Result<()> {
    // Initialize logging for hooks (best-effort, don't fail if it doesn't work)
    init_hook_logging(cli.verbose);

    // Read JSON from stdin
    let mut input_json = String::new();
    io::stdin().read_to_string(&mut input_json)?;

    // Parse the hook input
    let input: HookInput = serde_json::from_str(&input_json)
        .map_err(|e| anyhow::anyhow!("Failed to parse hook input: {e}"))?;

    // Handle the hook event
    let output = handle_hook(event, input)?;

    // Output JSON to stdout
    println!("{}", serde_json::to_string(&output)?);

    Ok(())
}

/// Initialize logging for hook execution
fn init_hook_logging(verbose: bool) {
    // Try to find CAS root and load config
    let cas_root = store::find_cas_root().ok();
    let logging_config = cas_root
        .as_ref()
        .and_then(|root| Config::load(root).ok())
        .and_then(|c| c.logging)
        .unwrap_or_default();

    // Initialize logging - ignore errors (may already be initialized)
    let _ = crate::logging::init(cas_root.as_deref(), verbose, &logging_config);
}

/// Configure Claude Code hooks
fn execute_configure(force: bool, cli: &Cli) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;

    match configure_claude_hooks(&cwd, force) {
        Ok(created) => {
            if cli.json {
                println!(r#"{{"status":"configured","created":{created}}}"#);
            } else {
                let theme = ActiveTheme::default();
                let mut stdout = io::stdout();
                let mut fmt = Formatter::stdout(&mut stdout, theme);

                if created {
                    StatusLine::success("Created .claude/settings.json with CAS hooks")
                        .render(&mut fmt)?;
                    fmt.newline()?;
                    Header::h2("Hooks configured").render(&mut fmt)?;
                    fmt.bullet("SessionStart - Injects context at session start")?;
                    fmt.bullet("Stop - Generates session summary")?;
                    fmt.bullet("PostToolUse - Captures Write/Edit/Bash activity")?;
                    fmt.bullet("StatusLine - Shows CAS status in Claude Code footer")?;
                    fmt.newline()?;
                    Header::h2("Permissions configured (Claude Code 2.1.0+)").render(&mut fmt)?;
                    fmt.bullet("Bash(cas :*) - All CAS commands auto-allowed")?;
                    fmt.newline()?;
                    StatusLine::info("Restart your Claude Code session for hooks to take effect.")
                        .render(&mut fmt)?;
                } else {
                    StatusLine::success("Updated .claude/settings.json with CAS hooks")
                        .render(&mut fmt)?;
                    fmt.newline()?;
                    StatusLine::info("Restart your Claude Code session for hooks to take effect.")
                        .render(&mut fmt)?;
                }
            }
        }
        Err(e) => {
            if cli.json {
                println!(r#"{{"status":"error","message":"{e}"}}"#);
            } else {
                anyhow::bail!("Failed to configure hooks: {e}");
            }
        }
    }

    Ok(())
}

/// Show hooks configuration status
fn execute_status(cli: &Cli) -> anyhow::Result<()> {
    let cwd = std::env::current_dir()?;
    let claude_dir = cwd.join(".claude");
    let settings_path = claude_dir.join("settings.json");

    if !settings_path.exists() {
        if cli.json {
            println!(r#"{{"configured":false,"reason":"no_settings_file"}}"#);
        } else {
            let theme = ActiveTheme::default();
            let mut stdout = io::stdout();
            let mut fmt = Formatter::stdout(&mut stdout, theme);
            StatusLine::error("No .claude/settings.json found").render(&mut fmt)?;
            fmt.newline()?;
            fmt.info("Run 'cas hook configure' to set up Claude Code hooks.")?;
        }
        return Ok(());
    }

    // Read and parse settings
    let content = std::fs::read_to_string(&settings_path)?;
    let settings: serde_json::Value = serde_json::from_str(&content)?;

    let has_hooks = settings.get("hooks").is_some();

    let check_hook = |name: &str| -> bool {
        settings
            .pointer(&format!("/hooks/{name}"))
            .map(|v| v.is_array() && !v.as_array().unwrap().is_empty())
            .unwrap_or(false)
    };

    let has_session_start = check_hook("SessionStart");
    let has_session_end = check_hook("SessionEnd");
    let has_stop = check_hook("Stop");
    let has_subagent_stop = check_hook("SubagentStop");
    let has_post_tool = check_hook("PostToolUse");
    let has_pre_tool = check_hook("PreToolUse");
    let has_user_prompt = check_hook("UserPromptSubmit");
    let has_permission_request = check_hook("PermissionRequest");
    let has_notification = check_hook("Notification");
    let has_pre_compact = check_hook("PreCompact");

    if cli.json {
        println!(
            r#"{{"configured":{has_hooks},"session_start":{has_session_start},"session_end":{has_session_end},"stop":{has_stop},"subagent_stop":{has_subagent_stop},"post_tool_use":{has_post_tool},"pre_tool_use":{has_pre_tool},"user_prompt_submit":{has_user_prompt},"permission_request":{has_permission_request},"notification":{has_notification},"pre_compact":{has_pre_compact}}}"#
        );
    } else {
        let theme = ActiveTheme::default();
        let mut stdout = io::stdout();
        let mut fmt = Formatter::stdout(&mut stdout, theme);

        Header::h1("Claude Code Hooks Status").render(&mut fmt)?;

        let status_str = |configured: bool| -> &'static str {
            if configured {
                "configured"
            } else {
                "not configured"
            }
        };

        KeyValue::new()
            .add("SessionStart", status_str(has_session_start))
            .add("SessionEnd", status_str(has_session_end))
            .add("Stop", status_str(has_stop))
            .add("SubagentStop", status_str(has_subagent_stop))
            .add("PostToolUse", status_str(has_post_tool))
            .add("PreToolUse", status_str(has_pre_tool))
            .add("UserPromptSubmit", status_str(has_user_prompt))
            .add("PermissionRequest", status_str(has_permission_request))
            .add("Notification", status_str(has_notification))
            .add("PreCompact", status_str(has_pre_compact))
            .render(&mut fmt)?;

        let all_configured = has_session_start
            && has_session_end
            && has_stop
            && has_subagent_stop
            && has_post_tool
            && has_pre_tool
            && has_user_prompt
            && has_permission_request
            && has_notification
            && has_pre_compact;
        if !all_configured {
            fmt.newline()?;
            fmt.info("Run 'cas hook configure' to set up missing hooks.")?;
        }
    }

    Ok(())
}

/// Configure Claude Code hooks in .claude/settings.json
///
/// This function creates or updates the settings.json file to include
/// CAS hooks for SessionStart, Stop, and PostToolUse events.
///
/// Returns Ok(true) if file was created, Ok(false) if updated.
pub fn configure_claude_hooks(project_root: &Path, force: bool) -> anyhow::Result<bool> {
    let claude_dir = project_root.join(".claude");
    let settings_path = claude_dir.join("settings.json");
    let cas_dir = project_root.join(".cas");

    // Create .claude directory if needed
    if !claude_dir.exists() {
        std::fs::create_dir_all(&claude_dir)?;
    }

    // Load hook config from .cas/config.toml (or defaults)
    let hook_config = if cas_dir.exists() {
        Config::load(&cas_dir)
            .map(|c| c.hooks())
            .unwrap_or_default()
    } else {
        crate::config::HookConfig::default()
    };

    let cas_hooks = get_cas_hooks_config(&hook_config);

    let created = if settings_path.exists() && !force {
        // Merge with existing settings
        let content = std::fs::read_to_string(&settings_path)?;
        let mut settings: serde_json::Value = serde_json::from_str(&content)?;

        let settings_obj = settings
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("settings.json is not an object"))?;

        // Get or create hooks object
        let hooks = settings_obj
            .entry("hooks")
            .or_insert_with(|| serde_json::json!({}));

        let hooks_obj = hooks
            .as_object_mut()
            .ok_or_else(|| anyhow::anyhow!("hooks is not an object"))?;

        // Merge CAS hooks (don't overwrite existing non-CAS hooks)
        let cas_hooks_obj = cas_hooks.as_object().unwrap();
        for (key, value) in cas_hooks_obj.get("hooks").unwrap().as_object().unwrap() {
            hooks_obj.insert(key.clone(), value.clone());
        }

        // Add statusLine configuration (overwrite if exists - CAS owns this)
        if let Some(status_line) = cas_hooks_obj.get("statusLine") {
            settings_obj.insert("statusLine".to_string(), status_line.clone());
        }

        // Merge CAS Bash permissions (Claude Code 2.1.0+ wildcard patterns)
        if let Some(cas_permissions) = cas_hooks_obj.get("permissions") {
            let permissions = settings_obj
                .entry("permissions")
                .or_insert_with(|| serde_json::json!({}));
            if let Some(permissions_obj) = permissions.as_object_mut() {
                let allow = permissions_obj
                    .entry("allow")
                    .or_insert_with(|| serde_json::json!([]));
                if let Some(allow_arr) = allow.as_array_mut() {
                    // Add CAS permissions if not already present
                    if let Some(cas_allow) = cas_permissions.get("allow").and_then(|a| a.as_array())
                    {
                        for pattern in cas_allow {
                            if !allow_arr.contains(pattern) {
                                allow_arr.push(pattern.clone());
                            }
                        }
                    }
                }
            }
        }

        // Add worktree directory to additionalDirectories if worktrees are enabled
        let cas_root = project_root.join(".cas");
        if let Ok(config) = Config::load(&cas_root) {
            let worktrees_config = config.worktrees();
            if worktrees_config.enabled {
                // Compute worktree base path
                let base = worktrees_config.base_path.replace(
                    "{project}",
                    project_root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project"),
                );

                let worktree_path = if base.starts_with('/') {
                    base
                } else {
                    // Relative path - resolve from project root's parent
                    project_root
                        .parent()
                        .unwrap_or(project_root)
                        .join(&base)
                        .to_string_lossy()
                        .to_string()
                };

                let permissions = settings_obj
                    .entry("permissions")
                    .or_insert_with(|| serde_json::json!({}));
                if let Some(permissions_obj) = permissions.as_object_mut() {
                    let additional_dirs = permissions_obj
                        .entry("additionalDirectories")
                        .or_insert_with(|| serde_json::json!([]));
                    if let Some(dirs_arr) = additional_dirs.as_array_mut() {
                        let path_value = serde_json::Value::String(worktree_path.clone());
                        if !dirs_arr.contains(&path_value) {
                            dirs_arr.push(path_value);
                        }
                    }
                }
            }
        }

        // Write back
        let output = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&settings_path, output)?;
        false
    } else {
        // Create new settings file
        let mut settings = cas_hooks.clone();

        // Add worktree directory if worktrees are enabled
        let cas_root = project_root.join(".cas");
        if let Ok(config) = Config::load(&cas_root) {
            let worktrees_config = config.worktrees();
            if worktrees_config.enabled {
                // Compute worktree base path
                let base = worktrees_config.base_path.replace(
                    "{project}",
                    project_root
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("project"),
                );

                let worktree_path = if base.starts_with('/') {
                    base
                } else {
                    project_root
                        .parent()
                        .unwrap_or(project_root)
                        .join(&base)
                        .to_string_lossy()
                        .to_string()
                };

                if let Some(settings_obj) = settings.as_object_mut() {
                    let permissions = settings_obj
                        .entry("permissions")
                        .or_insert_with(|| serde_json::json!({}));
                    if let Some(permissions_obj) = permissions.as_object_mut() {
                        permissions_obj.insert(
                            "additionalDirectories".to_string(),
                            serde_json::json!([worktree_path]),
                        );
                    }
                }
            }
        }

        let output = serde_json::to_string_pretty(&settings)?;
        std::fs::write(&settings_path, output)?;
        true
    };

    Ok(created)
}

#[cfg(test)]
#[path = "hook_tests/tests.rs"]
mod tests;
