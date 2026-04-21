//! `cas worktree ...` — worktree-scoped diagnostics and maintenance commands.
//!
//! Currently exposes `cas worktree sweep`. Intended to grow further as
//! Units 3/5 of EPIC cas-7c88 land follow-up commands (e.g. `restore`).

use anyhow::Result;
use clap::Subcommand;

use crate::cli::sweep::{SweepArgs, execute_sweep};

#[derive(Subcommand, Clone, Debug)]
pub enum WorktreeCommands {
    /// Reclaim factory worker worktree directories that are clean+merged
    /// (and optionally salvage dirty ones via `--salvage-dirty`).
    Sweep(SweepArgs),
}

pub fn execute(cmd: &WorktreeCommands) -> Result<()> {
    match cmd {
        WorktreeCommands::Sweep(args) => execute_sweep(args),
    }
}
