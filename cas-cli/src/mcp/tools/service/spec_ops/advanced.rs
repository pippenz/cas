use crate::mcp::tools::service::imports::*;

impl CasService {
    pub(crate) async fn spec_supersede(
        &self,
        req: SpecRequest,
    ) -> Result<CallToolResult, McpError> {
        use cas_store::{SpecStore, SqliteSpecStore};
        use cas_types::SpecStatus;
        use chrono::Utc;

        let id = req
            .id
            .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "id required (new spec)"))?;

        let supersedes_id = req.supersedes_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "supersedes_id required (spec being superseded)",
            )
        })?;

        let store = SqliteSpecStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spec store: {e}"),
            )
        })?;
        store.init().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to initialize spec store: {e}"),
            )
        })?;

        // Get the new spec
        let mut new_spec = store.get(&id).map_err(|e| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                format!("New spec not found: {e}"),
            )
        })?;

        // Get the old spec
        let mut old_spec = store.get(&supersedes_id).map_err(|e| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                format!("Spec to supersede not found: {e}"),
            )
        })?;

        // Update new spec to reference old one
        new_spec.previous_version_id = Some(supersedes_id.clone());
        new_spec.version = old_spec.version + 1;
        new_spec.updated_at = Utc::now();

        // Mark old spec as superseded
        old_spec.status = SpecStatus::Superseded;
        old_spec.updated_at = Utc::now();

        store.update(&new_spec).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to update new spec: {e}"),
            )
        })?;
        store.update(&old_spec).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to mark old spec as superseded: {e}"),
            )
        })?;

        Ok(Self::success(format!(
            "Spec {} (v{}) now supersedes {} (v{})\n\n{} marked as superseded.",
            id, new_spec.version, supersedes_id, old_spec.version, supersedes_id
        )))
    }

    pub(crate) async fn spec_link(&self, req: SpecRequest) -> Result<CallToolResult, McpError> {
        use cas_store::{SpecStore, SqliteSpecStore};
        use chrono::Utc;

        let id = req
            .id
            .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "id required"))?;

        let task_id = req
            .task_id
            .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "task_id required"))?;

        let store = SqliteSpecStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spec store: {e}"),
            )
        })?;
        store.init().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to initialize spec store: {e}"),
            )
        })?;

        let mut spec = store
            .get(&id)
            .map_err(|e| Self::error(ErrorCode::INVALID_PARAMS, format!("Spec not found: {e}")))?;

        spec.task_id = Some(task_id.clone());
        spec.updated_at = Utc::now();

        store.update(&spec).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to link spec: {e}"),
            )
        })?;

        Ok(Self::success(format!("Linked spec {id} to task {task_id}")))
    }

    pub(crate) async fn spec_unlink(&self, req: SpecRequest) -> Result<CallToolResult, McpError> {
        use cas_store::{SpecStore, SqliteSpecStore};
        use chrono::Utc;

        let id = req
            .id
            .ok_or_else(|| Self::error(ErrorCode::INVALID_PARAMS, "id required"))?;

        let store = SqliteSpecStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spec store: {e}"),
            )
        })?;
        store.init().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to initialize spec store: {e}"),
            )
        })?;

        let mut spec = store
            .get(&id)
            .map_err(|e| Self::error(ErrorCode::INVALID_PARAMS, format!("Spec not found: {e}")))?;

        let old_task_id = spec.task_id.take();
        spec.updated_at = Utc::now();

        store.update(&spec).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to unlink spec: {e}"),
            )
        })?;

        match old_task_id {
            Some(task_id) => Ok(Self::success(format!(
                "Unlinked spec {id} from task {task_id}"
            ))),
            None => Ok(Self::success(format!(
                "Spec {id} was not linked to any task"
            ))),
        }
    }

    pub(crate) async fn spec_sync(&self, _req: SpecRequest) -> Result<CallToolResult, McpError> {
        use cas_core::SpecSyncer;
        use cas_store::{SpecStore, SqliteSpecStore};

        let store = SqliteSpecStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spec store: {e}"),
            )
        })?;
        store.init().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to initialize spec store: {e}"),
            )
        })?;

        // Get all specs (syncer handles filtering by approved status)
        let specs = store.list(None).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to list specs: {e}"),
            )
        })?;

        // Use SpecSyncer for consistent sync logic and correct path (.cas/specs/)
        let syncer = SpecSyncer::with_defaults(&self.inner.cas_root);
        let report = syncer.sync_all(&specs).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to sync specs: {e}"),
            )
        })?;

        let mut output = format!(
            "Synced {} approved spec(s) to {}/",
            report.synced,
            syncer.target_dir().display()
        );

        if report.removed > 0 {
            output.push_str(&format!("\nRemoved {} stale spec(s)", report.removed));
        }

        Ok(Self::success(output))
    }

    pub(crate) async fn spec_get_for_task(
        &self,
        req: SpecRequest,
    ) -> Result<CallToolResult, McpError> {
        use cas_store::{SpecStore, SqliteSpecStore};

        let task_id = req.task_id.ok_or_else(|| {
            Self::error(
                ErrorCode::INVALID_PARAMS,
                "task_id required for get_for_task",
            )
        })?;

        let store = SqliteSpecStore::open(&self.inner.cas_root).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to open spec store: {e}"),
            )
        })?;
        store.init().map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to initialize spec store: {e}"),
            )
        })?;

        let specs = store.get_for_task(&task_id).map_err(|e| {
            Self::error(
                ErrorCode::INTERNAL_ERROR,
                format!("Failed to get specs for task: {e}"),
            )
        })?;

        if specs.is_empty() {
            return Ok(Self::success(format!(
                "No specs found linked to task {task_id}"
            )));
        }

        // Sort specs: approved first, then by version descending
        let mut sorted_specs = specs;
        sorted_specs.sort_by(|a, b| {
            use cas_types::SpecStatus;
            // Approved specs first
            let a_approved = matches!(a.status, SpecStatus::Approved);
            let b_approved = matches!(b.status, SpecStatus::Approved);
            match (a_approved, b_approved) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => b.version.cmp(&a.version), // Higher version first
            }
        });

        let mut output = format!(
            "Specs for task {} ({} found):\n\n",
            task_id,
            sorted_specs.len()
        );
        for spec in &sorted_specs {
            output.push_str(&format!(
                "- {} [{}] v{} - {}\n",
                spec.id, spec.status, spec.version, spec.title
            ));
            if !spec.summary.is_empty() {
                output.push_str(&format!("  Summary: {}\n", spec.summary));
            }
        }

        // Include full details of the first (recommended) spec
        if let Some(spec) = sorted_specs.first() {
            output.push_str(&format!(
                "\n## Recommended Spec: {} (v{})\n\n",
                spec.id, spec.version
            ));
            output.push_str(&format!("**Title**: {}\n", spec.title));
            output.push_str(&format!("**Type**: {}\n", spec.spec_type));
            output.push_str(&format!("**Status**: {}\n\n", spec.status));

            if !spec.summary.is_empty() {
                output.push_str(&format!("### Summary\n{}\n\n", spec.summary));
            }
            if !spec.goals.is_empty() {
                output.push_str("### Goals\n");
                for goal in &spec.goals {
                    output.push_str(&format!("- {goal}\n"));
                }
                output.push('\n');
            }
            if !spec.in_scope.is_empty() {
                output.push_str("### In Scope\n");
                for item in &spec.in_scope {
                    output.push_str(&format!("- {item}\n"));
                }
                output.push('\n');
            }
            if !spec.out_of_scope.is_empty() {
                output.push_str("### Out of Scope\n");
                for item in &spec.out_of_scope {
                    output.push_str(&format!("- {item}\n"));
                }
                output.push('\n');
            }
            if !spec.acceptance_criteria.is_empty() {
                output.push_str("### Acceptance Criteria\n");
                for criterion in &spec.acceptance_criteria {
                    output.push_str(&format!("- {criterion}\n"));
                }
                output.push('\n');
            }
            if !spec.technical_requirements.is_empty() {
                output.push_str("### Technical Requirements\n");
                for req in &spec.technical_requirements {
                    output.push_str(&format!("- {req}\n"));
                }
                output.push('\n');
            }
            if !spec.design_notes.is_empty() {
                output.push_str(&format!("### Design Notes\n{}\n", spec.design_notes));
            }
        }

        Ok(Self::success(output))
    }

    /// Parse comma-separated string or JSON array into Vec<String>.
    /// Handles both `"Goal 1, Goal 2"` and `'["Goal 1", "Goal 2"]'` formats.
    pub(super) fn parse_comma_list(opt: &Option<String>) -> Vec<String> {
        opt.as_ref()
            .map(|s| {
                let trimmed = s.trim();
                // Try JSON array first (agents sometimes send JSON-encoded arrays)
                if trimmed.starts_with('[') {
                    if let Ok(arr) = serde_json::from_str::<Vec<String>>(trimmed) {
                        return arr
                            .into_iter()
                            .map(|item| item.trim().to_string())
                            .filter(|item| !item.is_empty())
                            .collect();
                    }
                }
                // Fall back to comma-separated
                trimmed
                    .split(',')
                    .map(|item| item.trim().to_string())
                    .filter(|item| !item.is_empty())
                    .collect()
            })
            .unwrap_or_default()
    }
}
