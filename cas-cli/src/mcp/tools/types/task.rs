use rmcp::schemars::JsonSchema;
use serde::Deserialize;

use crate::mcp::tools::types::defaults::{
    default_dep_type, default_note_type, default_priority, default_subagent_tokens,
    default_task_type, default_true,
};

/// Allowed values for `execution_note` (cas-7fc1). Validated at the MCP
/// tool layer rather than via a SQL CHECK constraint so new values can be
/// added without a schema migration.
pub const EXECUTION_NOTE_VALUES: &[&str] =
    &["test-first", "characterization-first", "additive-only"];

/// Validate and normalize an incoming `execution_note` parameter. Returns
/// - `Ok(None)` if the input is absent or an empty string (clear/omitted)
/// - `Ok(Some(v))` for an accepted enum value
/// - `Err(message)` otherwise, with a human-readable hint listing the
///   allowed values
pub fn validate_execution_note(value: Option<&str>) -> Result<Option<String>, String> {
    match value {
        None => Ok(None),
        Some(s) if s.is_empty() => Ok(None),
        Some(s) if EXECUTION_NOTE_VALUES.contains(&s) => Ok(Some(s.to_string())),
        Some(s) => Err(format!(
            "Invalid execution_note: '{}'. Must be one of: {} (or empty/absent to clear)",
            s,
            EXECUTION_NOTE_VALUES.join(", ")
        )),
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCreateRequest {
    /// Task title
    #[schemars(description = "Short descriptive title for the task")]
    pub title: String,

    /// Task description
    #[schemars(description = "Detailed description of what needs to be done")]
    #[serde(default)]
    pub description: Option<String>,

    /// Priority (0-4)
    #[schemars(description = "Priority: 0=Critical, 1=High, 2=Medium (default), 3=Low, 4=Backlog")]
    #[serde(default = "default_priority")]
    pub priority: u8,

    /// Task type
    #[schemars(description = "Type: 'task' (default), 'bug', 'feature', 'epic', 'chore', 'spike'")]
    #[serde(default = "default_task_type")]
    pub task_type: String,

    /// Labels
    #[schemars(description = "Comma-separated labels for categorization")]
    #[serde(default)]
    pub labels: Option<String>,

    /// Working notes
    #[schemars(description = "Working notes (use for ongoing updates)")]
    #[serde(default)]
    pub notes: Option<String>,

    /// Blocked by task IDs
    #[schemars(description = "Comma-separated task IDs that block this task")]
    #[serde(default)]
    pub blocked_by: Option<String>,

    /// Design notes
    #[schemars(description = "Design notes or technical approach")]
    #[serde(default)]
    pub design: Option<String>,

    /// Acceptance criteria
    #[schemars(description = "Acceptance criteria for task completion")]
    #[serde(default)]
    pub acceptance_criteria: Option<String>,

    /// External reference
    #[schemars(description = "External reference (URL, ticket ID, etc.)")]
    #[serde(default)]
    pub external_ref: Option<String>,

    /// Assignee
    #[schemars(description = "Assignee identifier (agent ID or name)")]
    #[serde(default)]
    pub assignee: Option<String>,

    /// Demo statement
    #[schemars(
        description = "What can be demonstrated when this task is complete (e.g., 'Type a query, results filter live')"
    )]
    #[serde(default)]
    pub demo_statement: Option<String>,

    /// Execution methodology note
    #[schemars(
        description = "Execution methodology for this task. One of: test-first, characterization-first, additive-only. Omit or leave null if no methodology is declared."
    )]
    #[serde(default)]
    pub execution_note: Option<String>,

    /// Epic ID to associate with
    #[schemars(
        description = "Epic task ID to associate this task with (creates ParentChild dependency)"
    )]
    #[serde(default)]
    pub epic: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskCloseRequest {
    /// Task ID
    #[schemars(description = "Task ID to close")]
    pub id: String,

    /// Close reason
    #[schemars(description = "Reason for closing (resolution notes)")]
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskUpdateRequest {
    /// Task ID to update
    #[schemars(description = "ID of the task to update")]
    pub id: String,

    /// Update title
    #[schemars(description = "New title for the task")]
    #[serde(default)]
    pub title: Option<String>,

    /// Add to notes
    #[schemars(description = "Notes to append (working notes, progress updates)")]
    #[serde(default)]
    pub notes: Option<String>,

    /// Update priority
    #[schemars(description = "New priority (0-4)")]
    #[serde(default)]
    pub priority: Option<u8>,

    /// Add labels
    #[schemars(description = "Comma-separated labels to add")]
    #[serde(default)]
    pub labels: Option<String>,

    /// Update description
    #[schemars(description = "New description")]
    #[serde(default)]
    pub description: Option<String>,

    /// Update design notes
    #[schemars(description = "New design notes")]
    #[serde(default)]
    pub design: Option<String>,

    /// Update acceptance criteria
    #[schemars(description = "New acceptance criteria")]
    #[serde(default)]
    pub acceptance_criteria: Option<String>,

    /// Update demo statement
    #[schemars(
        description = "What can be demonstrated when this task is complete (e.g., 'Type a query, results filter live')"
    )]
    #[serde(default)]
    pub demo_statement: Option<String>,

    /// Update execution note
    #[schemars(
        description = "Execution methodology for this task. One of: test-first, characterization-first, additive-only. Pass an empty string to clear."
    )]
    #[serde(default)]
    pub execution_note: Option<String>,

    /// Update external reference
    #[schemars(description = "New external reference")]
    #[serde(default)]
    pub external_ref: Option<String>,

    /// Update assignee
    #[schemars(description = "New assignee")]
    #[serde(default)]
    pub assignee: Option<String>,

    /// Update status
    #[schemars(description = "New status: 'open', 'in_progress', 'closed', 'blocked'")]
    #[serde(default)]
    pub status: Option<String>,

    /// Set or change epic association
    #[schemars(
        description = "Epic task ID to associate this task with (updates ParentChild dependency)"
    )]
    #[serde(default)]
    pub epic: Option<String>,

    /// Set epic verification owner (for epics in factory mode)
    #[schemars(
        description = "Agent ID responsible for epic verification (supervisor in factory mode)"
    )]
    #[serde(default)]
    pub epic_verification_owner: Option<String>,
}

// ============================================================================
// Task Show Request
// ============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskShowRequest {
    /// Task ID
    #[schemars(description = "ID of the task to show")]
    pub id: String,

    /// Include dependencies
    #[schemars(description = "Include dependency information (default: true)")]
    #[serde(default = "default_true")]
    pub with_deps: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DependencyRequest {
    /// From task ID
    #[schemars(description = "Task that has the dependency")]
    pub from_id: String,

    /// To task ID
    #[schemars(description = "Task that blocks/relates to the first")]
    pub to_id: String,

    /// Dependency type
    #[schemars(description = "Type: 'blocks' (default), 'related', 'parent', 'duplicate'")]
    #[serde(default = "default_dep_type")]
    pub dep_type: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TaskNotesRequest {
    /// Task ID
    #[schemars(description = "ID of the task to add notes to")]
    pub id: String,

    /// Note content
    #[schemars(description = "The note content to append")]
    pub note: String,

    /// Note type for structured categorization
    #[schemars(
        description = "Type: 'progress' (default), 'blocker', 'decision', 'discovery', 'question'"
    )]
    #[serde(default = "default_note_type")]
    pub note_type: String,
}

// ============================================================================
// Sub-Agent Context Request
// ============================================================================

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubAgentContextRequest {
    /// Task ID to get context for
    #[schemars(description = "Task ID to build focused context for")]
    pub task_id: String,

    /// Maximum tokens for the context
    #[schemars(description = "Maximum tokens for sub-agent context (default: 2000)")]
    #[serde(default = "default_subagent_tokens")]
    pub max_tokens: usize,

    /// Include related memories
    #[schemars(description = "Search and include related memories (default: true)")]
    #[serde(default = "default_true")]
    pub include_memories: bool,
}
