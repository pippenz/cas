//! Claude Code hook handling
//!
//! Processes hook events from Claude Code via stdin/stdout JSON protocol.
//!
//! # Architecture
//!
//! This module re-exports core types from `cas-core::hooks` and provides
//! CLI-specific wrappers that handle store opening and configuration loading.
//!
//! # Supported Hooks
//!
//! - **SessionStart**: Injects relevant context at session start
//! - **SessionEnd**: Marks observations for extraction
//! - **Stop**: Generates session summary when agent finishes
//! - **SubagentStop**: Cleans up subagent leases when subagent finishes
//! - **PostToolUse**: Captures interesting tool interactions as observations
//! - **UserPromptSubmit**: Optional prompt capture (currently passthrough)
//!
//! # Usage
//!
//! ```bash
//! # Handle a hook event (reads JSON from stdin)
//! cas hook SessionStart
//! ```

mod context;
pub(crate) mod handlers;
pub mod scorer;
pub mod transcript;
mod types;

// Re-export types from cas-core
pub use cas_core::hooks::{
    // Context scoring
    BasicContextScorer,
    ContextItem,
    ContextItemType,
    ContextQuery,
    ContextScorer,
    ContextStats,
    ContextStores,
    // Config trait
    DefaultHooksConfig,
    // Types
    HookInput,
    HookOutput,
    HookSpecificOutput,
    HooksConfig,
    PlanModeConfig,
    // Caching
    RuleMatchCache,
    SurfacedItemCallback,
    // Context building (with stores)
    build_context_with_stores,
    build_plan_context_with_stores,
    // Utilities
    estimate_tokens,
    rule_matches_path,
    token_display,
    truncate,
};

// Re-export CLI scorers
pub use scorer::HybridContextScorer;

// Re-export transcript functions from cas-core
pub use cas_core::hooks::transcript::{
    ContentBlock, TranscriptEntry, TranscriptMessage, check_promise_in_transcript,
    get_last_assistant_text, get_recent_assistant_messages,
};

// Re-export CLI-specific wrappers
pub use context::{build_context, build_context_ai, build_plan_context};

// Re-export handlers
pub use handlers::{
    get_session_files, handle_notification, handle_permission_request, handle_post_tool_use,
    handle_pre_compact, handle_pre_tool_use, handle_session_end, handle_session_start, handle_stop,
    handle_subagent_start, handle_subagent_stop, handle_user_prompt_submit,
};

use std::path::PathBuf;

use crate::error::MemError;
use crate::store::find_cas_root;

/// Route a hook event to its handler
///
/// This is the single entry point that resolves cas_root once and passes it
/// to all handlers, eliminating redundant find_cas_root() calls.
pub fn handle_hook(event_name: &str, input: HookInput) -> Result<HookOutput, MemError> {
    // Resolve cas_root once at entry point using full discovery logic:
    // 1. CAS_ROOT env var (factory workers use this to share main repo's .cas)
    // 2. Git worktree detection (worktrees share main repo's .cas)
    // 3. Walk up directory tree from cwd
    //
    // IMPORTANT: We use find_cas_root() not find_cas_root_from() to preserve
    // CAS_ROOT env var priority for factory mode compatibility.
    let cas_root: Option<PathBuf> = find_cas_root().ok();

    match event_name {
        "SessionStart" => handle_session_start(&input, cas_root.as_deref()),
        "SessionEnd" => handle_session_end(&input, cas_root.as_deref()),
        "Stop" => handle_stop(&input, cas_root.as_deref()),
        "SubagentStart" => handle_subagent_start(&input, cas_root.as_deref()),
        "SubagentStop" => handle_subagent_stop(&input, cas_root.as_deref()),
        "PostToolUse" => handle_post_tool_use(&input, cas_root.as_deref()),
        "PreToolUse" => handle_pre_tool_use(&input, cas_root.as_deref()),
        "UserPromptSubmit" => handle_user_prompt_submit(&input, cas_root.as_deref()),
        "PermissionRequest" => handle_permission_request(&input, cas_root.as_deref()),
        "Notification" => handle_notification(&input, cas_root.as_deref()),
        "PreCompact" => handle_pre_compact(&input, cas_root.as_deref()),
        _ => {
            // Unknown hook, just pass through
            Ok(HookOutput::empty())
        }
    }
}
