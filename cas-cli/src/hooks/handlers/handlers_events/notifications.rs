use crate::hooks::handlers::*;
use super::pre_tool::FACTORY_AUTO_APPROVE_TOOLS;

pub fn handle_permission_request(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    let tool_name = match &input.tool_name {
        Some(name) => name.as_str(),
        None => return Ok(HookOutput::empty()),
    };

    // ========================================================================
    // FACTORY PERMISSION-REQUEST AUTO-APPROVE — belt #3 (cas-7f33)
    //
    // In some Claude Code 2.1.x builds, PreToolUse `permissionDecision:"allow"`
    // does not pre-empt team-mode leader-escalation cleanly, and the decision
    // flow falls through to a PermissionRequest notification. The UG9 self-
    // check bug (see pre_tool.rs) then escalates to the team leader for
    // every filesystem write — self-deadlock for supervisors.
    //
    // Scope and structural asymmetry with pre_tool.rs (INTENTIONAL):
    //   - The PreToolUse hoist is gated on `cas_root.is_none()` because a
    //     second copy exists below the protection block to handle the
    //     `cas_root=Some` case after `.env`/credential denies have run.
    //   - This handler has NO protection gates today (pre-cas-7f33 code
    //     only did task-context/lease-based auto-approve). There is no
    //     deny-first invariant to preserve, so the belt fires
    //     unconditionally for factory agents — both `cas_root=None` and
    //     `cas_root=Some` need the deadlock bypass whenever the
    //     PermissionRequest path is reached.
    //   - Trade-off: the prior lease/task-context auto-approve (below)
    //     is short-circuited for factory agents on the 7 allowlisted
    //     tools. Factory agents are already privileged via the PreToolUse
    //     auto-approve; this does not materially change their write surface.
    //   - If a future contributor adds a protection gate (e.g., `.env`
    //     deny) to this handler, it MUST be hoisted above this belt or
    //     factory agents will bypass it. See FACTORY_AUTO_APPROVE_TOOLS
    //     doc comment in pre_tool.rs for the full consumer list.
    //
    // Runs BEFORE the cas_root check because CAS_AGENT_ROLE is pure env,
    // no store access required.
    // ========================================================================
    let is_factory_agent = std::env::var("CAS_AGENT_ROLE").is_ok();
    if is_factory_agent && FACTORY_AUTO_APPROVE_TOOLS.contains(&tool_name) {
        eprintln!(
            "cas: PermissionRequest factory auto-approve for {tool_name}"
        );
        return Ok(HookOutput::with_permission_request(
            "allow",
            &format!(
                "Factory agent auto-approve ({tool_name}) — bypasses Claude Code team-mode leader-escalation deadlock (UG9 bug)"
            ),
        ));
    }

    // Check if CAS is initialized
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    // Get the agent's claimed tasks
    let agent_store = open_agent_store(cas_root)?;
    let task_store = open_task_store(cas_root)?;

    // Agent ID is the session_id
    let agent = match agent_store.get(&input.session_id) {
        Ok(a) => a,
        Err(_) => return Ok(HookOutput::empty()),
    };

    // Get agent's active task leases
    let leases = agent_store.list_agent_leases(&agent.id).unwrap_or_default();
    if leases.is_empty() {
        return Ok(HookOutput::empty());
    }

    // Get file path if applicable
    let file_path = input
        .tool_input
        .as_ref()
        .and_then(|ti| ti.get("file_path").and_then(|v| v.as_str()));

    // Check if the operation relates to any claimed task
    for lease in &leases {
        if let Ok(task) = task_store.get(&lease.task_id) {
            // For now, auto-approve Read/Glob/Grep for any claimed task
            // This allows exploration without friction
            if matches!(
                tool_name,
                "Read" | "Glob" | "Grep" | "WebSearch" | "WebFetch"
            ) {
                eprintln!(
                    "cas: PermissionRequest auto-approved {} for task {}",
                    tool_name, &task.id
                );
                return Ok(HookOutput::with_permission_request(
                    "allow",
                    &format!("Agent has claimed task: {}", task.title),
                ));
            }

            // For Write/Edit, check if path is mentioned in task notes/description
            if let Some(path) = file_path {
                let path_filename = std::path::Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(path);

                // Check if the file is mentioned in task context
                let task_context = format!("{} {} {}", task.title, task.description, task.notes);
                if task_context.contains(path_filename) {
                    eprintln!(
                        "cas: PermissionRequest auto-approved {tool_name} on {path} (task context match)"
                    );
                    return Ok(HookOutput::with_permission_request(
                        "allow",
                        &format!("File mentioned in claimed task: {}", task.title),
                    ));
                }
            }
        }
    }

    // No auto-approval - let Claude ask the user
    Ok(HookOutput::empty())
}

// ============================================================================
// Notification Hook Handler
// ============================================================================

/// Handle Notification hook - external alerts
///
/// This hook fires on various notification events and can:
/// 1. Send desktop notifications (via terminal-notifier on macOS)
/// 2. Optional webhook support for Slack/Discord
/// 3. Log notification patterns for analytics
///
/// Supported matchers: permission_prompt, idle_prompt, auth_success
pub fn handle_notification(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    // Check if CAS is initialized
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    // Load config for notification settings
    let config = Config::load(cas_root).unwrap_or_default();
    let notification_config = match config.notifications {
        Some(nc) if nc.enabled => nc,
        _ => return Ok(HookOutput::empty()),
    };

    // Get the notification type from hook event name
    let notification_type = input.hook_event_name.as_str();

    // Check matchers to see if we should notify
    let should_notify = match notification_type {
        "permission_prompt" => notification_config.on_permission_prompt,
        "idle_prompt" => notification_config.on_idle_prompt,
        "auth_success" => notification_config.on_auth_success,
        _ => false,
    };

    if !should_notify {
        return Ok(HookOutput::empty());
    }

    // Build notification message
    let message = match notification_type {
        "permission_prompt" => format!(
            "Claude needs permission for: {}",
            input.tool_name.as_deref().unwrap_or("unknown action")
        ),
        "idle_prompt" => "Claude is waiting for your input".to_string(),
        "auth_success" => "Claude Code authentication successful".to_string(),
        _ => format!("Claude Code: {notification_type}"),
    };

    // Send desktop notification (macOS)
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                &format!(
                    "display notification \"{}\" with title \"Claude Code\"",
                    message.replace('"', "\\\"")
                ),
            ])
            .spawn();
    }

    // Linux notifications via notify-send
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args(["Claude Code", &message])
            .spawn();
    }

    // Optional webhook support
    if let Some(ref webhook_url) = notification_config.webhook_url {
        // Send async notification via webhook (fire and forget)
        let url = webhook_url.clone();
        let msg = message.clone();
        std::thread::spawn(move || {
            let _ = send_webhook_notification(&url, &msg);
        });
    }

    // Log the notification event
    eprintln!("cas: Notification sent: {message}");

    Ok(HookOutput::empty())
}

/// Send a webhook notification (blocking, for use in spawned thread)
pub fn send_webhook_notification(url: &str, message: &str) -> Result<(), MemError> {
    // Simple HTTP POST with JSON payload
    let payload = serde_json::json!({
        "text": message,
        "username": "Claude Code",
    });

    // Use curl for simplicity (available on most systems)
    let output = std::process::Command::new("curl")
        .args([
            "-s",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-d",
            &payload.to_string(),
            url,
        ])
        .output();

    match output {
        Ok(_) => Ok(()),
        Err(e) => Err(MemError::Other(format!("Webhook failed: {e}"))),
    }
}

// ============================================================================
// PreCompact Hook Handler
// ============================================================================

/// Handle PreCompact hook - preserve critical context before compaction
///
/// This hook fires before Claude's context is compacted and can:
/// 1. Inject high-importance memories before compaction
/// 2. Preserve active task context and decisions
/// 3. Support both manual (/compact) and auto triggers
/// 4. Optionally generate pre-compaction summary
///
/// Returns additionalContext with preserved memories
pub fn handle_pre_compact(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    // Check if CAS is initialized
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    let mut context_parts: Vec<String> = Vec::new();

    // 1. Inject high-importance memories (importance >= 0.7)
    let store = open_store(cas_root)?;
    let entries = store.list()?;
    let high_importance: Vec<_> = entries
        .iter()
        .filter(|e| e.importance >= 0.7 && e.entry_type != EntryType::Observation)
        .take(10)
        .collect();

    if !high_importance.is_empty() {
        context_parts.push("## Critical Memories (preserve across compaction)".to_string());
        for entry in &high_importance {
            let preview = truncate_display(&entry.content, 200);
            context_parts.push(format!("- [{}] {}", entry.id, preview));
        }
    }

    // 2. Preserve active task context
    let task_store = open_task_store(cas_root)?;
    let in_progress = task_store.list(Some(TaskStatus::InProgress))?;

    if !in_progress.is_empty() {
        context_parts.push(String::new());
        context_parts.push("## Active Tasks (preserve across compaction)".to_string());
        for task in in_progress.iter().take(5) {
            context_parts.push(format!("- [{}] {} ({})", task.id, task.title, task.status));
            if !task.notes.is_empty() {
                // Get last note line
                if let Some(last_note) = task.notes.lines().last() {
                    if last_note.len() <= 100 {
                        context_parts.push(format!("  Last note: {last_note}"));
                    }
                }
            }
        }
    }

    // 3. Get recent decisions from current session
    let session_entries = store.list_by_session(&input.session_id)?;
    let decisions: Vec<_> = session_entries
        .iter()
        .filter(|e| {
            e.observation_type == Some(ObservationType::Decision)
                || e.tags.iter().any(|t| t == "decision")
        })
        .take(5)
        .collect();

    if !decisions.is_empty() {
        context_parts.push(String::new());
        context_parts.push("## Session Decisions (preserve across compaction)".to_string());
        for decision in &decisions {
            let preview = truncate_display(&decision.content, 150);
            context_parts.push(format!("- {preview}"));
        }
    }

    // 4. Include proven rules for current file context
    let rule_store = open_rule_store(cas_root)?;
    let critical_rules = rule_store.list_critical()?;

    if !critical_rules.is_empty() {
        context_parts.push(String::new());
        context_parts.push("## Critical Rules (always apply)".to_string());
        for rule in critical_rules.iter().take(3) {
            context_parts.push(format!("- [{}] {}", rule.id, rule.preview(80)));
        }
    }

    if context_parts.is_empty() {
        return Ok(HookOutput::empty());
    }

    let context = context_parts.join("\n");
    eprintln!(
        "cas: PreCompact injecting {} chars of context",
        context.len()
    );

    // PreCompact doesn't support hookSpecificOutput in Claude Code's schema
    // (only PreToolUse, UserPromptSubmit, PostToolUse do)
    // Use systemMessage instead to inject context
    Ok(HookOutput {
        system_message: Some(context),
        ..Default::default()
    })
}
