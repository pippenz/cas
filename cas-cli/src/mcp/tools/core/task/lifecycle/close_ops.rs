use crate::harness_policy::{
    is_supervisor_from_env, is_worker_without_subagents_from_env, supervisor_harness_from_env,
    verification_policy, worker_harness_from_env,
};
use crate::mcp::tools::core::imports::*;

/// Maximum time a task may sit in `pending_verification` before the close path
/// treats the task-verifier subagent as dead, auto-escalates, and releases the
/// jail. Addresses cas-c29a (within-task verification deadlock): if the
/// verifier subagent crashes, is never spawned, or fails silently, the
/// original dispatch-request row stays in `Error` status forever and every
/// close retry returns `VERIFICATION REQUIRED`.
const VERIFICATION_JAIL_TIMEOUT_SECS: i64 = 600;

/// Heartbeat staleness threshold (seconds) for deciding whether an assignee
/// is still considered active for verification-skip purposes. Aligned with
/// the same 5-minute window used by task-claim reclaim.
const ASSIGNEE_STALE_SECS: i64 = 300;

/// Marker prefix used on the dispatch-request verification row (see
/// lines ~255-272 below). Used to distinguish a stale dispatch from a real
/// verifier-written Error verdict during auto-escalation.
const DISPATCH_SUMMARY_PREFIX: &str = "Dispatch requested";

/// Why the close path decided to skip (or not skip) the task-verifier step
/// for a given close attempt.
///
/// Carried through to the response message so the audit trail cites the
/// real reason instead of the catch-all "assignee inactive" phrase that
/// surfaced cas-3bd4.
///
/// The pre-cas-3bd4 implementation represented this as a single
/// `assignee_inactive: bool`. Every lookup failure — including the
/// very-common name-vs-id mismatch described below — defaulted to `true`
/// and the success message confidently lied that the assignee was inactive.
/// This enum preserves the same skip *behavior* (supervisor still closes
/// orphaned or genuinely-stale tasks without a verifier hop) but forces
/// every skip reason to be named.
///
/// ## Why the old `agent_store.get(task.assignee)` kept returning "inactive"
///
/// `task.assignee` is set by `task_claiming.rs:89` to
/// `Some(agent_name.clone())` — the human-readable display name, e.g.
/// `"mighty-viper-52"`. But `AgentStore::get(id)` runs `WHERE id = ?` in
/// `ops_agent.rs:79`, and `id` is the session-id (a UUID-like
/// identifier), not the name. The lookup never found the row, so
/// `unwrap_or(true)` treated the worker as inactive even though it was
/// actively holding a fresh lease. `compute_verification_skip_reason`
/// fixes this by consulting the task's active lease first — `TaskLease`
/// stores the real `agent_id`, not the name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum VerificationSkipReason {
    /// The assignee is alive and no bypass flag was set. Verification
    /// runs normally; this is *not* a skip.
    None,
    /// The task has no assignee at all. Treated as orphaned; legacy
    /// callers reached this via the same skip path.
    NoAssignee,
    /// The assignee exists and is registered, but their heartbeat or
    /// lease is stale. `minutes_stale` is the observed staleness if we
    /// could measure it.
    AssigneeInactive { minutes_stale: Option<i64> },
    /// `task.assignee` is set but we cannot resolve it through the
    /// lease *or* a direct id lookup. The agent may have been GC'd or
    /// the assignee row holds an old display name no longer in the
    /// agent store. Skip verification and cite the real reason.
    AssigneeUnknown,
    /// Supervisor is closing a task whose assignee is still alive and
    /// has explicitly requested a verification skip via
    /// `bypass_code_review=true`. Separate from `AssigneeInactive` so
    /// the audit note reflects supervisor intent, not worker state.
    SupervisorBypass,
}

impl VerificationSkipReason {
    /// Whether this reason short-circuits the verification gate.
    pub(crate) fn is_skip(&self) -> bool {
        !matches!(self, VerificationSkipReason::None)
    }

    /// Short human-readable suffix appended to the `Closed task:` line.
    /// Must start with a leading space so it slots cleanly into the
    /// format string.
    pub(crate) fn response_suffix(&self, verification_enabled: bool) -> String {
        match self {
            VerificationSkipReason::None => {
                if verification_enabled {
                    " (verified)".to_string()
                } else {
                    String::new()
                }
            }
            VerificationSkipReason::NoAssignee => {
                " (verification skipped — orphaned task, no assignee)".to_string()
            }
            VerificationSkipReason::AssigneeInactive {
                minutes_stale: Some(m),
            } => {
                format!(" (verification skipped — assignee inactive for {m}m)")
            }
            VerificationSkipReason::AssigneeInactive { minutes_stale: None } => {
                " (verification skipped — assignee lease expired)".to_string()
            }
            VerificationSkipReason::AssigneeUnknown => {
                " (verification skipped — assignee unknown)".to_string()
            }
            VerificationSkipReason::SupervisorBypass => {
                " (verification skipped — supervisor bypass via bypass_code_review=true)"
                    .to_string()
            }
        }
    }

    /// Reason text written to the `Skipped` verification row so the
    /// audit trail records the accurate reason alongside the row, not
    /// just in the response text.
    pub(crate) fn audit_reason(&self) -> String {
        match self {
            VerificationSkipReason::None => String::new(),
            VerificationSkipReason::NoAssignee => {
                "Closed via supervisor bypass — task had no assignee (orphaned).".to_string()
            }
            VerificationSkipReason::AssigneeInactive {
                minutes_stale: Some(m),
            } => format!(
                "Closed via supervisor bypass — assignee inactive for {m} minute(s) at close time."
            ),
            VerificationSkipReason::AssigneeInactive { minutes_stale: None } => {
                "Closed via supervisor bypass — assignee lease had expired at close time."
                    .to_string()
            }
            VerificationSkipReason::AssigneeUnknown => {
                "Closed via supervisor bypass — assignee row not found in agent store (likely \
                 a stale or renamed agent)."
                    .to_string()
            }
            VerificationSkipReason::SupervisorBypass => {
                "Closed via supervisor bypass — bypass_code_review=true explicitly set by \
                 supervisor while assignee was still active."
                    .to_string()
            }
        }
    }
}

impl CasCore {
    pub async fn cas_task_close(
        &self,
        Parameters(req): Parameters<TaskCloseRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        let task = task_store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Task not found: {e}")),
            data: None,
        })?;

        // For Epics: Check that all worker branches are merged before verification
        // This ensures epic-level verification runs on the complete merged code
        if task.task_type == TaskType::Epic {
            let target_branch = task.branch.as_deref().unwrap_or("master");
            let unmerged = check_unmerged_epic_branches(&req.id, target_branch);
            if !unmerged.is_empty() {
                let branch_list = unmerged.join("\n  - ");
                return Ok(Self::tool_error(format!(
                    "⚠️ MERGE REQUIRED\n\n\
                    Epic {} has {} unmerged worker branch(es):\n  - {}\n\n\
                    Worker branches must be merged to {} before closing the epic.\n\n\
                    Use /factory-merge-epic to:\n\
                    1. Fetch all worker branches from remote\n\
                    2. Merge each branch to {}\n\
                    3. Run tests on the merged code\n\n\
                    After merging, call mcp__cas__task action=close id={} again.",
                    req.id,
                    unmerged.len(),
                    branch_list,
                    target_branch,
                    target_branch,
                    req.id
                )));
            }
        }

        // Check verification status if enabled
        let config = self.load_config();
        let policy = verification_policy(supervisor_harness_from_env(), worker_harness_from_env());
        let is_factory_worker = std::env::var("CAS_AGENT_ROLE")
            .map(|r| r.eq_ignore_ascii_case("worker"))
            .unwrap_or(false)
            && std::env::var("CAS_FACTORY_MODE").is_ok();
        let verification_enabled = config.verification_enabled()
            && if task.task_type == TaskType::Epic {
                if is_supervisor_from_env() {
                    policy.epic_required()
                } else {
                    true
                }
            } else {
                policy.task_required()
            };

        // Skip verification for orphaned tasks: if caller is supervisor and the
        // task's assignee is inactive (heartbeat expired or lease gone), allow
        // close without verification. cas-3bd4: compute the reason as a typed
        // enum so the response message cites the actual state instead of
        // defaulting to "assignee inactive" for every lookup failure.
        let skip_reason = if verification_enabled && is_supervisor_from_env() {
            self.compute_verification_skip_reason(&task, &req)
        } else {
            VerificationSkipReason::None
        };
        let skip_verification = skip_reason.is_skip();

        // Also allow supervisor to skip verification jail when they are the
        // task assignee for a non-epic task (fixes supervisor self-close deadlock).
        let supervisor_is_assignee = is_supervisor_from_env()
            && task.task_type != TaskType::Epic
            && self
                .get_agent_id()
                .ok()
                .map(|aid| task.assignee.as_deref() == Some(aid.as_str()))
                .unwrap_or(false);

        if verification_enabled && !skip_verification {
            let is_worker_without_subagents = is_worker_without_subagents_from_env();

            // Check for approved verification
            if let Ok(verification_store) = self.open_verification_store() {
                // Determine verification type and agent based on task type
                let is_epic = task.task_type == TaskType::Epic;
                let (verification_type, verifier_agent) = if is_epic {
                    (VerificationType::Epic, "task-verifier")
                } else {
                    (VerificationType::Task, "task-verifier")
                };

                // Get the appropriate verification (by type for epics, any for tasks)
                let latest = if is_epic {
                    verification_store.get_latest_for_task_by_type(&req.id, verification_type)
                } else {
                    verification_store.get_latest_for_task(&req.id)
                };

                // Whether a prior verification row (of any status) already
                // exists. Used below to decide whether to persist a fresh
                // dispatch-request marker so the close attempt is durably
                // observable instead of fire-and-forget.
                let had_prior_verification = matches!(&latest, Ok(Some(_)));

                match latest {
                    Ok(Some(v))
                        if v.status == VerificationStatus::Approved
                            || v.status == VerificationStatus::Skipped =>
                    {
                        // Verification approved or explicitly skipped
                        // (supervisor bypass row from a prior orphaned close) —
                        // proceed with close. See cas-82d6.
                    }
                    Ok(Some(v)) if v.status == VerificationStatus::Rejected => {
                        // Verification rejected, block close
                        // Only auto-claim if the closing agent is the task's assignee.
                        // If a supervisor closes a worker's task, skip the lease to avoid
                        // locking the task to the supervisor.
                        let is_assignee = self
                            .get_agent_id()
                            .ok()
                            .map(|aid| task.assignee.as_deref() == Some(aid.as_str()))
                            .unwrap_or(false);
                        if is_assignee {
                            self.auto_claim_for_verification(&req.id, task_store.as_ref())?;
                        }

                        let issue_count = v.issues.len();
                        let blocking = v
                            .issues
                            .iter()
                            .filter(|i| i.severity == crate::types::IssueSeverity::Blocking)
                            .count();

                        // Include new close reason if provided (may have been fixed)
                        let close_reason_note = if let Some(ref reason) = req.reason {
                            format!(
                                "\n\n## New Close Reason Provided\n\
                                ```\n{reason}\n```\n\n\
                                If resubmitting, ensure the close reason describes COMPLETED work only.\n\
                                Do not use language like 'remaining', 'beyond scope', 'will need to', etc."
                            )
                        } else {
                            String::new()
                        };

                        return Ok(Self::tool_error(format!(
                            "⚠️ VERIFICATION FAILED\n\n\
                            Task {} has a rejected verification with {} issue(s) ({} blocking).\n\n\
                            Summary: {}\n\n\
                            {}{}\n\n\
                            {}",
                            req.id,
                            issue_count,
                            blocking,
                            v.summary,
                            if is_worker_without_subagents {
                                "To fix: Address the issues in this worker.\n\
                                    Then ask supervisor to run verification (task-verifier or direct mcp__cs__verification) and close the task on your behalf."
                                    .to_string()
                            } else {
                                format!(
                                    "To fix: Address the issues and run the {verifier_agent} agent again."
                                )
                            },
                            close_reason_note,
                            if is_worker_without_subagents {
                                format!(
                                    "Suggested message: mcp__cs__coordination action=message target=supervisor message=\"Task {} is ready for re-verification. Please verify (task-verifier or direct mcp__cs__verification) and close if approved.\"",
                                    req.id
                                )
                            } else {
                                format!(
                                    "To verify: Task(subagent_type=\"{}\", prompt=\"Verify task {}\")",
                                    verifier_agent, req.id
                                )
                            }
                        )));
                    }
                    Ok(Some(ref v))
                        if v.status == VerificationStatus::Error
                            && v.summary.starts_with(DISPATCH_SUMMARY_PREFIX)
                            && (chrono::Utc::now() - v.created_at).num_seconds()
                                > VERIFICATION_JAIL_TIMEOUT_SECS =>
                    {
                        // Stale dispatch-request row: the task-verifier subagent was
                        // supposed to write a verdict but never did. This is the
                        // within-task verification deadlock from cas-c29a. Auto-escalate
                        // so the supervisor sees a clean failure instead of an infinite
                        // VERIFICATION REQUIRED loop.
                        let elapsed_mins =
                            (chrono::Utc::now() - v.created_at).num_seconds() / 60;

                        // Clear pending_verification so the jail releases.
                        let mut task_to_update = task.clone();
                        task_to_update.pending_verification = false;
                        task_to_update.updated_at = chrono::Utc::now();
                        let _ = task_store.update(&task_to_update);

                        // Release any lease so the supervisor can reclaim the task.
                        if let Ok(agent_store) = self.open_agent_store() {
                            let _ = agent_store.release_lease_for_task(&req.id);
                        }

                        // Replace the stale dispatch row with a timeout diagnostic so
                        // the audit trail shows escalation instead of a dangling
                        // "Dispatch requested" row.
                        let mut timeout_row = v.clone();
                        timeout_row.summary = format!(
                            "Verification timed out after {elapsed_mins} minutes — \
                             task-verifier subagent never recorded a verdict. \
                             Auto-escalated by cas_task_close: pending_verification cleared, \
                             lease released. Supervisor must re-dispatch verifier or record \
                             verdict manually."
                        );
                        timeout_row.created_at = chrono::Utc::now();
                        let _ = verification_store.update(&timeout_row);

                        // Surface an activity event so the TUI shows the escalation.
                        if let Ok(agent_id) = self.get_agent_id() {
                            let event = crate::mcp::socket::DaemonEvent::WorkerActivity {
                                session_id: agent_id,
                                event_type: "verification_timeout_escalated".to_string(),
                                description: format!(
                                    "Verification timed out ({elapsed_mins}m): {}",
                                    req.id
                                ),
                                entity_id: Some(req.id.clone()),
                            };
                            let _ = crate::mcp::socket::send_event(&self.cas_root, &event);
                        }

                        return Ok(Self::tool_error(format!(
                            "⚠️ VERIFICATION TIMED OUT\n\n\
                            Task {} was awaiting verification for {} minutes with no verdict \
                            from the task-verifier subagent. Auto-escalated: verification jail \
                            released, lease freed.\n\n\
                            This usually means the task-verifier subagent crashed, was never \
                            spawned, or failed silently.\n\n\
                            To proceed:\n\
                            1. Re-dispatch verifier: Task(subagent_type=\"task-verifier\", prompt=\"Verify task {}\")\n\
                            2. Or record verdict directly: mcp__cas__verification action=add task_id={} status=approved summary=\"...\"\n\
                            3. Then call cas_task_close again.",
                            req.id, elapsed_mins, req.id, req.id
                        )));
                    }
                    Ok(None) | Ok(Some(_)) => {
                        // No verification or pending/error status
                        // Only auto-claim if the closing agent is the task's assignee.
                        // If a supervisor closes a worker's task, skip the lease to avoid
                        // locking the task to the supervisor.
                        let is_assignee = self
                            .get_agent_id()
                            .ok()
                            .map(|aid| task.assignee.as_deref() == Some(aid.as_str()))
                            .unwrap_or(false);
                        if is_assignee {
                            self.auto_claim_for_verification(&req.id, task_store.as_ref())?;
                        }

                        // Set pending_verification flag to enable verification jail
                        let mut task_to_update = task.clone();
                        task_to_update.pending_verification = true;
                        if task_to_update.assignee.is_none() {
                            if let Ok(agent_id) = self.get_agent_id() {
                                task_to_update.assignee = Some(agent_id);
                            }
                        }
                        task_to_update.updated_at = chrono::Utc::now();
                        let _ = task_store.update(&task_to_update);

                        // Include close reason in the message so verifier can check it
                        let close_reason_section = if let Some(ref reason) = req.reason {
                            format!(
                                "\n\n## Proposed Close Reason\n\
                                ```\n{reason}\n```\n\n\
                                IMPORTANT: The {verifier_agent} MUST validate this close reason.\n\
                                Reject if it admits incomplete work (e.g., 'remaining items', 'beyond scope', 'will need to')."
                            )
                        } else {
                            String::new()
                        };

                        let verification_desc = if is_epic {
                            "Epic verification runs on master to verify the complete merged implementation.\n\
                            The agent will check that all subtask implementations integrate correctly.\n\
                            The verifier MUST record verification_type=epic."
                        } else {
                            "The agent will check for TODO comments, stubs, incomplete implementations,\n\
                            AND validate the close reason doesn't admit incomplete work."
                        };

                        // Send verification blocked activity event (for supervisor visibility)
                        if let Ok(agent_id) = self.get_agent_id() {
                            let event = crate::mcp::socket::DaemonEvent::WorkerActivity {
                                session_id: agent_id,
                                event_type: "worker_verification_blocked".to_string(),
                                description: format!("Awaiting verification: {}", req.id),
                                entity_id: Some(req.id.clone()),
                            };
                            let _ = crate::mcp::socket::send_event(&self.cas_root, &event);
                        }

                        // Persist a durable dispatch-request row so the close
                        // attempt is observable (in tests, in the UI, and in
                        // audit trails) instead of fire-and-forget text. The
                        // task-verifier subagent will later write its verdict
                        // as a newer row; get_latest_for_task returns the
                        // newest, so behavior on retry is unchanged. Only
                        // create the row on the first attempt — don't
                        // duplicate on repeated close calls.
                        if !had_prior_verification {
                            if let Ok(ver_id) = verification_store.generate_id() {
                                let mut dispatch_row =
                                    Verification::new(ver_id, req.id.clone());
                                dispatch_row.verification_type = verification_type;
                                dispatch_row.status = VerificationStatus::Error;
                                if let Ok(agent_id) = self.get_agent_id() {
                                    dispatch_row.agent_id = Some(agent_id);
                                }
                                dispatch_row.summary = format!(
                                    "Dispatch requested — task-verifier subagent must be spawned via \
                                     Task(subagent_type=\"task-verifier\", prompt=\"Verify task {}\"). \
                                     This row will be superseded by the subagent's verdict.",
                                    req.id
                                );
                                let _ = verification_store.add(&dispatch_row);
                            }
                        }

                        let verification_gate = if is_factory_worker {
                            format!(
                                "🔒 Factory worker verification gate: this close will only succeed after a task-verifier records a verdict.\n\n\
                                 Spawn the verifier now (other tools remain available while it runs):\n\n\
                                 Task(subagent_type=\"{}\", prompt=\"Verify task {}\")",
                                verifier_agent, req.id
                            )
                        } else if supervisor_is_assignee {
                            format!(
                                "You implemented this task yourself. Spawn a task-verifier to review your work:\n\n\
                                 Task(subagent_type=\"{}\", prompt=\"Verify task {}\")\n\n\
                                 Or record verification directly:\n\
                                 mcp__cas__verification action=add task_id={} status=approved summary=\"Self-verified: <reason>\"",
                                verifier_agent, req.id, req.id
                            )
                        } else {
                            format!(
                                "🔒 VERIFICATION JAIL ACTIVE: You cannot use other tools until you verify this task.\n\n\
                                 Use the Task tool to spawn a task-verifier subagent: \
                                 Task(subagent_type=\"{}\", prompt=\"Verify task {}\")",
                                verifier_agent, req.id
                            )
                        };

                        return Ok(Self::tool_error(format!(
                            "⚠️ VERIFICATION REQUIRED\n\n\
                            Task {} requires verification before closing.\n\n\
                            {}{}\n\n\
                            {}{}\n\n\
                            {}",
                            req.id,
                            verification_gate,
                            verification_desc,
                            close_reason_section.as_str(),
                            if is_worker_without_subagents {
                                format!(
                                    "Ask supervisor to run verification (task-verifier or direct mcp__cs__verification) and close task {} on your behalf.",
                                    req.id
                                )
                            } else {
                                String::new()
                            },
                            if is_worker_without_subagents {
                                format!(
                                    "Suggested message: mcp__cs__coordination action=message target=supervisor message=\"Please verify task {} (task-verifier or direct mcp__cs__verification) and close it if approved.\"",
                                    req.id
                                )
                            } else {
                                "After verification passes, call cas_task_close again.".to_string()
                            }
                        )));
                    }
                    Err(_) => {
                        // Verification store error, proceed anyway
                    }
                }
            }
        }

        // Check for worktree that needs merging (only for epics or tasks with worktrees)
        // This check happens AFTER verification passes
        if let Some(worktree_id) = &task.worktree_id {
            let config = self.load_config();

            // Only trigger jail if worktrees are enabled and require_merge_on_epic_close is true
            let should_check_worktree = config
                .worktrees
                .as_ref()
                .map(|wc| wc.enabled && wc.require_merge_on_epic_close)
                .unwrap_or(false);

            if should_check_worktree {
                if let Ok(wt_store) = self.open_worktree_store() {
                    if let Ok(worktree) = wt_store.get(worktree_id) {
                        // Check if worktree still exists, is active, and hasn't been merged
                        // Skip jail if: removed, merged status, or has merged_at timestamp
                        let needs_merge = worktree.removed_at.is_none()
                            && worktree.status == WorktreeStatus::Active
                            && worktree.merged_at.is_none();

                        if needs_merge {
                            // Set pending_worktree_merge flag to enable worktree jail
                            let mut task_to_update = task.clone();
                            task_to_update.pending_worktree_merge = true;
                            if task_to_update.assignee.is_none() {
                                if let Ok(agent_id) = self.get_agent_id() {
                                    task_to_update.assignee = Some(agent_id);
                                }
                            }
                            task_to_update.updated_at = chrono::Utc::now();
                            let _ = task_store.update(&task_to_update);

                            return Ok(Self::tool_error(format!(
                                "⚠️ WORKTREE MERGE REQUIRED\n\n\
                                Task {} has an associated worktree that needs to be merged before closing.\n\n\
                                📍 Worktree: {}\n\
                                🌿 Branch: {}\n\n\
                                🔒 WORKTREE JAIL ACTIVE: You cannot use other tools until you spawn the 'worktree-merger' agent.\n\n\
                                To merge: Spawn the 'worktree-merger' agent to:\n\
                                1. Check for uncommitted changes and commit them\n\
                                2. Push the branch to remote\n\
                                3. Merge the branch to the parent branch\n\
                                4. Clean up the worktree directory\n\n\
                                After the merge completes, call cas_task_close again.",
                                req.id,
                                worktree.path.display(),
                                worktree.branch
                            )));
                        }
                    }
                }
            }
        }

        // cas-e235: additive-only execution_note backstop.
        // If the worker declared `execution_note=additive-only`, reject the
        // close if git sees any modified, deleted, or renamed files in the
        // project tree. This only fires when the project root is a git
        // repository; in non-git contexts the check silently no-ops so the
        // gate never blocks closes it can't reason about.
        if task.execution_note.as_deref() == Some("additive-only") {
            let project_root = self.cas_root.parent().unwrap_or(&self.cas_root);
            let violations = check_additive_only_violations(project_root);
            if !violations.is_empty() {
                let file_list = violations
                    .iter()
                    .map(|v| format!("  {} ({})", v.path, v.status))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(Self::tool_error(format!(
                    "⚠️ ADDITIVE-ONLY VIOLATION\n\n\
                    task close rejected: execution_note=additive-only but diff contains \
                    modifications.\n\n\
                    Modified/deleted/renamed files:\n{file_list}\n\n\
                    Use execution_note=null or test-first to modify existing files."
                )));
            }
        }

        // cas-b39f: cas-code-review P0 close gate (Unit 9).
        //
        // This is the integration point for the multi-persona code review
        // pipeline. The *dispatch* of the review skill itself happens via
        // the worker's harness (the skill must be invoked through the
        // Task tool by an LLM, not from Rust), so the Phase 1 gate works
        // in three cooperating layers:
        //
        //   1. Skip conditions (here) — additive-only tasks, non-code
        //      diffs, and supervisor overrides bypass the gate before
        //      any review is attempted.
        //   2. The pure-Rust decision helper at
        //      `cas_store::code_review::close_gate::evaluate_gate` —
        //      given a residual finding set, returns Allow or
        //      BlockOnP0. Exhaustively unit-tested there.
        //   3. Graceful degradation — if the review pipeline is
        //      unavailable (skill not installed, orchestrator crash,
        //      no findings-cache entry), log a warning and allow the
        //      close. The task description is explicit: code review
        //      must not become a SPOF for closes.
        //
        // Supervisor override flow:
        //   * Caller sets `bypass_code_review=true` on the close
        //     request.
        //   * If `CAS_AGENT_ROLE=supervisor`, the gate is skipped and
        //     a decision note is appended to the task capturing who
        //     overrode and the close reason.
        //   * Any other caller setting the flag gets an explicit
        //     rejection — we do not silently ignore unauthorized
        //     overrides because that would mask a misconfigured
        //     harness.
        let close_project_root = self.cas_root.parent().unwrap_or(&self.cas_root);
        match run_code_review_gate(&task, &req, close_project_root) {
            CodeReviewGateOutcome::Proceed => {}
            CodeReviewGateOutcome::AppendDecisionNote(note) => {
                let mut t = task.clone();
                if t.notes.is_empty() {
                    t.notes = note;
                } else {
                    t.notes = format!("{}\n\n{}", t.notes, note);
                }
                t.updated_at = chrono::Utc::now();
                let _ = task_store.update(&t);
            }
            CodeReviewGateOutcome::Reject(msg) => {
                return Ok(Self::tool_error(msg));
            }
        }

        // Proceed with close
        let mut task = task;
        let now = chrono::Utc::now();
        task.status = TaskStatus::Closed;
        task.closed_at = Some(now);
        task.updated_at = now;

        // Capture deliverables on close
        let mut deliverables = task.deliverables.clone();
        if let Some(worktree_id) = &task.worktree_id {
            if let Ok(wt_store) = self.open_worktree_store() {
                if let Ok(worktree) = wt_store.get(worktree_id) {
                    if let Some(commit) = worktree.merge_commit.clone() {
                        deliverables.merge_commit = Some(commit);
                    }
                }
            }
        }
        task.deliverables = deliverables;

        // When closing via the supervisor bypass (assignee inactive / orphaned /
        // supervisor-forced), we skip the verification gate but MUST still
        // write a durable `Skipped` verification row. Without this row, the
        // MCP jail (`check_pending_verification`) treats the task as
        // unverified and blocks every downstream worker that inherits a
        // BlockedBy on this task. See cas-82d6.
        //
        // cas-3bd4: the Skipped row now records the *actual* skip reason
        // (from `VerificationSkipReason::audit_reason`) instead of the
        // catch-all "assignee inactive or orphaned task" string.
        if skip_verification && verification_enabled {
            if let Ok(verification_store) = self.open_verification_store() {
                let needs_row = verification_store
                    .get_latest_for_task(&req.id)
                    .map(|v| {
                        !matches!(
                            v,
                            Some(ref r) if r.status == VerificationStatus::Approved
                                || r.status == VerificationStatus::Skipped
                        )
                    })
                    .unwrap_or(true);
                if needs_row {
                    if let Ok(ver_id) = verification_store.generate_id() {
                        let mut row = Verification::skipped(
                            ver_id,
                            req.id.clone(),
                            skip_reason.audit_reason(),
                        );
                        row.verification_type = if task.task_type == TaskType::Epic {
                            VerificationType::Epic
                        } else {
                            VerificationType::Task
                        };
                        if let Ok(agent_id) = self.get_agent_id() {
                            row.agent_id = Some(agent_id);
                        }
                        let _ = verification_store.add(&row);
                    }
                }
            }
        }

        if let Some(reason) = &req.reason {
            task.close_reason = Some(reason.clone());
            let timestamp = now.format("%Y-%m-%d %H:%M");
            let close_note = format!("[{timestamp}] Closed: {reason}");
            if task.notes.is_empty() {
                task.notes = close_note;
            } else {
                task.notes = format!("{}\n\n{}", task.notes, close_note);
            }
        }

        task_store.update(&task).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        // Auto-unblock tasks that were blocked solely by this task.
        // This keeps dependency state and task status synchronized.
        let mut auto_unblocked_tasks: Vec<String> = Vec::new();
        if let Ok(dependents) = task_store.get_dependents(&req.id) {
            for dep in dependents
                .iter()
                .filter(|dep| dep.dep_type == DependencyType::Blocks)
            {
                if let Ok(mut dependent_task) = task_store.get(&dep.from_id) {
                    if dependent_task.status != TaskStatus::Blocked {
                        continue;
                    }
                    let is_unblocked = task_store
                        .get_blockers(&dependent_task.id)
                        .map(|blockers| blockers.is_empty())
                        .unwrap_or(false);
                    if !is_unblocked {
                        continue;
                    }
                    dependent_task.status = TaskStatus::Open;
                    dependent_task.updated_at = chrono::Utc::now();
                    let timestamp = dependent_task.updated_at.format("%Y-%m-%d %H:%M");
                    let unblock_note = format!(
                        "[{}] Auto-unblocked: all blockers closed (latest: {}).",
                        timestamp, req.id
                    );
                    if dependent_task.notes.is_empty() {
                        dependent_task.notes = unblock_note;
                    } else {
                        dependent_task.notes =
                            format!("{}\n\n{}", dependent_task.notes, unblock_note);
                    }
                    if task_store.update(&dependent_task).is_ok() {
                        auto_unblocked_tasks.push(dependent_task.id.clone());
                    }
                }
            }
        }

        // Track epic completion with subtask count and duration
        if task.task_type == TaskType::Epic {
            let subtasks = task_store.get_subtasks(&req.id).unwrap_or_default();
            let duration_mins = task
                .closed_at
                .zip(Some(task.created_at))
                .map(|(closed, created)| (closed - created).num_minutes().max(0) as u64)
                .unwrap_or(0);
            crate::telemetry::track_epic_completed(subtasks.len(), duration_mins);
        }

        // Release any lease on this task (regardless of who owns it)
        let lease_msg = if let Ok(agent_store) = self.open_agent_store() {
            match agent_store.release_lease_for_task(&req.id) {
                Ok(true) => " (lease released)",
                Ok(false) => "",
                Err(_) => "",
            }
        } else {
            ""
        };

        // cas-3bd4: use the typed skip reason so the audit suffix cites
        // the real reason (e.g. "assignee unknown" for name/id mismatches,
        // "supervisor bypass" for explicit overrides) instead of always
        // saying "assignee inactive".
        let verification_note = skip_reason.response_suffix(verification_enabled);

        // Note about worktree status (merge already handled by worktree-merger agent)
        let worktree_msg = if let Some(worktree_id) = &task.worktree_id {
            if let Ok(wt_store) = self.open_worktree_store() {
                if let Ok(worktree) = wt_store.get(worktree_id) {
                    if worktree.removed_at.is_some() {
                        // Worktree was merged and cleaned up by worktree-merger
                        format!("\n🌳 Worktree merged (branch: {})", worktree.branch)
                    } else {
                        // Worktree still exists - this shouldn't happen if jail worked correctly
                        format!("\n⚠️ Worktree still exists at {}", worktree.path.display())
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Check if this task is a subtask of an epic, and if all siblings are now closed
        let epic_close_msg = {
            // Get dependencies of this task to find its parent
            let deps = task_store.get_dependencies(&req.id).unwrap_or_default();
            let parent_dep = deps
                .iter()
                .find(|d| d.dep_type == cas_types::DependencyType::ParentChild);

            if let Some(dep) = parent_dep {
                // Get the parent task
                if let Ok(parent) = task_store.get(&dep.to_id) {
                    // Check if parent is an Epic
                    if parent.task_type == cas_types::TaskType::Epic
                        && parent.status != TaskStatus::Closed
                    {
                        // Get all subtasks of this epic
                        let subtasks = task_store.get_subtasks(&parent.id).unwrap_or_default();

                        // Check if all subtasks are now closed
                        let all_closed = subtasks.iter().all(|t| t.status == TaskStatus::Closed);

                        if all_closed && !subtasks.is_empty() {
                            // In factory mode, workers shouldn't close epics - supervisor handles that
                            let is_factory_worker = std::env::var("CAS_AGENT_ROLE")
                                .map(|r| r.to_lowercase() == "worker")
                                .unwrap_or(false);

                            if is_factory_worker {
                                // Send real notification to supervisor via daemon event
                                if let Ok(agent_id) = self.get_agent_id() {
                                    let event = crate::mcp::socket::DaemonEvent::WorkerActivity {
                                        session_id: agent_id,
                                        event_type: "epic_subtasks_complete".to_string(),
                                        description: format!(
                                            "All subtasks of epic '{}' ({}) are complete — ready to close",
                                            parent.title, parent.id
                                        ),
                                        entity_id: Some(parent.id.clone()),
                                    };
                                    let _ = crate::mcp::socket::send_event(&self.cas_root, &event);
                                }

                                format!(
                                    "\n\n🎉 All subtasks of epic '{}' ({}) are now complete!\n\
                                     → The supervisor has been notified to close the epic.",
                                    parent.title, parent.id
                                )
                            } else {
                                format!(
                                    "\n\n🎉 All subtasks of epic '{}' ({}) are now complete!\n\
                                     → Consider closing the epic with: mcp__cas__task action=close id={}",
                                    parent.title, parent.id, parent.id
                                )
                            }
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            }
        };

        // Check if commit nudge is enabled
        let commit_nudge = config.tasks().commit_nudge_on_close;
        let commit_nudge_msg =
            if commit_nudge && worktree_msg.is_empty() && epic_close_msg.is_empty() {
                "\n\n💡 Consider committing your changes for this completed task."
            } else {
                ""
            };

        let auto_unblock_msg = if auto_unblocked_tasks.is_empty() {
            String::new()
        } else {
            format!(
                "\n\n🔓 Auto-unblocked task(s): {}",
                auto_unblocked_tasks.join(", ")
            )
        };

        Ok(Self::success(format!(
            "Closed task: {} - {}{}{}{}{}{}{}",
            req.id,
            task.title,
            verification_note,
            lease_msg,
            worktree_msg,
            epic_close_msg,
            commit_nudge_msg,
            auto_unblock_msg
        )))
    }

    /// Compute why (if at all) the task-verifier step should be skipped
    /// for this close attempt.
    ///
    /// Only invoked after the caller has been identified as a supervisor
    /// and `verification_enabled` is true — the `VerificationSkipReason::None`
    /// cases here represent "supervisor is closing, but the assignee is
    /// still alive and no bypass flag was set, so run the verifier".
    ///
    /// Resolution order:
    ///
    /// 1. No assignee at all → `NoAssignee`.
    /// 2. Consult the task's active lease via `agent_store.get_lease`.
    ///    `TaskLease.agent_id` is the real session-id even when
    ///    `task.assignee` stores a display name, so this is the most
    ///    reliable liveness source. If the lease is valid and the
    ///    referenced agent is alive+fresh → not a skip (unless the
    ///    supervisor passed `bypass_code_review=true`, in which case
    ///    we honor it as `SupervisorBypass`). If the lease is stale or
    ///    the referenced agent is dead → `AssigneeInactive`.
    /// 3. No lease — try a direct `agent_store.get(task.assignee)` for
    ///    legacy tasks whose assignee field may hold an agent_id. Same
    ///    liveness logic as above.
    /// 4. Everything failed → `AssigneeUnknown` (never falsely reported
    ///    as "assignee inactive" — the agent row is simply missing).
    pub(crate) fn compute_verification_skip_reason(
        &self,
        task: &cas_types::Task,
        req: &TaskCloseRequest,
    ) -> VerificationSkipReason {
        let Some(assignee) = task.assignee.as_deref() else {
            return VerificationSkipReason::NoAssignee;
        };

        let Ok(agent_store) = self.open_agent_store() else {
            // Can't reach the agent store at all — be conservative and
            // let verification run (None is the safe default).
            return VerificationSkipReason::None;
        };

        let bypass_requested = req.bypass_code_review.unwrap_or(false);
        let alive_result = |agent: &cas_types::Agent| {
            agent.is_alive() && !agent.is_heartbeat_expired(ASSIGNEE_STALE_SECS)
        };
        let stale_minutes = |agent: &cas_types::Agent| {
            chrono::Utc::now()
                .signed_duration_since(agent.last_heartbeat)
                .num_minutes()
        };

        // 1) Lease-based path. TaskLease.agent_id always holds the real
        //    session id, so this survives the name-vs-id mismatch that
        //    broke the pre-cas-3bd4 path.
        if let Ok(Some(lease)) = agent_store.get_lease(&task.id) {
            if lease.is_valid() {
                if let Ok(agent) = agent_store.get(&lease.agent_id) {
                    return if alive_result(&agent) {
                        if bypass_requested {
                            VerificationSkipReason::SupervisorBypass
                        } else {
                            VerificationSkipReason::None
                        }
                    } else {
                        VerificationSkipReason::AssigneeInactive {
                            minutes_stale: Some(stale_minutes(&agent)),
                        }
                    };
                }
                // Lease is valid but the referenced agent row is gone —
                // agent was unregistered but the lease wasn't cleaned up.
                return VerificationSkipReason::AssigneeUnknown;
            }
            // Lease exists but expired.
            return VerificationSkipReason::AssigneeInactive {
                minutes_stale: None,
            };
        }

        // 2) No lease — try the legacy direct-id lookup. Works only when
        //    task.assignee holds an agent_id, not a display name.
        if let Ok(agent) = agent_store.get(assignee) {
            return if alive_result(&agent) {
                if bypass_requested {
                    VerificationSkipReason::SupervisorBypass
                } else {
                    VerificationSkipReason::None
                }
            } else {
                VerificationSkipReason::AssigneeInactive {
                    minutes_stale: Some(stale_minutes(&agent)),
                }
            };
        }

        // 3) No lease, no matching agent row. The assignee is unknown
        //    to the store — do not falsely report "inactive".
        VerificationSkipReason::AssigneeUnknown
    }

    /// Reopen a closed task
    pub async fn cas_task_reopen(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        let mut task = task_store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Task not found: {e}")),
            data: None,
        })?;

        if task.status != TaskStatus::Closed {
            return Err(Self::error(
                ErrorCode::INVALID_PARAMS,
                format!(
                    "Task is already {} (only closed tasks can be reopened)",
                    task.status
                ),
            ));
        }

        task.status = TaskStatus::Open;
        task.closed_at = None;
        task.updated_at = chrono::Utc::now();

        task_store.update(&task).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Reopened task: {} - {}",
            req.id, task.title
        )))
    }
}

/// A single additive-only violation: a file whose git status indicates it
/// was modified, deleted, or renamed relative to HEAD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdditiveOnlyViolation {
    pub status: String,
    pub path: String,
}

/// Check whether the working tree at `project_root` contains any files that
/// violate the `additive-only` execution posture. A violation is any file
/// whose `git diff --name-status HEAD` (committed-against-HEAD plus
/// uncommitted) reports a status starting with `M`, `D`, or `R`.
///
/// Only new files (status `A`) are allowed under additive-only. `??`
/// untracked files are ignored because they are additive by definition.
///
/// If `project_root` is not a git repository, or git fails for any reason,
/// this returns an empty vec so the gate never blocks closes in contexts
/// where it cannot reason about the diff. The backstop is advisory when git
/// state is unknowable.
pub(crate) fn check_additive_only_violations(
    project_root: &std::path::Path,
) -> Vec<AdditiveOnlyViolation> {
    use std::process::Command;

    let mut violations = Vec::new();

    // 1. Uncommitted (staged + unstaged) changes vs HEAD.
    if let Ok(output) = Command::new("git")
        .args(["diff", "--name-status", "HEAD"])
        .current_dir(project_root)
        .output()
    {
        if output.status.success() {
            violations.extend(parse_name_status(&String::from_utf8_lossy(&output.stdout)));
        } else {
            // Not a git repo, or HEAD doesn't exist — treat as no-op.
            return Vec::new();
        }
    } else {
        return Vec::new();
    }

    violations
}

// ---------------------------------------------------------------------------
// cas-b39f (Unit 9): cas-code-review P0 close gate
// ---------------------------------------------------------------------------

/// Outcome of the cas-code-review close gate, as seen by `cas_task_close`.
///
/// This enum is deliberately tiny: the hard work (P0 residual evaluation)
/// lives in `cas_store::code_review::close_gate::evaluate_gate`, and the
/// soft conditions (supervisor override, additive-only skip, non-code
/// diff, graceful degradation) are resolved by [`run_code_review_gate`]
/// below. The call site in `cas_task_close` just pattern-matches on the
/// three outcomes.
#[derive(Debug)]
pub(crate) enum CodeReviewGateOutcome {
    /// Close may proceed. No note to write, no error to return.
    Proceed,
    /// Close may proceed, but the caller should append this decision
    /// note to the task before the main close transaction. Used for
    /// the supervisor override path so the audit trail captures who
    /// downgraded a P0 block and why.
    AppendDecisionNote(String),
    /// Close must be rejected with this user-facing error message.
    /// Used for (a) P0 residual blocks, and (b) unauthorized override
    /// attempts.
    Reject(String),
}

/// Decide whether the cas-code-review P0 close gate fires for this
/// close request.
///
/// Per brainstorm Outstanding Question #1 option (a): the worker runs
/// the cas-code-review skill *before* calling `task.close` and passes
/// the structured findings envelope in via
/// [`TaskCloseRequest::code_review_findings`]. This Rust helper only
/// enforces the gate on what the worker sends — it does not (and
/// cannot) invoke the skill itself.
///
/// Contract:
///
/// - `execution_note == "additive-only"` → [`Proceed`]. Pure-addition
///   closes are new-files-only by definition and already covered by
///   the cas-e235 gate above.
/// - `bypass_code_review == Some(true)` and caller is a supervisor →
///   [`AppendDecisionNote`] with the override reason. Gate skipped.
/// - `bypass_code_review == Some(true)` and caller is **not** a
///   supervisor → [`Reject`] with an unauthorized-override message.
///   Silently ignoring the flag would mask a misconfigured harness.
/// - `has_reviewable_changes(project_root) == false` → [`Proceed`].
///   Pure docs-only diffs (`*.md` / `docs/**`) and pure test-only
///   diffs do not require a code review pass.
/// - `code_review_findings == None` at this point → [`Reject`] with
///   `CODE_REVIEW_REQUIRED`, pointing the worker at the skill.
/// - `code_review_findings == Some(envelope)` that fails
///   [`ReviewOutcome::validate`] → [`Reject`] as a malformed envelope.
/// - Otherwise → defer to
///   [`cas_store::code_review::close_gate::evaluate_gate`]. Any
///   non-pre-existing P0 in `residual` → [`Reject`] with a formatted
///   block message; else [`Proceed`].
pub(crate) fn run_code_review_gate(
    task: &Task,
    req: &TaskCloseRequest,
    project_root: &std::path::Path,
) -> CodeReviewGateOutcome {
    // Skip 1: additive-only tasks bypass the gate entirely.
    if task.execution_note.as_deref() == Some("additive-only") {
        return CodeReviewGateOutcome::Proceed;
    }

    // Skip 2: supervisor override.
    if req.bypass_code_review.unwrap_or(false) {
        if is_supervisor_from_env() {
            let reason = req
                .reason
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("(no reason provided)");
            let note = format!(
                "[{}] DECISION: cas-code-review P0 gate overridden by supervisor. \
                 Reason: {}",
                chrono::Utc::now().format("%Y-%m-%d %H:%M"),
                reason
            );
            return CodeReviewGateOutcome::AppendDecisionNote(note);
        } else {
            return CodeReviewGateOutcome::Reject(
                "⚠️ UNAUTHORIZED OVERRIDE\n\n\
                 task close rejected: bypass_code_review=true is only honored \
                 when the caller runs as a supervisor (CAS_AGENT_ROLE=supervisor). \
                 Non-supervisor callers must either fix the P0 findings and retry \
                 close, or ask a supervisor to issue the override."
                    .to_string(),
            );
        }
    }

    // Skip 3: docs-only / test-only / empty diffs. The gate is not a
    // SPOF for changes it cannot meaningfully review.
    if !has_reviewable_changes(project_root) {
        return CodeReviewGateOutcome::Proceed;
    }

    // From here on, we require a findings envelope.
    let envelope_json = match req.code_review_findings.as_deref() {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return CodeReviewGateOutcome::Reject(
                "⚠️ CODE_REVIEW_REQUIRED\n\n\
                 task close rejected: this task has reviewable code changes \
                 and no code_review_findings envelope was provided.\n\n\
                 To resolve:\n\
                 1. Invoke the cas-code-review skill via the Skill or Task \
                    tool with mode=autofix and the current diff.\n\
                 2. Collect the returned ReviewOutcome envelope (residual, \
                    pre_existing, mode).\n\
                 3. Re-call task.close with the envelope JSON-stringified \
                    in code_review_findings.\n\n\
                 Supervisors may bypass this gate with \
                 bypass_code_review=true (logged as a decision note)."
                    .to_string(),
            );
        }
    };

    let envelope: cas_types::ReviewOutcome = match serde_json::from_str(envelope_json) {
        Ok(e) => e,
        Err(e) => {
            return CodeReviewGateOutcome::Reject(format!(
                "⚠️ MALFORMED REVIEW ENVELOPE\n\n\
                 task close rejected: code_review_findings failed to parse \
                 as ReviewOutcome JSON: {e}\n\n\
                 Expected shape: {{residual: Finding[], pre_existing: Finding[], mode: string}}."
            ));
        }
    };

    if let Err(e) = envelope.validate() {
        return CodeReviewGateOutcome::Reject(format!(
            "⚠️ MALFORMED REVIEW ENVELOPE\n\n\
             task close rejected: code_review_findings failed validation: {e}\n\n\
             The worker-side cas-code-review skill returned a structurally \
             invalid envelope. Re-run the review and retry close."
        ));
    }

    use cas_store::code_review::close_gate::{GateDecision, evaluate_gate, format_block_message};
    match evaluate_gate(&envelope.residual) {
        GateDecision::Allow => CodeReviewGateOutcome::Proceed,
        GateDecision::BlockOnP0(blocking) => {
            CodeReviewGateOutcome::Reject(format_block_message(&task.id, &blocking))
        }
    }
}

/// Return `true` if `project_root` has any staged, unstaged, or
/// committed-since-base changes in files that are worth asking the
/// multi-persona reviewer about. Returns `false` for docs-only
/// (`*.md`, anything under `docs/`) and test-only diffs, and for
/// non-git directories where we cannot reason about the diff.
///
/// The classification is deliberately *loose*: when we cannot tell
/// whether a change is reviewable, we assume it is, and the worker
/// runs the review. False positives waste latency; false negatives
/// silently skip the gate.
pub(crate) fn has_reviewable_changes(project_root: &std::path::Path) -> bool {
    use std::process::Command;

    // Collect changed paths from both the index/working-tree diff and
    // the HEAD diff. Union handles in-flight edits on top of the
    // already-committed task work.
    let mut changed: Vec<String> = Vec::new();

    for args in [
        &["diff", "--name-only", "HEAD"][..],
        &["diff", "--name-only", "--cached"][..],
    ] {
        if let Ok(output) = Command::new("git")
            .args(args)
            .current_dir(project_root)
            .output()
        {
            if !output.status.success() {
                // Not a git repo, or HEAD doesn't exist — we cannot
                // reason about the diff, so the gate should not block.
                // Per the "not a SPOF" rule, treat as no-reviewable.
                return false;
            }
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    changed.push(trimmed.to_string());
                }
            }
        } else {
            return false;
        }
    }

    changed.sort();
    changed.dedup();

    changed.iter().any(|path| is_reviewable_path(path))
}

/// Classify a single path as "worth running the multi-persona
/// reviewer on". Docs (`*.md`, anything under `docs/`) and tests
/// (anything under `tests/`, `test/`, or a file ending in
/// `_test.rs` / `.test.ts`) are excluded.
pub(crate) fn is_reviewable_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();

    // Docs
    if lower.ends_with(".md") {
        return false;
    }
    if lower.starts_with("docs/") || lower.contains("/docs/") {
        return false;
    }

    // Tests
    if lower.starts_with("tests/") || lower.contains("/tests/") {
        return false;
    }
    if lower.starts_with("test/") || lower.contains("/test/") {
        return false;
    }
    if lower.ends_with("_test.rs")
        || lower.ends_with(".test.ts")
        || lower.ends_with(".test.tsx")
        || lower.ends_with(".spec.ts")
        || lower.ends_with(".spec.tsx")
        || lower.ends_with("_test.py")
        || lower.ends_with("_test.go")
    {
        return false;
    }

    true
}

/// Parse the output of `git diff --name-status` into violations. Only rows
/// whose status starts with M, D, or R are returned. A, C, T, U, and ?? are
/// considered additive or uninteresting.
fn parse_name_status(output: &str) -> Vec<AdditiveOnlyViolation> {
    let mut violations = Vec::new();
    for line in output.lines() {
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        // Format: "<STATUS>\t<PATH>" or for renames "R100\t<OLD>\t<NEW>"
        let mut parts = line.splitn(3, '\t');
        let Some(status) = parts.next() else {
            continue;
        };
        let Some(first_path) = parts.next() else {
            continue;
        };
        let second_path = parts.next();
        let first_char = status.chars().next().unwrap_or(' ');
        match first_char {
            'M' | 'D' => violations.push(AdditiveOnlyViolation {
                status: status.to_string(),
                path: first_path.to_string(),
            }),
            'R' => {
                let path = second_path.unwrap_or(first_path).to_string();
                violations.push(AdditiveOnlyViolation {
                    status: status.to_string(),
                    path,
                });
            }
            _ => {}
        }
    }
    violations
}

#[cfg(test)]
mod additive_only_tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "test")
            .env("GIT_AUTHOR_EMAIL", "test@test")
            .env("GIT_COMMITTER_NAME", "test")
            .env("GIT_COMMITTER_EMAIL", "test@test")
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        let p = dir.path();
        git(p, &["init", "-q", "-b", "main"]);
        std::fs::write(p.join("existing.txt"), "original\n").unwrap();
        git(p, &["add", "existing.txt"]);
        git(p, &["commit", "-q", "-m", "initial"]);
        dir
    }

    #[test]
    fn non_git_dir_returns_no_violations() {
        let dir = tempdir().unwrap();
        assert!(check_additive_only_violations(dir.path()).is_empty());
    }

    #[test]
    fn clean_repo_returns_no_violations() {
        let dir = init_repo();
        assert!(check_additive_only_violations(dir.path()).is_empty());
    }

    #[test]
    fn new_file_is_not_a_violation() {
        let dir = init_repo();
        std::fs::write(dir.path().join("new.txt"), "hello\n").unwrap();
        git(dir.path(), &["add", "new.txt"]);
        let v = check_additive_only_violations(dir.path());
        assert!(v.is_empty(), "new file must be allowed, got: {v:?}");
    }

    #[test]
    fn modified_file_is_violation() {
        let dir = init_repo();
        std::fs::write(dir.path().join("existing.txt"), "changed\n").unwrap();
        let v = check_additive_only_violations(dir.path());
        assert_eq!(v.len(), 1);
        assert!(v[0].status.starts_with('M'));
        assert_eq!(v[0].path, "existing.txt");
    }

    #[test]
    fn deleted_file_is_violation() {
        let dir = init_repo();
        std::fs::remove_file(dir.path().join("existing.txt")).unwrap();
        let v = check_additive_only_violations(dir.path());
        assert_eq!(v.len(), 1);
        assert!(v[0].status.starts_with('D'));
        assert_eq!(v[0].path, "existing.txt");
    }

    #[test]
    fn renamed_file_is_violation() {
        let dir = init_repo();
        git(dir.path(), &["mv", "existing.txt", "renamed.txt"]);
        let v = check_additive_only_violations(dir.path());
        assert_eq!(v.len(), 1);
        assert!(v[0].status.starts_with('R'));
        assert_eq!(v[0].path, "renamed.txt");
    }

    #[test]
    fn parse_name_status_mixed() {
        let out = "A\tadded.txt\nM\tmodified.txt\nD\tdeleted.txt\nR100\told.txt\tnew.txt\n";
        let v = parse_name_status(out);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].path, "modified.txt");
        assert_eq!(v[1].path, "deleted.txt");
        assert_eq!(v[2].path, "new.txt");
        assert!(v[2].status.starts_with('R'));
    }
}

#[cfg(test)]
mod code_review_gate_tests {
    //! Unit tests for the cas-b39f close gate helper. Covers the full
    //! decision matrix in [`run_code_review_gate`] under the option-(a)
    //! architecture where the worker passes findings in via
    //! `TaskCloseRequest.code_review_findings` before retrying close.
    //!
    //! The pure-Rust decision helper at
    //! `cas_store::code_review::close_gate::evaluate_gate` is already
    //! tested exhaustively in that module; these tests focus on the
    //! close-side glue — env role check, envelope plumbing, override
    //! path, docs-only skip, CODE_REVIEW_REQUIRED rejection.
    use super::*;
    use cas_types::{AutofixClass, Finding, FindingSeverity, Owner, ReviewOutcome};
    use tempfile::TempDir;

    fn base_task() -> Task {
        Task {
            id: "cas-test1".to_string(),
            title: "test".to_string(),
            status: TaskStatus::InProgress,
            ..Default::default()
        }
    }

    fn base_req(id: &str) -> TaskCloseRequest {
        TaskCloseRequest {
            id: id.to_string(),
            reason: None,
            bypass_code_review: None,
            code_review_findings: None,
        }
    }

    fn p0_finding() -> Finding {
        Finding {
            title: "SQL injection".to_string(),
            severity: FindingSeverity::P0,
            file: "src/auth.rs".to_string(),
            line: 42,
            why_it_matters: "allows login bypass".to_string(),
            autofix_class: AutofixClass::Manual,
            owner: Owner::Human,
            confidence: 0.95,
            evidence: vec!["format!(\"... {}\", user_input)".to_string()],
            pre_existing: false,
            suggested_fix: None,
            requires_verification: false,
        }
    }

    fn p2_finding() -> Finding {
        Finding {
            title: "dead import".to_string(),
            severity: FindingSeverity::P2,
            file: "src/lib.rs".to_string(),
            line: 3,
            why_it_matters: "minor".to_string(),
            autofix_class: AutofixClass::Manual,
            owner: Owner::ReviewFixer,
            confidence: 0.9,
            evidence: vec!["use foo::bar;".to_string()],
            pre_existing: false,
            suggested_fix: None,
            requires_verification: false,
        }
    }

    fn autofix_envelope(residual: Vec<Finding>) -> String {
        let env = ReviewOutcome {
            residual,
            pre_existing: Vec::new(),
            mode: "autofix".to_string(),
        };
        serde_json::to_string(&env).expect("serialize ReviewOutcome")
    }

    /// Build a throwaway git repo with one committed file, then stage
    /// whatever paths the caller names so `git diff --cached` sees
    /// them. Returns the tempdir so the caller controls its lifetime.
    fn repo_with_staged(paths: &[(&str, &str)]) -> TempDir {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path();
        use std::process::Command;
        let git = |args: &[&str]| {
            let ok = Command::new("git")
                .args(args)
                .current_dir(p)
                .env("GIT_AUTHOR_NAME", "t")
                .env("GIT_AUTHOR_EMAIL", "t@t")
                .env("GIT_COMMITTER_NAME", "t")
                .env("GIT_COMMITTER_EMAIL", "t@t")
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .status()
                .expect("git")
                .success();
            assert!(ok, "git {args:?} failed");
        };
        git(&["init", "-q", "-b", "main"]);
        std::fs::write(p.join("seed.txt"), "seed\n").unwrap();
        git(&["add", "seed.txt"]);
        git(&["commit", "-q", "-m", "seed"]);
        for (path, contents) in paths {
            let full = p.join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(&full, contents).unwrap();
            git(&["add", path]);
        }
        dir
    }

    /// Serialize env-mutating tests so `CAS_AGENT_ROLE` changes don't
    /// leak between them.
    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        use std::sync::{Mutex, OnceLock};
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    // --- Path classification ------------------------------------------------

    #[test]
    fn docs_and_tests_are_not_reviewable() {
        assert!(!is_reviewable_path("README.md"));
        assert!(!is_reviewable_path("docs/foo.txt"));
        assert!(!is_reviewable_path("crates/cas-store/tests/foo.rs"));
        assert!(!is_reviewable_path("src/foo_test.rs"));
        assert!(!is_reviewable_path("app/bar.test.tsx"));
        assert!(!is_reviewable_path("tests/integration.py"));
    }

    #[test]
    fn code_files_are_reviewable() {
        assert!(is_reviewable_path("src/main.rs"));
        assert!(is_reviewable_path("app/login.ts"));
        assert!(is_reviewable_path("pkg/server/handler.go"));
    }

    // --- run_code_review_gate branches --------------------------------------

    #[test]
    fn additive_only_task_bypasses_gate() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/evil.rs", "bad\n")]);
        let mut t = base_task();
        t.execution_note = Some("additive-only".to_string());
        let mut req = base_req(&t.id);
        req.code_review_findings = Some(autofix_envelope(vec![p0_finding()]));
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(matches!(out, CodeReviewGateOutcome::Proceed));
    }

    #[test]
    fn docs_only_diff_skips_gate_without_findings() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("README.md", "new content\n"), ("docs/x.md", "x\n")]);
        let t = base_task();
        let req = base_req(&t.id); // no findings
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(
            matches!(out, CodeReviewGateOutcome::Proceed),
            "pure-docs diff must skip the review gate"
        );
    }

    #[test]
    fn code_change_without_findings_is_rejected_as_required() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn new() {}\n")]);
        let t = base_task();
        let req = base_req(&t.id);
        let out = run_code_review_gate(&t, &req, dir.path());
        match out {
            CodeReviewGateOutcome::Reject(msg) => {
                assert!(msg.contains("CODE_REVIEW_REQUIRED"));
                assert!(msg.contains("cas-code-review"));
                assert!(msg.contains("code_review_findings"));
            }
            other => panic!("expected CODE_REVIEW_REQUIRED reject, got {other:?}"),
        }
    }

    #[test]
    fn p0_residual_blocks_close() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn new() {}\n")]);
        let t = base_task();
        let mut req = base_req(&t.id);
        req.code_review_findings = Some(autofix_envelope(vec![p0_finding()]));
        let out = run_code_review_gate(&t, &req, dir.path());
        match out {
            CodeReviewGateOutcome::Reject(msg) => {
                assert!(msg.contains("P0 BLOCK"));
                assert!(msg.contains("SQL injection"));
                assert!(msg.contains("bypass_code_review=true"));
            }
            other => panic!("expected P0 block, got {other:?}"),
        }
    }

    #[test]
    fn p2_residual_does_not_block_close() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn new() {}\n")]);
        let t = base_task();
        let mut req = base_req(&t.id);
        req.code_review_findings = Some(autofix_envelope(vec![p2_finding()]));
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(
            matches!(out, CodeReviewGateOutcome::Proceed),
            "P2 residual must route to Unit 8, not block close"
        );
    }

    #[test]
    fn empty_residual_with_envelope_allows_close() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn ok() {}\n")]);
        let t = base_task();
        let mut req = base_req(&t.id);
        req.code_review_findings = Some(autofix_envelope(Vec::new()));
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(matches!(out, CodeReviewGateOutcome::Proceed));
    }

    #[test]
    fn malformed_envelope_validation_failure_is_rejected() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn ok() {}\n")]);
        let t = base_task();
        let mut req = base_req(&t.id);
        // Whitespace-only mode passes serde but fails validate().
        req.code_review_findings = Some(
            r#"{"residual":[],"pre_existing":[],"mode":"   "}"#.to_string(),
        );
        let out = run_code_review_gate(&t, &req, dir.path());
        match out {
            CodeReviewGateOutcome::Reject(msg) => {
                assert!(msg.contains("MALFORMED REVIEW ENVELOPE"));
            }
            other => panic!("expected malformed-envelope reject, got {other:?}"),
        }
    }

    #[test]
    fn unparseable_envelope_json_is_rejected() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn ok() {}\n")]);
        let t = base_task();
        let mut req = base_req(&t.id);
        req.code_review_findings = Some("not json at all".to_string());
        let out = run_code_review_gate(&t, &req, dir.path());
        match out {
            CodeReviewGateOutcome::Reject(msg) => {
                assert!(msg.contains("MALFORMED REVIEW ENVELOPE"));
                assert!(msg.contains("failed to parse"));
            }
            other => panic!("expected parse reject, got {other:?}"),
        }
    }

    #[test]
    fn supervisor_override_appends_decision_note() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn new() {}\n")]);
        let prev = std::env::var("CAS_AGENT_ROLE").ok();
        unsafe {
            std::env::set_var("CAS_AGENT_ROLE", "supervisor");
        }

        let t = base_task();
        let mut req = base_req(&t.id);
        req.bypass_code_review = Some(true);
        req.reason = Some("P0 is a false positive, tracked in cas-xyz".to_string());

        let out = run_code_review_gate(&t, &req, dir.path());

        unsafe {
            match prev {
                Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
                None => std::env::remove_var("CAS_AGENT_ROLE"),
            }
        }

        match out {
            CodeReviewGateOutcome::AppendDecisionNote(note) => {
                assert!(note.contains("DECISION"));
                assert!(note.contains("supervisor"));
                assert!(note.contains("false positive"));
            }
            other => panic!("expected AppendDecisionNote, got {other:?}"),
        }
    }

    #[test]
    fn non_supervisor_override_is_rejected() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/foo.rs", "fn new() {}\n")]);
        let prev = std::env::var("CAS_AGENT_ROLE").ok();
        unsafe {
            std::env::set_var("CAS_AGENT_ROLE", "worker");
        }

        let t = base_task();
        let mut req = base_req(&t.id);
        req.bypass_code_review = Some(true);

        let out = run_code_review_gate(&t, &req, dir.path());

        unsafe {
            match prev {
                Some(v) => std::env::set_var("CAS_AGENT_ROLE", v),
                None => std::env::remove_var("CAS_AGENT_ROLE"),
            }
        }

        match out {
            CodeReviewGateOutcome::Reject(msg) => {
                assert!(msg.contains("UNAUTHORIZED OVERRIDE"));
            }
            other => panic!("expected Reject, got {other:?}"),
        }
    }

    #[test]
    fn additive_only_plus_missing_findings_still_proceeds() {
        let _g = env_lock();
        let dir = repo_with_staged(&[("src/evil.rs", "bad\n")]);
        let mut t = base_task();
        t.execution_note = Some("additive-only".to_string());
        let req = base_req(&t.id); // no findings, no override
        // additive-only short-circuits before the findings check.
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(matches!(out, CodeReviewGateOutcome::Proceed));
    }

    #[test]
    fn non_git_project_root_skips_gate() {
        let _g = env_lock();
        let dir = tempfile::tempdir().unwrap();
        let t = base_task();
        let req = base_req(&t.id);
        // Non-git dir → has_reviewable_changes returns false → skip.
        let out = run_code_review_gate(&t, &req, dir.path());
        assert!(matches!(out, CodeReviewGateOutcome::Proceed));
    }
}
