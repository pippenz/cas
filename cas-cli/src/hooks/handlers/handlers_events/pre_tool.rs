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

    // ========================================================================
    // SUPERVISOR DISCIPLINE: Block Agent(isolation="worktree") for supervisors
    //
    // Supervisors must spawn workers via `mcp__cas__coordination spawn_workers`
    // so worktrees are factory-tracked and garbage-collected. Raw `Agent` calls
    // with `isolation: "worktree"` create worktrees Claude Code cleans up only
    // on process exit — which leaks across Petrastella repos when the session
    // is long-lived (see EPIC cas-7c88 / project_factory_worktree_leak).
    //
    // Non-isolation Agent calls (Explore, code-review personas, task-verifier)
    // stay allowed — they're load-bearing for correctness verification.
    //
    // Placed before the cas_root check so the gate fires even if CAS isn't
    // initialized in the supervisor's cwd (belt-and-suspenders; should never
    // happen in factory mode).
    // ========================================================================
    if tool_name == "Agent" && crate::harness_policy::is_supervisor(input) {
        let tool_input = input.tool_input.as_ref();
        let isolation = tool_input.and_then(|ti| ti.get("isolation").and_then(|v| v.as_str()));
        let subagent_type =
            tool_input.and_then(|ti| ti.get("subagent_type").and_then(|v| v.as_str()));
        // Task-verifier is exempt: supervisors legitimately spawn it to unjail
        // epic verification (see handlers_events/pre_tool.rs task-verifier unjail
        // block below). Blocking it here would strand supervisors in
        // pending_verification if a future caller ever pairs it with isolation.
        let is_verifier_exempt = subagent_type == Some("task-verifier");
        if isolation == Some("worktree") && !is_verifier_exempt {
            return Ok(HookOutput::with_pre_tool_permission(
                "deny",
                "🚫 Supervisors must not spawn isolated-worktree subagents.\n\
                Use mcp__cas__coordination action=spawn_workers — factory-managed worktrees get cleaned up; Agent(isolation=\"worktree\") ones leak.\n\
                If you genuinely need a throwaway subagent, drop `isolation` or run as a worker via `cas factory`.",
            ));
        }
    }

    // Check if CAS is initialized
    let cas_root = match cas_root {
        Some(root) => root,
        None => return Ok(HookOutput::empty()),
    };

    // Create shared store cache — all store accesses below go through this
    // instead of calling open_*() directly, reducing ~11 SQLite connections to ~3-4.
    let mut stores = ToolHookStores::new(cas_root);

    // Compute current agent's task IDs (via leases) once for all jail checks.
    // This prevents cross-agent jail contamination where Agent A's pending tasks
    // block Agent B in a different session.
    let current_agent_id = current_agent_id(input);
    let agent_task_ids: std::collections::HashSet<String> = stores
        .agents()
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
    let is_factory_agent = crate::harness_policy::is_factory_agent(input);
    if is_factory_agent && tool_name == "SendMessage" {
        return Ok(HookOutput::with_pre_tool_permission(
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
    let is_supervisor = crate::harness_policy::is_supervisor(input);
    let worker_supports_subagents = worker_harness_from_env().capabilities().supports_subagents;

    // ========================================================================
    // CODEMAP FRESHNESS GATE: Block supervisor from creating tasks / spawning
    // workers while CODEMAP.md is significantly out of date.
    //
    // Workers use CODEMAP for codebase orientation. Dispatching them against a
    // stale map wastes tokens and produces drift. The SessionStart warning is
    // informational; this gate enforces "update before assigning work".
    //
    // Only fires for supervisors, only on the two dispatch tools, only when
    // staleness >= SIGNIFICANT_STALENESS_THRESHOLD. Running `/codemap` bumps
    // CODEMAP.md's mtime and clears the gate on the next call.
    // ========================================================================
    if is_supervisor {
        let action = input
            .tool_input
            .as_ref()
            .and_then(|ti| ti.get("action").and_then(|v| v.as_str()));
        let is_gated = matches!(
            (tool_name, action),
            ("mcp__cas__task", Some("create"))
                | ("mcp__cas__coordination", Some("spawn_workers"))
                | ("mcp__cas__coordination", Some("spawn_worker"))
        );
        if is_gated {
            if let Some(crate::hooks::handlers::handlers_events::CodemapStaleness::SignificantlyStale { total_changes, .. }) =
                crate::hooks::handlers::handlers_events::check_codemap_freshness(cas_root)
            {
                return Ok(HookOutput::with_pre_tool_permission(
                    "deny",
                    &format!(
                        "🗺️  CODEMAP.md is significantly out of date ({total_changes} structural changes).\n\n\
                        Workers rely on CODEMAP for codebase orientation — dispatching against a stale map wastes tokens.\n\n\
                        Run `/codemap` to refresh, then retry."
                    ),
                ));
            }
        }
    }

    // Factory workers are exempt from verification jail — they may have multiple
    // tasks assigned and must be able to continue working on other tasks while
    // one awaits verification. The pending_verification flag on the task itself
    // still prevents re-closing without verification (enforced in close_ops.rs).
    let is_factory_worker =
        crate::harness_policy::is_worker(input) && std::env::var("CAS_FACTORY_MODE").is_ok();

    // Verification jail is only relevant when worker harness supports subagents.
    if worker_supports_subagents && !is_supervisor && !is_factory_worker {
        if let Some(task_store) = stores.tasks().cloned() {
            if let Ok(tasks) = task_store.list_pending_verification() {
                // Filter to tasks owned by the current agent:
                //    a. The current agent has an active lease on them (regular tasks), OR
                //    b. The current agent is the epic_verification_owner (epic tasks)
                let pending_tasks: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
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

                // cas-c29a: auto-escalate stale verification dispatches. If a task
                // has been jailed for >VERIFICATION_JAIL_TIMEOUT_SECS with a
                // dispatch-request row that never got a verdict, the task-verifier
                // subagent is presumed dead. Clear pending_verification so the jail
                // releases and the tool call proceeds instead of looping forever.
                const VERIFICATION_JAIL_TIMEOUT_SECS: i64 = 600;
                const DISPATCH_SUMMARY_PREFIX: &str = "Dispatch requested";
                let pending_tasks: Vec<_> =
                    if let Some(verification_store) = stores.verification().cloned() {
                        pending_tasks
                            .into_iter()
                            .filter(|t| {
                                let is_stale = matches!(
                                    verification_store.get_latest_for_task(&t.id),
                                    Ok(Some(ref v))
                                        if v.status == crate::types::VerificationStatus::Error
                                            && v.summary.starts_with(DISPATCH_SUMMARY_PREFIX)
                                            && (chrono::Utc::now() - v.created_at).num_seconds()
                                                > VERIFICATION_JAIL_TIMEOUT_SECS
                                );
                                if is_stale {
                                    let mut task_to_update = (*t).clone();
                                    task_to_update.pending_verification = false;
                                    task_to_update.updated_at = chrono::Utc::now();
                                    let _ = task_store.update(&task_to_update);
                                    warn!(
                                        task_id = %t.id,
                                        "[VERIFICATION JAIL] auto-escalated stale dispatch — verifier never responded"
                                    );
                                    false
                                } else {
                                    true
                                }
                            })
                            .collect()
                    } else {
                        pending_tasks
                    };

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
                    // Check if this is Task/Agent tool spawning task-verifier
                    // (Newer Claude Code renamed "Task" to "Agent" — accept both.)
                    let is_verifier_agent = if tool_name == "Task" || tool_name == "Agent" {
                        let subagent_type = input
                            .tool_input
                            .as_ref()
                            .and_then(|ti| ti.get("subagent_type").and_then(|v| v.as_str()));
                        debug!(
                            subagent_type = ?subagent_type,
                            tool = tool_name,
                            "[VERIFICATION JAIL] Task/Agent tool detected"
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

                        return Ok(HookOutput::with_pre_tool_permission(
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
    // SUPERVISOR TASK-VERIFIER UNJAIL
    //
    // Supervisors are exempt from verification jail (above), but when they
    // spawn task-verifier for their own tasks (or a worker's task), we still
    // need to write the unjail marker so the task-verifier subagent (running
    // in supervisor context) can record verification via cas_verification_add.
    // We also clear pending_verification so the task isn't stuck.
    // ========================================================================
    if is_supervisor && (tool_name == "Task" || tool_name == "Agent") {
        let is_task_verifier = input
            .tool_input
            .as_ref()
            .and_then(|ti| ti.get("subagent_type").and_then(|v| v.as_str()))
            == Some("task-verifier");

        if is_task_verifier {
            let marker_path = cas_root.join(".verifier_unjail_marker");
            let marker_content = format!(
                "session={}\ntimestamp={}",
                current_agent_id,
                chrono::Utc::now()
            );
            let _ = std::fs::write(&marker_path, &marker_content);

            // Clear pending_verification for tasks assigned to this supervisor
            if let Some(task_store) = stores.tasks().cloned() {
                if let Ok(tasks) = task_store.list_pending_verification() {
                    for task in &tasks {
                        let is_owned = task
                            .assignee
                            .as_deref()
                            .map(|a| a == current_agent_id)
                            .unwrap_or(false)
                            || agent_task_ids.contains(&task.id);
                        if is_owned {
                            let mut task_to_update = task.clone();
                            task_to_update.pending_verification = false;
                            task_to_update.updated_at = chrono::Utc::now();
                            let _ = task_store.update(&task_to_update);
                        }
                    }
                }
            }

            info!(
                "[VERIFICATION] Supervisor spawning task-verifier — wrote unjail marker and cleared pending_verification"
            );
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
    let worktrees_enabled = stores.config().worktrees_enabled();

    // Factory workers manage their own worktrees — skip CAS worktree enforcement
    // to avoid conflicting redirects (factory uses per-worker worktrees, CAS uses per-epic)
    let is_factory_worker_for_wt = crate::harness_policy::is_worker(input);

    if worktrees_enabled && !is_factory_worker_for_wt {
        if let Some(task_store) = stores.tasks().cloned() {
            if let Ok(tasks) = task_store.list_pending_worktree_merge() {
                // Only consider tasks the current agent owns (reuses agent_task_ids from above)
                let pending_merge_tasks: Vec<_> = tasks
                    .iter()
                    .filter(|t| {
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

                        return Ok(HookOutput::with_pre_tool_permission(
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
                if let Some(agent_store) = stores.agents().cloned() {
                    if let Some(task_store) = stores.tasks().cloned() {
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
                                                            if let Some(wt_store) =
                                                                stores.worktrees().cloned()
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

                                                                        return Ok(HookOutput::with_pre_tool_permission(
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
                if let Some(wt_store) = stores.worktrees().cloned() {
                    if let Some(agent_store) = stores.agents().cloned() {
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
    let rule_store = stores.rules()?;
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
                return Ok(HookOutput::with_pre_tool_permission(
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
                return Ok(HookOutput::with_pre_tool_permission(
                    "allow",
                    &format!("Auto-approved safe tool by rule {}", rule.id),
                ));
            }
        }
    }

    // Check for protected paths that should be blocked (configurable)
    let protection = &stores.config().hooks().pre_tool_use.protection;

    if protection.enabled {
        if let Some(path) = file_path {
            // Block access to protected files (e.g., .env files)
            for pattern in &protection.files {
                if path.ends_with(pattern) || path.contains(&format!("/{pattern}")) {
                    return Ok(HookOutput::with_pre_tool_permission(
                        "deny",
                        &format!("Protected file: {pattern} files may contain secrets"),
                    ));
                }
            }

            // Block access to credential files
            for pattern in &protection.patterns {
                if path.ends_with(pattern) || path.contains(pattern) {
                    return Ok(HookOutput::with_pre_tool_permission(
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
