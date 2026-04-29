//! Shared types for `cas integrate <platform> <action>`.
//!
//! Platform handlers in [`super::vercel`], [`super::neon`], and [`super::github`]
//! all return [`IntegrationOutcome`] on success so downstream consumers
//! (`cas init`, `cas doctor`, the verify path) can render or react to the
//! result without each platform inventing its own report shape.
//!
//! Errors flow through `anyhow::Result` — see the design note on task cas-e6b6.

use std::path::PathBuf;

use clap::ValueEnum;

/// Which third-party platform an integration targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Platform {
    Vercel,
    Neon,
    Github,
}

impl Platform {
    pub fn as_str(self) -> &'static str {
        match self {
            Platform::Vercel => "vercel",
            Platform::Neon => "neon",
            Platform::Github => "github",
        }
    }

    /// Task ID owning the platform-specific implementation.
    /// Used by stub handlers to point users at the correct follow-on task.
    pub fn handler_task(self) -> &'static str {
        match self {
            Platform::Vercel => "cas-8e37",
            Platform::Neon => "cas-1ece",
            Platform::Github => "cas-f425",
        }
    }
}

/// Which action a platform handler is performing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationAction {
    /// First-time setup: detect, prompt, write SKILL files.
    Init,
    /// Re-run detection against existing keep-block IDs; update outer content,
    /// preserve user-owned keep blocks (or replace with newly-fetched IDs).
    Refresh,
    /// Read the existing config, ping the platform's MCP, return a structured
    /// staleness report.
    Verify,
}

impl IntegrationAction {
    pub fn as_str(self) -> &'static str {
        match self {
            IntegrationAction::Init => "init",
            IntegrationAction::Refresh => "refresh",
            IntegrationAction::Verify => "verify",
        }
    }
}

/// Status flag for an [`IntegrationOutcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrationStatus {
    /// Init wrote new SKILL files.
    Configured,
    /// Refresh updated outer content; keep blocks preserved.
    Refreshed,
    /// Init found existing populated SKILL files; no changes written.
    AlreadyConfigured,
    /// Verify found drift between recorded IDs and platform state.
    Stale,
    /// User declined the prompt or platform not detected.
    Skipped,
}

/// Structured result of an integration action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationOutcome {
    pub platform: Platform,
    pub action: IntegrationAction,
    pub status: IntegrationStatus,
    /// Human-readable lines for terminal rendering.
    pub summary: Vec<String>,
    /// Files written or modified during this action (repo-relative).
    pub files: Vec<PathBuf>,
}

impl IntegrationOutcome {
    pub fn new(platform: Platform, action: IntegrationAction, status: IntegrationStatus) -> Self {
        Self {
            platform,
            action,
            status,
            summary: Vec::new(),
            files: Vec::new(),
        }
    }
}
