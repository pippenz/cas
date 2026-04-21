//! `cas known-repos {list,seed}` — inspect and bootstrap the host repo registry.
//!
//! The registry itself lives in `~/.cas/cas.db::known_repos` and is upserted
//! automatically by `cas init`, factory daemon startup, and MCP server
//! startup. The commands here exist for diagnostics and for one-time seeding
//! on hosts that pre-date the auto-upsert hooks.

use anyhow::Result;
use clap::Subcommand;

use crate::worktree::discovery::{list_tracked_repos, seed};

#[derive(Subcommand, Clone, Debug)]
pub enum KnownReposCommands {
    /// Print every repo in the host-scoped known_repos registry.
    List,
    /// Seed the registry from existing host state (sessions.cwd + session
    /// JSON files). Idempotent.
    Seed {
        /// Additionally scan $HOME up to depth 5 for `.cas/` directories.
        /// Slow on large home directories; opt in explicitly.
        #[arg(long)]
        scan_home: bool,
    },
}

pub fn execute(cmd: &KnownReposCommands) -> Result<()> {
    match cmd {
        KnownReposCommands::List => execute_list(),
        KnownReposCommands::Seed { scan_home } => execute_seed(*scan_home),
    }
}

fn execute_list() -> Result<()> {
    let repos = list_tracked_repos()?;
    if repos.is_empty() {
        println!("No known repos yet. Run `cas init` in a project, or `cas known-repos seed` to bootstrap from existing sessions.");
        return Ok(());
    }
    println!("{} known repo(s):", repos.len());
    for r in repos {
        let flag = if r.healthy { "ok    " } else { "MISSING" };
        println!("  [{flag}] touch_count={:<4} {}", r.touch_count, r.path.display());
    }
    Ok(())
}

fn execute_seed(scan_home: bool) -> Result<()> {
    eprintln!(
        "Seeding known_repos from sessions.cwd + ~/.cas/sessions/*.json{}...",
        if scan_home {
            " + $HOME walk (slow)"
        } else {
            ""
        }
    );
    let report = seed(scan_home)?;
    println!(
        "Seed complete: {} new, {} already-present, {} skipped (no .cas/)",
        report.new.len(),
        report.existing.len(),
        report.skipped_missing.len(),
    );
    if !report.new.is_empty() {
        println!("Newly registered:");
        for p in &report.new {
            println!("  + {}", p.display());
        }
    }
    if !report.skipped_missing.is_empty() {
        println!("Skipped (path has no .cas/ subdirectory):");
        for p in &report.skipped_missing {
            println!("  - {}", p.display());
        }
    }
    Ok(())
}
