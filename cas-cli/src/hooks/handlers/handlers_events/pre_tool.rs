use crate::harness_policy::worker_harness_from_env;
use crate::hooks::handlers::*;

// PreToolUse Hook Handler
// ============================================================================

/// Handle PreToolUse hook - rule-based auto-approval
///
/// This hook fires BEFORE a tool is executed and can:
/// 1. Auto-approve safe tools based on proven rules with path matching
/// 2. Block or warn for protected files/directories
/// 3. Modify tool parameters via updatedInput
///
/// Returns permission_decision: "allow" | "deny" | null (ask user)
pub fn handle_pre_tool_use(
    input: &HookInput,
    cas_root: Option<&Path>,
) -> Result<HookOutput, MemError> {
    let tool_name = match &input.tool_name {
        Some(name) => name.as_str(),
        None => return Ok(HookOutput::empty()),
    };

    // Check if CAS is initialized
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    // Compute current agent's task IDs (via leases) once for all jail checks.
    // This prevents cross-agent jail contamination where Agent A's pending tasks
    // block Agent B in a different session.
    let current_agent_id = current_agent_id(input);
    let agent_task_ids: std::collections::HashSet<String> = open_agent_store(cas_root)
        .ok()
        .and_then(|store| store.list_agent_leases(&current_agent_id).ok())
        .map(|leases| leases.into_iter().map(|l| l.task_id).collect())
        .unwrap_or_default();

    // ========================================================================
    // FACTORY MODE: Block SendMessage — agents must use coordination message
    //
    // In factory mode, agents communicate through CAS coordination (push-based
    // via the Director/TUI). The built-in SendMessage tool bypasses this system
    // and causes messages to be lost. Redirect to mcp__cas__coordination.
    // ========================================================================
    let is_factory_agent = std::env::var("CAS_AGENT_ROLE").is_ok();
    if is_factory_agent && tool_name == "SendMessage" {
        return Ok(HookOutput::with_permission_decision(
            "PreToolUse",
            "deny",
            "🚫 SendMessage is disabled in factory mode.\n\n\
            Use CAS coordination instead:\n\
            mcp__cas__coordination action=message target=<agent-name> message=\"...\" summary=\"<brief summary>\"\n\n\
            This ensures messages are routed through the factory Director.",
        ));
    }

    // ========================================================================
    // VERIFICATION JAIL: Block all tools except task-verifier when pending
    //
    // When a task has pending_verification=true, block all tools except:
    // 1. Task tool spawning task-verifier - unjails by clearing pending_verification
    // 2. mcp__cas__verification - allows recording verification results
    //
    // The unjail happens in PreToolUse when Task(task-verifier) is detected.
    // A marker file is also written as backup for edge cases.
    //
    // Only jail the agent that owns the tasks (via leases), not all agents.
    // ========================================================================
    // Supervisors are exempt from verification jail — their job is coordination
    let is_supervisor = crate::harness_policy::is_supervisor_from_env();
    let worker_supports_subagents = worker_harness_from_env().capabilities().supports_subagents;
    // Verification jail is only relevant when worker harness supports subagents.
    if worker_supports_subagents && !is_supervisor {
        if let Ok(task_store) = open_task_store(cas_root) {
            if let Ok(tasks) = task_store.list(None) {
                // Only consider tasks that:
                // 1. Have pending_verification=true AND
                // 2. Either:
                //    a. The current agent has an active lease on them (regular tasks), OR
                //    b. The current agent is the epic_verification_owner (epic tasks)
                let pending_tasks: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
                        if !t.pending_verification {
                            return false;
                        }
                        // For epics with epic_verification_owner set, jail that owner
                        if t.task_type == TaskType::Epic {
                            if let Some(ref owner) = t.epic_verification_owner {
                                return owner == &current_agent_id;
                            }
                        }
                        // For regular tasks (or epics without owner), use lease ownership
                        agent_task_ids.contains(&t.id)
                            || t.assignee
                                .as_ref()
                                .map(|a| a == &current_agent_id)
                                .unwrap_or(false)
                    })
                    .collect();

                // Check for unjail marker file (backup mechanism)
                // This marker indicates task-verifier is running and all tools should be allowed
                let marker_path = cas_root.join(".verifier_unjail_marker");
                let mut jail_cleared_via_marker = false;

                // Log verification jail state for debugging
                let task_ids_for_log: Vec<_> =
                    pending_tasks.iter().map(|t| t.id.as_str()).collect();
                debug!(
                    tool = tool_name,
                    agent = &current_agent_id[..8.min(current_agent_id.len())],
                    pending_tasks = task_ids_for_log.join(", ").as_str(),
                    marker_exists = marker_path.exists(),
                    "[VERIFICATION JAIL] checking jail state"
                );

                if marker_path.exists() {
                    if let Ok(contents) = std::fs::read_to_string(&marker_path) {
                        let marker_session = contents
                            .lines()
                            .find_map(|line| line.strip_prefix("session="))
                            .map(|s| s.trim());

                        debug!(
                            marker_session = ?marker_session,
                            current_agent = &current_agent_id[..8.min(current_agent_id.len())],
                            "[VERIFICATION JAIL] marker file found"
                        );

                        if marker_session == Some(current_agent_id.as_str()) {
                            // Clear jail via marker - verifier is running for this agent
                            for task in &pending_tasks {
                                let mut task_to_update = (*task).clone();
                                task_to_update.pending_verification = false;
                                task_to_update.updated_at = chrono::Utc::now();
                                let _ = task_store.update(&task_to_update);
                            }
                            // NOTE: Don't remove marker file here - keep it until verifier completes
                            // The marker indicates verifier is actively running
                            jail_cleared_via_marker = true;
                            info!(
                                "[VERIFICATION JAIL] jail cleared via marker file (task-verifier running)"
                            );
                        } else {
                            warn!(
                                marker_session = ?marker_session,
                                current_agent = current_agent_id.as_str(),
                                "[VERIFICATION JAIL] marker session MISMATCH"
                            );
                        }
                    }
                }

                // Skip jail check if marker cleared it (verifier is running)
                if !jail_cleared_via_marker && !pending_tasks.is_empty() {
                    // Check if this is Task tool spawning task-verifier
                    let is_verifier_agent = if tool_name == "Task" {
                        let subagent_type = input
                            .tool_input
                            .as_ref()
                            .and_then(|ti| ti.get("subagent_type").and_then(|v| v.as_str()));
                        debug!(
                            subagent_type = ?subagent_type,
                            "[VERIFICATION JAIL] Task tool detected"
                        );
                        subagent_type == Some("task-verifier")
                    } else {
                        false
                    };

                    // Allow verification tool for recording results (CAS + Codex alias)
                    let is_verification_tool = tool_name == "mcp__cas__verification"
                        || tool_name == "mcp__cs__verification";

                    debug!(
                        is_verifier_agent = is_verifier_agent,
                        is_verification_tool = is_verification_tool,
                        tool = tool_name,
                        "[VERIFICATION JAIL] evaluating tool"
                    );

                    if is_verifier_agent {
                        // Write unjail marker as backup
                        let marker_content = format!(
                            "session={}\ntimestamp={}",
                            current_agent_id,
                            chrono::Utc::now()
                        );
                        let _ = std::fs::write(&marker_path, &marker_content);

                        // Clear jail directly - subagent will see cleared flag
                        let task_ids: Vec<_> =
                            pending_tasks.iter().map(|t| t.id.as_str()).collect();
                        for task in &pending_tasks {
                            let mut task_to_update = (*task).clone();
                            task_to_update.pending_verification = false;
                            task_to_update.updated_at = chrono::Utc::now();
                            let _ = task_store.update(&task_to_update);

                            // Emit VerificationStarted event for task lifecycle tracking
                            #[cfg(feature = "mcp-server")]
                            {
                                let event = crate::mcp::socket::DaemonEvent::WorkerActivity {
                                    session_id: input.session_id.clone(),
                                    event_type: "verification_started".to_string(),
                                    description: format!("Verifying: {}", task.title),
                                    entity_id: Some(task.id.clone()),
                                };
                                let _ = crate::mcp::socket::send_event(cas_root, &event);
                            }
                        }
                        info!(
                            tasks = task_ids.join(", ").as_str(),
                            "[VERIFICATION JAIL] ALLOWING task-verifier spawn, unjailing tasks"
                        );
                    } else if is_verification_tool {
                        // Allow verification tool through - this records verification directly.
                        info!("[VERIFICATION JAIL] ALLOWING verification MCP tool through");
                    } else {
                        let task_ids: Vec<_> =
                            pending_tasks.iter().map(|t| t.id.as_str()).collect();
                        let task_list = task_ids.join(", ");

                        warn!(
                            tool = tool_name,
                            pending_tasks = task_list.as_str(),
                            "[VERIFICATION JAIL] BLOCKING tool"
                        );

                        // Use task-verifier for both tasks and epics (epics must set verification_type=epic)
                        let has_epic = pending_tasks.iter().any(|t| t.task_type == TaskType::Epic);
                        let verifier_name = "task-verifier";
                        let epic_note = if has_epic {
                            "\nFor epics: the verifier MUST record verification_type=epic."
                        } else {
                            ""
                        };

                        return Ok(HookOutput::with_permission_decision(
                            "PreToolUse",
                            "deny",
                            &format!(
                                "🔒 VERIFICATION JAIL: Task(s) {task_list} require verification before you can continue.\n\n\
                            You MUST spawn the '{verifier_name}' agent to review and verify the work.{epic_note}\n\n\
                            Example: Use the Task tool with subagent_type=\"{verifier_name}\" and prompt describing the task to verify."
                            ),
                        ));
                    }
                }
            }
        }
    }

    // ========================================================================
    // WORKTREE MERGE JAIL: Block all tools except worktree-merger when pending
    //
    // When a task has pending_worktree_merge=true, block all tools except:
    // 1. Task tool spawning worktree-merger - unjails by clearing pending_worktree_merge
    //
    // The unjail happens in PreToolUse when Task(worktree-merger) is detected.
    //
    // NOTE: This entire system is EXPERIMENTAL and only active when worktrees.enabled=true
    //
    // Only jail the agent that owns the tasks (via leases), not all agents.
    // ========================================================================
    let worktrees_enabled = Config::load(cas_root)
        .map(|c| c.worktrees_enabled())
        .unwrap_or(false);

    // Factory workers manage their own worktrees — skip CAS worktree enforcement
    // to avoid conflicting redirects (factory uses per-worker worktrees, CAS uses per-epic)
    let is_factory_worker_for_wt = std::env::var("CAS_AGENT_ROLE")
        .map(|role| role.to_lowercase() == "worker")
        .unwrap_or(false);

    if worktrees_enabled && !is_factory_worker_for_wt {
        if let Ok(task_store) = open_task_store(cas_root) {
            if let Ok(tasks) = task_store.list(None) {
                // Only consider tasks the current agent owns (reuses agent_task_ids from above)
                let pending_merge_tasks: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
                        if !t.pending_worktree_merge {
                            return false;
                        }
                        agent_task_ids.contains(&t.id)
                            || t.assignee
                                .as_ref()
                                .map(|a| a == &current_agent_id)
                                .unwrap_or(false)
                    })
                    .collect();

                if !pending_merge_tasks.is_empty() {
                    // Check if this is Task tool spawning worktree-merger
                    let is_worktree_merger = if tool_name == "Task" {
                        input
                            .tool_input
                            .as_ref()
                            .and_then(|ti| ti.get("subagent_type").and_then(|v| v.as_str()))
                            .map(|st| st == "worktree-merger")
                            .unwrap_or(false)
                    } else {
                        false
                    };

                    if is_worktree_merger {
                        // Clear jail - worktree-merger agent will handle the merge
                        let task_ids: Vec<_> =
                            pending_merge_tasks.iter().map(|t| t.id.as_str()).collect();
                        for task in &pending_merge_tasks {
                            let mut task_to_update = (*task).clone();
                            task_to_update.pending_worktree_merge = false;
                            task_to_update.updated_at = chrono::Utc::now();
                            let _ = task_store.update(&task_to_update);
                        }
                        eprintln!(
                            "cas: Unjailing for worktree-merger (tasks: {})",
                            task_ids.join(", ")
                        );
                    } else {
                        let task_ids: Vec<_> =
                            pending_merge_tasks.iter().map(|t| t.id.as_str()).collect();
                        let task_list = task_ids.join(", ");

                        return Ok(HookOutput::with_permission_decision(
                            "PreToolUse",
                            "deny",
                            &format!(
                                "🔒 WORKTREE MERGE JAIL: Task(s) {task_list} require worktree merge before you can continue.\n\n\
                            You MUST spawn the 'worktree-merger' agent to merge and clean up the worktree.\n\n\
                            Example: Use the Task tool with subagent_type=\"worktree-merger\" and prompt describing the task to merge."
                            ),
                        ));
                    }
                }
            }
        }

        // ========================================================================
        // WORKTREE PATH ENFORCEMENT: Redirect file ops to worktree when applicable
        //
        // When an agent is working on a task that belongs to an epic with a worktree,
        // block file operations in the main repo and redirect to the worktree.
        // This ensures isolation between concurrent agents working on different epics.
        // ========================================================================
        let file_tools = ["Read", "Write", "Edit", "Glob", "Grep", "Bash"];
        if file_tools.iter().any(|t| tool_name.eq_ignore_ascii_case(t)) {
            // Get file path from tool input
            let tool_file_path = input.tool_input.as_ref().and_then(|ti| {
                ti.get("file_path")
                    .or_else(|| ti.get("path"))
                    .and_then(|v| v.as_str())
            });

            if let Some(file_path) = tool_file_path {
                // Check if agent has tasks in epics with worktrees
                if let Ok(agent_store) = open_agent_store(cas_root) {
                    if let Ok(task_store) = open_task_store(cas_root) {
                        if let Ok(leases) = agent_store.list_agent_leases(&current_agent_id) {
                            for lease in &leases {
                                if let Ok(task) = task_store.get(&lease.task_id) {
                                    // Check if this task belongs to an epic with a worktree
                                    if let Ok(deps) = task_store.get_dependencies(&task.id) {
                                        for dep in &deps {
                                            if dep.dep_type == DependencyType::ParentChild {
                                                if let Ok(parent) = task_store.get(&dep.to_id) {
                                                    if parent.task_type == TaskType::Epic {
                                                        if let Some(ref worktree_id) =
                                                            parent.worktree_id
                                                        {
                                                            // Epic has a worktree - check if file is in main repo
                                                            if let Ok(wt_store) =
                                                                open_worktree_store(cas_root)
                                                            {
                                                                if let Ok(worktree) =
                                                                    wt_store.get(worktree_id)
                                                                {
                                                                    let worktree_path = worktree
                                                                        .path
                                                                        .to_string_lossy();
                                                                    let main_repo =
                                                                        input.cwd.clone();

                                                                    // If file is in main repo but NOT in worktree, block
                                                                    let file_in_main = file_path
                                                                        .starts_with(&main_repo);
                                                                    let file_in_worktree =
                                                                        file_path.starts_with(
                                                                            worktree_path.as_ref(),
                                                                        );

                                                                    if file_in_main
                                                                        && !file_in_worktree
                                                                    {
                                                                        // Calculate the equivalent path in worktree
                                                                        let relative_path =
                                                                            file_path
                                                                                .strip_prefix(
                                                                                    &main_repo,
                                                                                )
                                                                                .unwrap_or(
                                                                                    file_path,
                                                                                )
                                                                                .trim_start_matches(
                                                                                    '/',
                                                                                );
                                                                        let suggested_path = format!(
                                                                            "{worktree_path}/{relative_path}"
                                                                        );

                                                                        return Ok(HookOutput::with_permission_decision(
                                                                        "PreToolUse",
                                                                        "deny",
                                                                        &format!(
                                                                            "🌳 WORKTREE REDIRECT: You're working on epic [{}] \"{}\" which has a dedicated worktree.\n\n\
                                                                            ❌ Blocked: {}\n\
                                                                            ✅ Use instead: {}\n\n\
                                                                            All file operations for this epic should happen in the worktree directory:\n\
                                                                            📁 {}",
                                                                            parent.id,
                                                                            parent.title,
                                                                            file_path,
                                                                            suggested_path,
                                                                            worktree_path
                                                                        ),
                                                                    ));
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // ========================================================================
                // WORKTREE LEASE CHECK: Warn if accessing a worktree locked by another agent
                //
                // If the file path is in a worktree directory and another agent holds
                // the lease, warn the user (coordination-level, not blocking)
                // ========================================================================
                if let Ok(wt_store) = open_worktree_store(cas_root) {
                    if let Ok(agent_store) = open_agent_store(cas_root) {
                        // Get all active worktrees and check if file is in any of them
                        if let Ok(worktrees) = wt_store.list_active() {
                            for worktree in worktrees {
                                let worktree_path_str = worktree.path.to_string_lossy();
                                if file_path.starts_with(worktree_path_str.as_ref()) {
                                    // File is in this worktree - check the lease
                                    if let Ok(Some(lease)) =
                                        agent_store.get_worktree_lease(&worktree.id)
                                    {
                                        if lease.agent_id != current_agent_id && lease.is_valid() {
                                            // Another agent holds the lease - warn but don't block
                                            eprintln!(
                                                "⚠️  WORKTREE LEASE: {} is locked by agent {} (expires in {}s)",
                                                worktree.path.display(),
                                                lease.agent_id,
                                                lease.remaining_secs()
                                            );
                                        }
                                    }
                                    break; // Found the worktree, no need to check others
                                }
                            }
                        }
                    }
                }
            }
        }
    } // End of worktrees_enabled block

    // Get file path from tool input (if applicable)
    let file_path = input
        .tool_input
        .as_ref()
        .and_then(|ti| ti.get("file_path").and_then(|v| v.as_str()));

    // Load proven rules with auto-approve configuration
    let rule_store = open_rule_store(cas_root)?;
    let rules = rule_store.list_proven()?;

    // Check if any rule auto-approves this tool call
    for rule in &rules {
        if !rule.can_auto_approve() {
            continue;
        }

        // Check if this tool is in the rule's auto-approve list
        if !rule.auto_approves_tool(tool_name) {
            continue;
        }

        // If rule has path patterns, check if the file matches
        if let Some(path) = file_path {
            if rule.matches_auto_approve_path(path) {
                eprintln!(
                    "cas: PreToolUse auto-approved {} on {} via rule {}",
                    tool_name, path, &rule.id
                );
                return Ok(HookOutput::with_permission_decision(
                    "PreToolUse",
                    "allow",
                    &format!("Auto-approved by rule {}: {}", rule.id, rule.preview(50)),
                ));
            }
        } else {
            // No file path - auto-approve if tool is in safe list and rule allows it
            if Rule::SAFE_AUTO_APPROVE_TOOLS
                .iter()
                .any(|t| t.eq_ignore_ascii_case(tool_name))
            {
                eprintln!(
                    "cas: PreToolUse auto-approved {} (safe tool) via rule {}",
                    tool_name, &rule.id
                );
                return Ok(HookOutput::with_permission_decision(
                    "PreToolUse",
                    "allow",
                    &format!("Auto-approved safe tool by rule {}", rule.id),
                ));
            }
        }
    }

    // Check for protected paths that should be blocked (configurable)
    let config = Config::load(cas_root).unwrap_or_default();
    let protection = &config.hooks().pre_tool_use.protection;

    if protection.enabled {
        if let Some(path) = file_path {
            // Block access to protected files (e.g., .env files)
            for pattern in &protection.files {
                if path.ends_with(pattern) || path.contains(&format!("/{pattern}")) {
                    return Ok(HookOutput::with_permission_decision(
                        "PreToolUse",
                        "deny",
                        &format!("Protected file: {pattern} files may contain secrets"),
                    ));
                }
            }

            // Block access to credential files
            for pattern in &protection.patterns {
                if path.ends_with(pattern) || path.contains(pattern) {
                    return Ok(HookOutput::with_permission_decision(
                        "PreToolUse",
                        "deny",
                        "Protected file: may contain credentials or private keys",
                    ));
                }
            }
        }
    }

    // No rule matched, no protection triggered - let Claude ask the user
    Ok(HookOutput::empty())
}
