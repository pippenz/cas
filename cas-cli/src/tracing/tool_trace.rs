use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Rich tool trace for learning loop detection
///
/// Captures detailed information about tool usage to detect patterns like:
/// - Edit -> Fail -> Investigate -> Fix -> Success
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolTrace {
    /// Unique trace ID
    pub id: String,
    /// Session ID
    pub session_id: String,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Tool name (Edit, Write, Bash, Read, Grep, Glob, WebFetch, Task, etc.)
    pub tool_name: String,
    /// Sequence position within session (for ordering)
    pub sequence_pos: u32,
    /// File path (for Edit/Write/Read)
    pub file_path: Option<String>,
    /// Whether file is in deps/dependencies
    pub is_dependency: bool,
    /// Command (for Bash)
    pub command: Option<String>,
    /// Command type classification (build, test, run, other)
    pub command_type: Option<String>,
    /// Exit code (for Bash)
    pub exit_code: Option<i32>,
    /// Whether operation succeeded
    pub success: bool,
    /// Error snippet (first 500 chars of error output)
    pub error_snippet: Option<String>,
    /// Classified error type (type_error, import_error, syntax_error, runtime_error, etc.)
    pub error_type: Option<String>,
    /// Output snippet (first 500 chars for Bash)
    pub output_snippet: Option<String>,
    /// Lines added (for Edit/Write)
    pub lines_added: Option<u32>,
    /// Lines removed (for Edit)
    pub lines_removed: Option<u32>,
    /// Old content for Edit (truncated to 1000 chars, for seeing what changed)
    pub old_content: Option<String>,
    /// New content for Edit/Write (truncated to 1000 chars)
    pub new_content: Option<String>,
    /// Hash of old content (for dedup when content is large)
    pub old_content_hash: Option<String>,
    /// Hash of new content
    pub new_content_hash: Option<String>,
    /// Search pattern (for Grep/Glob)
    pub search_pattern: Option<String>,
    /// Number of search results (for Grep/Glob)
    pub search_results_count: Option<u32>,
    /// URL (for WebFetch)
    pub url: Option<String>,
    /// Previous tool in sequence
    pub prev_tool: Option<String>,
    /// Whether previous tool failed
    pub prev_failed: bool,
    /// Time since last tool (ms)
    pub time_since_prev_ms: Option<u64>,
    /// Attempt ID - groups related operations (e.g., all ops trying to fix same error)
    pub attempt_id: Option<String>,
}

impl ToolTrace {
    /// Create a new tool trace
    pub fn new(session_id: String, tool_name: String, sequence_pos: u32) -> Self {
        Self {
            id: format!(
                "tt-{:x}",
                chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64
            ),
            session_id,
            timestamp: chrono::Utc::now(),
            tool_name,
            sequence_pos,
            file_path: None,
            is_dependency: false,
            command: None,
            command_type: None,
            exit_code: None,
            success: true,
            error_snippet: None,
            error_type: None,
            output_snippet: None,
            lines_added: None,
            lines_removed: None,
            old_content: None,
            new_content: None,
            old_content_hash: None,
            new_content_hash: None,
            search_pattern: None,
            search_results_count: None,
            url: None,
            prev_tool: None,
            prev_failed: false,
            time_since_prev_ms: None,
            attempt_id: None,
        }
    }

    /// Compute a simple hash of content for dedup
    pub fn hash_content(content: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    /// Classify error type from error message
    pub fn classify_error(error: &str) -> &'static str {
        let error_lower = error.to_lowercase();

        // Type errors
        if error_lower.contains("type")
            && (error_lower.contains("mismatch") || error_lower.contains("expected"))
        {
            return "type_error";
        }
        if error_lower.contains("cannot find type") || error_lower.contains("unknown type") {
            return "type_error";
        }

        // Import/module errors
        if error_lower.contains("import") || error_lower.contains("module not found") {
            return "import_error";
        }
        if error_lower.contains("unresolved import") || error_lower.contains("no module named") {
            return "import_error";
        }
        if error_lower.contains("cannot find") && error_lower.contains("in this scope") {
            return "import_error";
        }

        // Syntax errors
        if error_lower.contains("syntax") || error_lower.contains("parse") {
            return "syntax_error";
        }
        if error_lower.contains("unexpected token") || error_lower.contains("expected ") {
            return "syntax_error";
        }

        // Undefined/not found errors
        if error_lower.contains("undefined") || error_lower.contains("not defined") {
            return "undefined_error";
        }
        if error_lower.contains("cannot find function") || error_lower.contains("no function") {
            return "undefined_error";
        }
        if error_lower.contains("method not found") || error_lower.contains("no method named") {
            return "undefined_error";
        }

        // Argument/parameter errors
        if error_lower.contains("argument") || error_lower.contains("parameter") {
            return "argument_error";
        }
        if error_lower.contains("arity") || error_lower.contains("wrong number of") {
            return "argument_error";
        }

        // Borrow/ownership errors (Rust specific)
        if error_lower.contains("borrow") || error_lower.contains("lifetime") {
            return "borrow_error";
        }
        if error_lower.contains("moved") || error_lower.contains("ownership") {
            return "borrow_error";
        }

        // Runtime errors
        if error_lower.contains("runtime") || error_lower.contains("panic") {
            return "runtime_error";
        }
        if error_lower.contains("nil") || error_lower.contains("null") {
            return "runtime_error";
        }

        // Permission/access errors
        if error_lower.contains("permission") || error_lower.contains("access denied") {
            return "permission_error";
        }

        // Network errors
        if error_lower.contains("connection") || error_lower.contains("network") {
            return "network_error";
        }
        if error_lower.contains("timeout") || error_lower.contains("refused") {
            return "network_error";
        }

        "other"
    }

    /// Classify bash command type
    pub fn classify_command(cmd: &str) -> &'static str {
        let cmd_lower = cmd.to_lowercase();
        let first_word = cmd_lower.split_whitespace().next().unwrap_or("");

        // Build commands
        if first_word == "cargo" && cmd_lower.contains("build") {
            return "build";
        }
        if first_word == "cargo" && cmd_lower.contains("check") {
            return "build";
        }
        if first_word == "mix" && cmd_lower.contains("compile") {
            return "build";
        }
        if first_word == "npm" && cmd_lower.contains("build") {
            return "build";
        }
        if first_word == "make" {
            return "build";
        }
        if first_word == "go" && cmd_lower.contains("build") {
            return "build";
        }

        // Test commands
        if first_word == "cargo" && cmd_lower.contains("test") {
            return "test";
        }
        if first_word == "mix" && cmd_lower.contains("test") {
            return "test";
        }
        if first_word == "npm" && cmd_lower.contains("test") {
            return "test";
        }
        if first_word == "pytest" || first_word == "jest" {
            return "test";
        }
        if first_word == "go" && cmd_lower.contains("test") {
            return "test";
        }

        // Run commands
        if first_word == "cargo" && cmd_lower.contains("run") {
            return "run";
        }
        if first_word == "mix" && cmd_lower.contains("phx.server") {
            return "run";
        }
        if first_word == "npm" && cmd_lower.contains("start") {
            return "run";
        }
        if first_word == "node" || first_word == "python" {
            return "run";
        }

        // Git commands
        if first_word == "git" {
            return "git";
        }

        "other"
    }

    /// Check if path looks like a dependency
    pub fn is_dep_path(path: &str) -> bool {
        path.contains("/deps/")
            || path.contains("/node_modules/")
            || path.contains("/vendor/")
            || path.contains("/.cargo/")
            || path.contains("/target/debug/deps/")
            || path.contains("/site-packages/")
    }
}
