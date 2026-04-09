use crate::mcp::tools::core::imports::*;

impl CasCore {
    pub async fn cas_task_update(
        &self,
        Parameters(req): Parameters<TaskUpdateRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        let mut task = task_store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Task not found: {e}")),
            data: None,
        })?;

        let mut changes = Vec::new();

        if let Some(title) = req.title {
            task.title = title;
            changes.push("title");
        }

        if let Some(notes) = req.notes {
            let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M");
            let note_with_timestamp = format!("[{timestamp}] {notes}");
            if task.notes.is_empty() {
                task.notes = note_with_timestamp;
            } else {
                task.notes = format!("{}\n\n{}", task.notes, note_with_timestamp);
            }
            changes.push("notes");
        }

        if let Some(priority) = req.priority {
            task.priority = Priority(priority.min(4) as i32);
            changes.push("priority");
        }

        if let Some(labels) = req.labels {
            for label in labels
                .split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                if !task.labels.contains(&label.to_string()) {
                    task.labels.push(label.to_string());
                }
            }
            changes.push("labels");
        }

        if let Some(description) = req.description {
            task.description = description;
            changes.push("description");
        }

        if let Some(design) = req.design {
            task.design = design;
            changes.push("design");
        }

        if let Some(acceptance_criteria) = req.acceptance_criteria {
            task.acceptance_criteria = acceptance_criteria;
            changes.push("acceptance_criteria");
        }

        if let Some(demo_statement) = req.demo_statement {
            task.demo_statement = demo_statement;
            changes.push("demo_statement");
        }

        if let Some(raw) = req.execution_note.as_deref() {
            let validated = crate::mcp::tools::types::validate_execution_note(Some(raw))
                .map_err(|msg| McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(msg),
                    data: None,
                })?;
            task.execution_note = validated;
            changes.push("execution_note");
        }

        if let Some(external_ref) = req.external_ref {
            task.external_ref = Some(external_ref);
            changes.push("external_ref");
        }

        // Track warnings to include in response
        let mut warnings: Vec<String> = Vec::new();

        if let Some(ref assignee) = req.assignee {
            // Check worktree staleness when assigning in factory mode
            if std::env::var("CAS_FACTORY_MODE").is_ok() {
                let config = self.load_config();
                let factory_config = config.factory();

                if factory_config.warn_stale_assignment || factory_config.block_stale_assignment {
                    // Look up the worker agent by name to get their clone_path
                    if let Ok(agent_store) = self.open_agent_store() {
                        if let Ok(agents) = agent_store.list(None) {
                            if let Some(worker) = agents.iter().find(|a| &a.name == assignee) {
                                if let Some(clone_path) = worker.metadata.get("clone_path") {
                                    if let Some((behind_count, branch)) =
                                        check_worktree_staleness(clone_path)
                                    {
                                        if behind_count > 0 {
                                            let warning_msg = format!(
                                                "⚠️ Worker '{assignee}' is {behind_count} commit(s) behind {branch}. Consider syncing first."
                                            );

                                            // Block if configured and above threshold
                                            if factory_config.block_stale_assignment
                                                && behind_count
                                                    >= factory_config.stale_threshold_commits
                                            {
                                                return Err(McpError {
                                                    code: ErrorCode::INVALID_PARAMS,
                                                    message: Cow::from(format!(
                                                        "Cannot assign to worker '{}': {} commits behind {} (threshold: {}). Ask the worker to rebase: `git rebase {}`",
                                                        assignee,
                                                        behind_count,
                                                        branch,
                                                        factory_config.stale_threshold_commits,
                                                        branch
                                                    )),
                                                    data: None,
                                                });
                                            }

                                            warnings.push(warning_msg);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            task.assignee = Some(assignee.clone());
            changes.push("assignee");
        }

        if let Some(owner) = req.epic_verification_owner {
            task.epic_verification_owner = Some(owner);
            changes.push("epic_verification_owner");
        }

        if let Some(status_str) = req.status {
            use std::str::FromStr;
            let new_status =
                cas_types::TaskStatus::from_str(&status_str).map_err(|_| McpError {
                    code: ErrorCode::INVALID_PARAMS,
                    message: Cow::from(format!(
                        "Invalid status: {status_str}. Valid: open, in_progress, closed, blocked"
                    )),
                    data: None,
                })?;
            task.status = new_status;
            changes.push("status");
        }

        // Handle epic association change
        if let Some(epic_id) = req.epic {
            let epic_id = epic_id.trim();
            let existing_parent_deps: Vec<Dependency> = task_store
                .get_dependencies(&req.id)
                .map_err(|e| McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!("Failed to read dependencies: {e}")),
                    data: None,
                })?
                .into_iter()
                .filter(|dep| dep.dep_type == DependencyType::ParentChild)
                .collect();

            // Validate requested epic first so we don't drop existing relationships on failure.
            if !epic_id.is_empty() {
                match task_store.get(epic_id) {
                    Ok(epic_task) => {
                        if epic_task.task_type != TaskType::Epic {
                            return Err(McpError {
                                code: ErrorCode::INVALID_PARAMS,
                                message: Cow::from(format!(
                                    "Task {} is not an epic (type: {})",
                                    epic_id, epic_task.task_type
                                )),
                                data: None,
                            });
                        }
                    }
                    Err(_) => {
                        return Err(McpError {
                            code: ErrorCode::INVALID_PARAMS,
                            message: Cow::from(format!("Epic not found: {epic_id}")),
                            data: None,
                        });
                    }
                }
            }

            // Remove existing ParentChild dependency only after validation succeeded.
            for dep in existing_parent_deps {
                task_store
                    .remove_dependency(&req.id, &dep.to_id)
                    .map_err(|e| McpError {
                        code: ErrorCode::INTERNAL_ERROR,
                        message: Cow::from(format!(
                            "Failed to remove existing epic dependency: {e}"
                        )),
                        data: None,
                    })?;
            }

            // Add new ParentChild dependency if epic_id is not empty.
            if !epic_id.is_empty() {
                let dep = Dependency {
                    from_id: req.id.clone(),
                    to_id: epic_id.to_string(),
                    dep_type: DependencyType::ParentChild,
                    created_at: chrono::Utc::now(),
                    created_by: Some("mcp".to_string()),
                };
                task_store.add_dependency(&dep).map_err(|e| McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!("Failed to add epic dependency: {e}")),
                    data: None,
                })?;
            }
            changes.push("epic");
        }

        if changes.is_empty() {
            return Ok(Self::success("No changes specified"));
        }

        task.updated_at = chrono::Utc::now();

        task_store.update(&task).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        // Build response message with warnings if any
        let mut response = format!("Updated task {}: {}", req.id, changes.join(", "));
        if !warnings.is_empty() {
            response = format!("{}\n\n{}", response, warnings.join("\n"));
        }

        Ok(Self::success(response))
    }
}
