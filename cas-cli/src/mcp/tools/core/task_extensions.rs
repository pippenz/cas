use crate::mcp::tools::core::imports::*;

impl CasCore {
    // ========================================================================
    // Additional Task Tools
    // ========================================================================

    /// Delete a task
    pub async fn cas_task_delete(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        // Verify task exists
        task_store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Task not found: {e}")),
            data: None,
        })?;

        task_store.delete(&req.id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to delete: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!("Deleted task: {}", req.id)))
    }

    /// List dependencies for a task (both outgoing and incoming)
    pub async fn cas_task_dep_list(
        &self,
        Parameters(req): Parameters<IdRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        // Get outgoing deps (this task depends on others)
        let outgoing = task_store.get_dependencies(&req.id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to get dependencies: {e}")),
            data: None,
        })?;

        // Get incoming deps (others depend on this task, e.g. children of an epic)
        let incoming = task_store.get_dependents(&req.id).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to get dependents: {e}")),
            data: None,
        })?;

        if outgoing.is_empty() && incoming.is_empty() {
            return Ok(Self::success(format!("No dependencies for {}", req.id)));
        }

        let mut output = format!("Dependencies for {}:\n\n", req.id);

        if !outgoing.is_empty() {
            output.push_str("Outgoing (this task depends on):\n");
            for dep in &outgoing {
                output.push_str(&format!(
                    "  - {:?}: {} -> {}\n",
                    dep.dep_type, dep.from_id, dep.to_id
                ));
            }
        }

        if !incoming.is_empty() {
            output.push_str("Incoming (depend on this task):\n");
            for dep in &incoming {
                // For ParentChild: from_id is the child, to_id is the parent (this task)
                output.push_str(&format!(
                    "  - {:?}: {} -> {}\n",
                    dep.dep_type, dep.from_id, dep.to_id
                ));
            }
        }

        Ok(Self::success(output))
    }
}
