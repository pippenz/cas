use crate::mcp::tools::core::imports::*;

impl CasCore {
    pub async fn cas_task_notes(
        &self,
        Parameters(req): Parameters<TaskNotesRequest>,
    ) -> Result<CallToolResult, McpError> {
        let task_store = self.open_task_store()?;

        let mut task = task_store.get(&req.id).map_err(|e| McpError {
            code: ErrorCode::INVALID_PARAMS,
            message: Cow::from(format!("Task not found: {e}")),
            data: None,
        })?;

        // Format the note with type prefix and timestamp
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M");
        let type_prefix = match req.note_type.to_lowercase().as_str() {
            "blocker" => "🚫 BLOCKER",
            "decision" => "✅ DECISION",
            "discovery" => "💡 DISCOVERY",
            "question" => "❓ QUESTION",
            _ => "📝 PROGRESS",
        };

        let formatted_note = format!("[{}] {} {}", timestamp, type_prefix, req.note);

        // Append to existing notes
        if task.notes.is_empty() {
            task.notes = formatted_note;
        } else {
            task.notes = format!("{}\n\n{}", task.notes, formatted_note);
        }

        task.updated_at = chrono::Utc::now();

        task_store.update(&task).map_err(|e| McpError {
            code: ErrorCode::INTERNAL_ERROR,
            message: Cow::from(format!("Failed to update task: {e}")),
            data: None,
        })?;

        Ok(Self::success(format!(
            "Added {} note to task {}",
            req.note_type, req.id
        )))
    }
}
