use crate::mcp::tools::core::imports::*;

impl CasCore {
    pub async fn cas_loop_start(
        &self,
        Parameters(req): Parameters<LoopStartRequest>,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_loop_store;
        use crate::types::Loop;

        let loop_store = open_loop_store(&self.cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open loop store: {e}")),
            data: None,
        })?;

        // Check if there's already an active loop for this session
        if let Ok(Some(existing)) = loop_store.get_active_for_session(&req.session_id) {
            return Err(McpError {
                code: ErrorCode::INVALID_REQUEST,
                message: Cow::from(format!(
                    "Session already has an active loop: {} (iteration {})",
                    existing.id, existing.iteration
                )),
                data: None,
            });
        }

        let id = loop_store.generate_id().map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to generate ID: {e}")),
            data: None,
        })?;

        let cwd = self
            .cas_root
            .parent()
            .unwrap_or(&self.cas_root)
            .to_string_lossy()
            .to_string();

        let loop_state = Loop::with_options(
            id.clone(),
            req.session_id.clone(),
            req.prompt.clone(),
            cwd,
            req.completion_promise.clone(),
            req.max_iterations,
            req.task_id.clone(),
        );

        loop_store.add(&loop_state).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to create loop: {e}")),
            data: None,
        })?;

        let mut output = format!("🔄 Loop {id} started\n\n");
        output.push_str(&format!("Session: {}\n", req.session_id));
        output.push_str("Iteration: 1\n");

        if req.max_iterations > 0 {
            output.push_str(&format!("Max iterations: {}\n", req.max_iterations));
        } else {
            output.push_str("Max iterations: unlimited\n");
        }

        if let Some(ref promise) = req.completion_promise {
            output.push_str(&format!("Completion promise: {promise}\n"));
            output.push_str("\nTo complete this loop, output: <promise>");
            output.push_str(promise);
            output.push_str("</promise>\n");
        }

        if let Some(ref task_id) = req.task_id {
            output.push_str(&format!("Linked task: {task_id}\n"));
        }

        output.push_str(
            "\nThe Stop hook will now iterate. When you try to exit, the prompt will be fed back.",
        );

        Ok(Self::success(output))
    }

    /// Cancel an active loop
    pub async fn cas_loop_cancel(
        &self,
        Parameters(req): Parameters<LoopCancelRequest>,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_loop_store;

        let loop_store = open_loop_store(&self.cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open loop store: {e}")),
            data: None,
        })?;

        let mut active_loop = match loop_store.get_active_for_session(&req.session_id) {
            Ok(Some(l)) => l,
            Ok(None) => {
                return Ok(Self::success("No active loop for this session"));
            }
            Err(e) => {
                return Err(McpError {
                    code: ErrorCode::INTERNAL_ERROR,
                    message: Cow::from(format!("Failed to get loop: {e}")),
                    data: None,
                });
            }
        };

        active_loop.cancel(req.reason.as_deref());
        loop_store.update(&active_loop).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update loop: {e}")),
            data: None,
        })?;

        let output = format!(
            "Loop {} cancelled after {} iterations\n{}",
            active_loop.id,
            active_loop.iteration,
            req.reason
                .map(|r| format!("Reason: {r}"))
                .unwrap_or_default()
        );

        Ok(Self::success(output))
    }

    /// Get loop status
    pub async fn cas_loop_status(
        &self,
        Parameters(req): Parameters<LoopStatusRequest>,
    ) -> Result<CallToolResult, McpError> {
        use crate::store::open_loop_store;

        let loop_store = open_loop_store(&self.cas_root).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to open loop store: {e}")),
            data: None,
        })?;

        match loop_store.get_active_for_session(&req.session_id) {
            Ok(Some(active_loop)) => {
                let mut output = format!("🔄 Loop: {}\n\n", active_loop.id);
                output.push_str(&format!("Status: {}\n", active_loop.status));
                output.push_str(&format!("Iteration: {}", active_loop.iteration));

                if active_loop.max_iterations > 0 {
                    output.push_str(&format!("/{}", active_loop.max_iterations));
                }
                output.push('\n');

                if let Some(ref promise) = active_loop.completion_promise {
                    output.push_str(&format!("Completion promise: {promise}\n"));
                }

                if let Some(ref task_id) = active_loop.task_id {
                    output.push_str(&format!("Linked task: {task_id}\n"));
                }

                output.push_str(&format!(
                    "Started: {}\n",
                    active_loop.started_at.format("%Y-%m-%d %H:%M:%S")
                ));

                Ok(Self::success(output))
            }
            Ok(None) => Ok(Self::success("No active loop for this session")),
            Err(e) => Err(McpError {
                code: ErrorCode::INTERNAL_ERROR,
                message: Cow::from(format!("Failed to get loop: {e}")),
                data: None,
            }),
        }
    }

    // ========================================================================
    // Verification Tools (Task Quality Gates)
    // ========================================================================
}
