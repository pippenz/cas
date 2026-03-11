use crate::harness_policy::{
    is_supervisor_from_env, is_worker_without_subagents_from_env, supervisor_harness_from_env,
    verification_policy, worker_harness_from_env,
};
use crate::mcp::tools::core::imports::*;

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

        if verification_enabled {
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

                match latest {
                    Ok(Some(v)) if v.status == VerificationStatus::Approved => {
                        // Verification approved, proceed with close
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

                        let verification_gate = if is_factory_worker {
                            "Factory worker flow: verification is pending. Continue with other assigned tasks while waiting."
                                .to_string()
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

        let verification_note = if verification_enabled {
            " (verified)"
        } else {
            ""
        };

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
                                format!(
                                    "\n\n🎉 All subtasks of epic '{}' ({}) are now complete!\n\
                                     → The supervisor will be notified to close the epic.",
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
