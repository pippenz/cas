//! CAS - Coding Agent System
//!
//! A library for AI agents to build persistent memory across sessions.
//!
//! This crate provides unified task tracking, memory management, rules, and skills
//! for AI coding agents.

// The MCP panic catcher in `mcp::tools::service::panic_catch` relies on
// `tokio::spawn` + `JoinError::is_panic` to convert handler panics into
// `INTERNAL_ERROR` responses. Under `panic = "abort"` the process is
// terminated before the `JoinHandle` is reachable, and A2 (EPIC cas-c351
// / cas-a436) provides no protection — a single handler panic aborts
// `cas serve`. Rust overrides panic=abort to unwind for `cargo test`
// automatically, so this guard only fires on non-test builds.
#[cfg(all(not(test), panic = "abort"))]
compile_error!(
    "cas requires `panic = \"unwind\"` (see EPIC cas-c351). \
     The MCP dispatch panic catcher in cas::mcp::tools::service::panic_catch \
     depends on stack unwinding; `panic = \"abort\"` disables it and makes \
     `cas serve` crash on the first handler panic with no server-side trace. \
     Remove `panic = \"abort\"` from the build profile."
);

// Core modules
pub mod agent_id;
pub mod async_runtime;
pub mod bridge;
pub mod builtins;
pub mod cli;
pub mod cloud;
pub mod config;
pub mod consolidation;
pub mod daemon;
pub mod duplicate_check;
pub mod error;
pub mod extraction;
pub mod harness_policy;
pub mod hooks;
pub mod hybrid_search;
pub mod logging;
pub mod migration;
pub mod notifications;
pub mod orchestration;
pub mod otel;
pub mod rules;
pub mod sentry;
pub mod store;
pub mod sync;
pub mod telemetry;
pub mod tracing;
pub mod ui;
pub mod worktree;

// Re-export cas-types as types for backward compatibility
pub use cas_types as types;

// MCP server (behind feature flag)
#[cfg(feature = "mcp-server")]
pub mod mcp;

// Re-exports for convenience
pub use error::{CasError, Result};
pub use types::{
    Agent, ChangeType, CommitLink, Entry, EntryType, Event, FileChange, Prompt, Rule, RuleStatus,
    Session, SessionOutcome, Skill, Spec, Task, TaskStatus, Verification, Worktree, WorktreeStatus,
};
