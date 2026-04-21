//! CAS - Coding Agent System
//!
//! A library for AI agents to build persistent memory across sessions.
//!
//! This crate provides unified task tracking, memory management, rules, and skills
//! for AI coding agents.

// Build-time enforcement of the panic=unwind invariant the MCP tool
// dispatch panic catcher depends on. The `not(test)` exemption exists
// because Rust forces panic=unwind when compiling the lib under
// `cargo test --lib`; the guard still fires for `cargo build`,
// `cargo check`, and integration-test dependency compilations (where
// cfg(test) is false on the lib).
#[cfg(all(not(test), panic = "abort"))]
compile_error!(
    "cas requires `panic = \"unwind\"` (see EPIC cas-c351). The MCP dispatch \
     panic catcher at cas-cli/src/mcp/tools/service/panic_catch.rs depends \
     on stack unwinding; `panic = \"abort\"` disables it and makes `cas serve` \
     crash on the first handler panic with no server-side trace. Remove \
     `panic = \"abort\"` from the build profile."
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

/// Shared test-only utilities. Kept in one place so cross-module statics
/// (like the HOME env-var mutex used by known_repos + discovery tests)
/// refer to a single instance; otherwise each test module's own static
/// would race against the other's.
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;
    /// Serializes tests that mutate `HOME`. `std::env::set_var` is
    /// process-global; parallel tests would otherwise race.
    pub static HOME_MUTEX: Mutex<()> = Mutex::new(());
}

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
