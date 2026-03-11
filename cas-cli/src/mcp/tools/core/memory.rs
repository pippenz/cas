use crate::mcp::tools::core::imports::*;

impl CasCore {
    /// Create a new CAS service with daemon support
    pub fn with_daemon(
        cas_root: std::path::PathBuf,
        activity: Option<std::sync::Arc<ActivityTracker>>,
        daemon: Option<std::sync::Arc<EmbeddedDaemon>>,
    ) -> Self {
        Self {
            cas_root,
            activity,
            daemon,
            agent_id: std::sync::OnceLock::new(),
            peer: std::sync::Arc::new(std::sync::RwLock::new(None)),
            cached_store: std::sync::OnceLock::new(),
            cached_rule_store: std::sync::OnceLock::new(),
            cached_task_store: std::sync::OnceLock::new(),
            cached_skill_store: std::sync::OnceLock::new(),
            cached_entity_store: std::sync::OnceLock::new(),
            cached_agent_store: std::sync::OnceLock::new(),
            cached_verification_store: std::sync::OnceLock::new(),
            cached_worktree_store: std::sync::OnceLock::new(),
        }
    }

    /// Get the project store path
    pub fn project_path(&self) -> &std::path::Path {
        &self.cas_root
    }

    /// Pre-set the agent ID (for testing where no daemon is running)
    ///
    /// In production, the agent_id is discovered lazily via daemon socket query.
    /// In tests, there's no daemon, so we pre-set the agent_id directly.
    pub fn set_agent_id_for_testing(&self, agent_id: String) {
        let _ = self.agent_id.set(Some(agent_id));
    }

    // ========================================================================
    // Workflow Guidance (injected on task start/claim)
    // ========================================================================

    /// Generate workflow guidance to show when starting or claiming a task
    pub(super) fn workflow_guidance() -> String {
        "\n\n📋 Workflow Guidance:\n\
         • Search: `mcp__cas__search` for exploratory queries, Grep for exact patterns\n\
         • Progress: `mcp__cas__task action: notes` to track discoveries\n\
         • Learnings: `mcp__cas__memory action: remember` for reusable knowledge"
            .to_string()
    }

    // ========================================================================
    // Memory Tools (12)
    // ========================================================================

    /// Store a new memory
    pub async fn cas_remember(
        &self,
        Parameters(req): Parameters<RememberRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let entry_type: EntryType = req.entry_type.parse().unwrap_or(EntryType::Learning);
        let id = store.generate_id().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to generate ID: {e}")),
            data: None,
        })?;

        let tags: Vec<String> = req
            .tags
            .map(|t| {
                t.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        // Auto-detect branch for worktree scoping
        let branch = self.current_worktree_branch();

        // Parse temporal validity timestamps
        let valid_from = req.valid_from.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .ok()
        });
        let valid_until = req.valid_until.and_then(|s| {
            chrono::DateTime::parse_from_rfc3339(&s)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .ok()
        });

        let entry = Entry {
            id: id.clone(),
            scope: Scope::default(),
            entry_type,
            observation_type: None,
            tags,
            created: chrono::Utc::now(),
            content: req.content,
            raw_content: None,
            compressed: false,
            memory_tier: MemoryTier::Working,
            title: req.title,
            helpful_count: 0,
            harmful_count: 0,
            last_accessed: None,
            archived: false,
            session_id: None,
            source_tool: Some("mcp".to_string()),
            pending_extraction: false,
            pending_embedding: true,
            stability: 0.5,
            access_count: 0,
            importance: req.importance,
            valid_from,
            valid_until,
            review_after: None,
            last_reviewed: None,
            domain: None,
            belief_type: Default::default(),
            confidence: 1.0,
            branch,
            team_id: req.team_id.clone(),
        };

        store.add(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to store entry: {e}")),
            data: None,
        })?;

        if let Ok(search) = self.open_search_index() {
            let _ = search.index_entry(&entry);
        }

        Ok(Self::success(format!("Created entry: {id}")))
    }

    /// Get an entry by ID
    ///
    /// Also tracks access for session-aware context boosting:
    /// - Updates `last_accessed` timestamp
    /// - Increments `access_count`
    /// - Reinforces memory stability
    pub async fn cas_get(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        // Track access for session-aware context boosting
        // reinforce() updates last_accessed, access_count, and stability
        entry.reinforce();

        // Persist access tracking (best-effort, don't fail the get)
        let _ = store.update(&entry);

        let output = format!(
            "ID: {}\nType: {:?}\nTags: {}\nCreated: {}\nImportance: {:.2}\nStability: {:.2}\nFeedback: +{} -{}\n\n{}",
            entry.id,
            entry.entry_type,
            if entry.tags.is_empty() {
                "none".to_string()
            } else {
                entry.tags.join(", ")
            },
            entry.created.format("%Y-%m-%d %H:%M"),
            entry.importance,
            entry.stability,
            entry.helpful_count,
            entry.harmful_count,
            entry.content
        );

        Ok(Self::success(output))
    }

    /// Mark entry as helpful
    pub async fn cas_helpful(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        entry.helpful_count += 1;
        entry.reinforce();
        entry.last_reviewed = Some(chrono::Utc::now());

        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Marked {} as helpful (score: {:.2})",
            req.id,
            entry.feedback_score()
        )))
    }

    /// Mark entry as harmful
    pub async fn cas_harmful(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        entry.harmful_count += 1;

        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Marked {} as harmful (score: {:.2})",
            req.id,
            entry.feedback_score()
        )))
    }

    /// Mark entry as reviewed (sets last_reviewed timestamp)
    pub async fn cas_mark_reviewed(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        entry.last_reviewed = Some(chrono::Utc::now());

        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!("Marked {} as reviewed", req.id)))
    }

    /// List recent entries
    pub async fn cas_recent(
        &self,
        Parameters(req): Parameters<RecentRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        // Fetch more to account for branch filtering
        let all_entries = store.recent(req.n * 3).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to get recent: {e}")),
            data: None,
        })?;

        // Filter by branch context (worktree scoping)
        let current_branch = self.current_worktree_branch();
        let entries: Vec<_> = all_entries
            .into_iter()
            .filter(|e| {
                match (&current_branch, &e.branch) {
                    // In a worktree: show entries from this branch or unscoped entries
                    (Some(cb), Some(eb)) => cb == eb,
                    (Some(_), None) => true, // Unscoped entries visible in all worktrees
                    // Not in a worktree: show all entries
                    (None, _) => true,
                }
            })
            .take(req.n)
            .collect();

        if entries.is_empty() {
            return Ok(Self::success("No entries found"));
        }

        let mut output = format!("Recent entries ({}):\n\n", entries.len());
        for entry in entries {
            let branch_indicator = entry
                .branch
                .as_ref()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default();
            output.push_str(&format!(
                "- [{}] {}{} {}\n",
                entry.id,
                entry.created.format("%Y-%m-%d %H:%M"),
                branch_indicator,
                entry.preview(50)
            ));
        }

        Ok(Self::success(output))
    }

    /// Delete an entry
    pub async fn cas_delete(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        // Verify entry exists
        store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        store.delete(&req.id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to delete: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!("Deleted entry: {}", req.id)))
    }

    /// List all entries
    pub async fn cas_list(
        &self,
        Parameters(req): Parameters<LimitRequest>,
    ) -> Result<CallToolResult, McpError> {
        use cas_types::EntrySortOptions;

        let store = self.open_store()?;

        let all_entries = store.list().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to list: {e}")),
            data: None,
        })?;

        // Filter by branch context (worktree scoping)
        let current_branch = self.current_worktree_branch();
        let mut entries: Vec<_> = all_entries
            .into_iter()
            .filter(|e| {
                match (&current_branch, &e.branch) {
                    // In a worktree: show entries from this branch or unscoped entries
                    (Some(cb), Some(eb)) => cb == eb,
                    (Some(_), None) => true, // Unscoped entries visible in all worktrees
                    // Not in a worktree: show all entries
                    (None, _) => true,
                }
            })
            .collect();

        // Filter by team_id if specified
        if let Some(ref team_id) = req.team_id {
            entries.retain(|e| e.team_id.as_ref() == Some(team_id));
        }

        // Apply sorting
        let sort_opts =
            EntrySortOptions::from_params(req.sort.as_deref(), req.sort_order.as_deref());

        use cas_types::{EntrySortField, SortOrder};
        match sort_opts.field {
            EntrySortField::Created => {
                entries.sort_by(|a, b| match sort_opts.order {
                    SortOrder::Asc => a.created.cmp(&b.created),
                    SortOrder::Desc => b.created.cmp(&a.created),
                });
            }
            EntrySortField::Updated => {
                // Use last_accessed as "updated" time, falling back to created
                entries.sort_by(|a, b| {
                    let a_time = a.last_accessed.as_ref().unwrap_or(&a.created);
                    let b_time = b.last_accessed.as_ref().unwrap_or(&b.created);
                    match sort_opts.order {
                        SortOrder::Asc => a_time.cmp(b_time),
                        SortOrder::Desc => b_time.cmp(a_time),
                    }
                });
            }
            EntrySortField::Importance => {
                entries.sort_by(|a, b| {
                    let cmp = a.importance.total_cmp(&b.importance);
                    match sort_opts.order {
                        SortOrder::Asc => cmp,
                        SortOrder::Desc => cmp.reverse(),
                    }
                });
            }
            EntrySortField::Title => {
                entries.sort_by(|a, b| {
                    let a_title = a.title.as_deref().unwrap_or("");
                    let b_title = b.title.as_deref().unwrap_or("");
                    match sort_opts.order {
                        SortOrder::Asc => a_title.cmp(b_title),
                        SortOrder::Desc => b_title.cmp(a_title),
                    }
                });
            }
        }

        if entries.is_empty() {
            return Ok(Self::success("No entries found"));
        }

        let limit = req.limit.unwrap_or(20);
        let mut output = format!(
            "Entries ({} total, showing {}):\n\n",
            entries.len(),
            entries.len().min(limit)
        );
        for entry in entries.iter().take(limit) {
            let branch_indicator = entry
                .branch
                .as_ref()
                .map(|b| format!(" [{b}]"))
                .unwrap_or_default();
            output.push_str(&format!(
                "- [{}] {:?} {}{} - {}\n",
                entry.id,
                entry.entry_type,
                entry.created.format("%Y-%m-%d"),
                branch_indicator,
                entry.preview(40)
            ));
        }

        if entries.len() > limit {
            output.push_str(&format!("\n... and {} more", entries.len() - limit));
        }

        Ok(Self::success(output))
    }

    /// Archive an entry
    pub async fn cas_archive(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        entry.archived = true;
        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to archive: {e}")),
            data: None,
        })?;

        // Remove from search index
        if let Ok(search) = self.open_search_index() {
            let _ = search.delete(&req.id);
        }

        Ok(Self::success(format!("Archived entry: {}", req.id)))
    }

    /// Unarchive an entry
    pub async fn cas_unarchive(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        // Try to get from archived entries
        let archived = store.list_archived().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to list archived: {e}")),
            data: None,
        })?;

        let mut entry = archived
            .into_iter()
            .find(|e| e.id == req.id)
            .ok_or_else(|| McpError {
                code: ErrorCode::INVALID_PARAMS,
                message: Cow::from(format!("Archived entry not found: {}", req.id)),
                data: None,
            })?;

        entry.archived = false;
        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to unarchive: {e}")),
            data: None,
        })?;

        // Re-add to search index
        if let Ok(search) = self.open_search_index() {
            let _ = search.index_entry(&entry);
        }

        // Note: Vector embedding will be regenerated by the daemon

        Ok(Self::success(format!("Restored entry: {}", req.id)))
    }

    /// Update an entry
    pub async fn cas_update(
        &self,
        Parameters(req): Parameters<EntryUpdateRequest>,
    ) -> Result<CallToolResult, McpError> {
        let store = self.open_store()?;

        let mut entry = store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Entry not found: {e}")),
            data: None,
        })?;

        let mut changes = Vec::new();

        if let Some(content) = req.content {
            entry.content = content;
            changes.push("content");
        }

        if let Some(tags) = req.tags {
            entry.tags = tags
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            changes.push("tags");
        }

        if let Some(importance) = req.importance {
            entry.importance = importance.clamp(0.0, 1.0);
            changes.push("importance");
        }

        if changes.is_empty() {
            return Ok(Self::success("No changes specified"));
        }

        store.update(&entry).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Updated {}: {}",
            req.id,
            changes.join(", ")
        )))
    }
}
