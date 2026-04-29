//! `cas integrate <platform> <action>` — auto-integration scaffolding for
//! Vercel, Neon, and GitHub.
//!
//! This module is the **foundation** for EPIC cas-b65f. It owns:
//!
//! 1. The clap subcommand surface (`cas integrate vercel|neon|github init|refresh|verify`).
//! 2. Shared types in [`types`] returned by every platform handler.
//! 3. The keep-block helper in [`keep_block`] for `<!-- keep -->` … `<!-- /keep -->`
//!    block round-tripping. All three platform handlers reuse this.
//!
//! Platform handlers ([`vercel`], [`neon`], [`github`]) are intentional stubs;
//! the full implementations land in cas-8e37, cas-1ece, and cas-f425. Each
//! stub returns an error pointing at its owning task.
//!
//! ## Template loading convention
//!
//! Platform handlers embed their SKILL.md templates with `include_str!` at
//! compile time:
//!
//! ```ignore
//! const TEMPLATE_INIT: &str = include_str!(
//!     "../../../assets/integrate/vercel_init.md.tmpl"
//! );
//! ```
//!
//! Templates contain `<!-- keep -->` / `<!-- /keep -->` markers around the
//! ID payloads they want preserved across regeneration. See [`keep_block::merge`]
//! for the merge semantics.

pub mod github;
pub mod keep_block;
pub mod neon;
pub mod types;
pub mod vercel;

use clap::Subcommand;

use super::Cli;
use types::{IntegrationAction, IntegrationOutcome};

/// `cas integrate <platform>` — pick a platform.
#[derive(Subcommand, Debug)]
pub enum IntegrateCommands {
    /// Integrate the project with Vercel (project, team, env→branch mappings).
    Vercel {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Integrate the project with Neon (project, branches, org).
    Neon {
        #[command(subcommand)]
        action: PlatformAction,
    },
    /// Integrate the project with GitHub (repo path from `git remote -v`).
    ///
    /// Uses a github-specific action enum so `init` and `refresh` can accept
    /// `--repo OWNER/REPO` to override auto-detection. See
    /// [`github::GithubAction`].
    Github {
        #[command(subcommand)]
        action: github::GithubAction,
    },
}

/// `cas integrate <platform> <action>` — pick an action.
#[derive(Subcommand, Debug, Clone, Copy)]
pub enum PlatformAction {
    /// First-time setup: detect platform, prompt, write SKILL files.
    Init,
    /// Re-run detection; update outer content, preserve user-owned keep blocks.
    Refresh,
    /// Read recorded IDs, ping the platform's MCP, return a staleness report.
    Verify,
}

impl From<PlatformAction> for IntegrationAction {
    fn from(p: PlatformAction) -> Self {
        match p {
            PlatformAction::Init => IntegrationAction::Init,
            PlatformAction::Refresh => IntegrationAction::Refresh,
            PlatformAction::Verify => IntegrationAction::Verify,
        }
    }
}

/// CLI dispatch. Each platform handler is currently a stub that returns an
/// error pointing at its owning task — see module docs.
pub fn execute(cmd: &IntegrateCommands, _cli: &Cli) -> anyhow::Result<()> {
    let outcome = match cmd {
        IntegrateCommands::Vercel { action } => vercel::execute((*action).into())?,
        IntegrateCommands::Neon { action } => neon::execute((*action).into())?,
        IntegrateCommands::Github { action } => github::execute(action.clone())?,
    };
    render_outcome(&outcome);
    Ok(())
}

fn render_outcome(outcome: &IntegrationOutcome) {
    println!(
        "{} {}: {}",
        outcome.platform.as_str(),
        outcome.action.as_str(),
        outcome.status.as_str()
    );
    for line in &outcome.summary {
        println!("  {line}");
    }
    for f in &outcome.files {
        println!("  wrote {}", f.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Minimal clap harness so we can drive the subcommand parser without
    /// pulling in the full `Cli` from `super::Cli`.
    #[derive(Parser, Debug)]
    struct TestCli {
        #[command(subcommand)]
        cmd: IntegrateCommands,
    }

    fn parse(args: &[&str]) -> IntegrateCommands {
        let mut all = vec!["test"];
        all.extend_from_slice(args);
        TestCli::try_parse_from(all).unwrap().cmd
    }

    fn dispatch(cmd: &IntegrateCommands) -> anyhow::Result<IntegrationOutcome> {
        match cmd {
            IntegrateCommands::Vercel { action } => vercel::execute((*action).into()),
            IntegrateCommands::Neon { action } => neon::execute((*action).into()),
            IntegrateCommands::Github { action } => github::execute(action.clone()),
        }
    }

    #[test]
    fn vercel_init_stub_points_at_task_cas_8e37() {
        let cmd = parse(&["vercel", "init"]);
        let err = dispatch(&cmd).unwrap_err().to_string();
        assert!(err.contains("vercel init"), "msg was: {err}");
        assert!(err.contains("cas-8e37"), "msg was: {err}");
        assert!(err.contains("not yet implemented"), "msg was: {err}");
    }

    #[test]
    fn vercel_refresh_stub_points_at_task_cas_8e37() {
        let cmd = parse(&["vercel", "refresh"]);
        let err = dispatch(&cmd).unwrap_err().to_string();
        assert!(err.contains("vercel refresh"));
        assert!(err.contains("cas-8e37"));
    }

    #[test]
    fn vercel_verify_stub_points_at_task_cas_8e37() {
        let cmd = parse(&["vercel", "verify"]);
        let err = dispatch(&cmd).unwrap_err().to_string();
        assert!(err.contains("vercel verify"));
        assert!(err.contains("cas-8e37"));
    }

    #[test]
    fn neon_subcommand_parses_cleanly() {
        // cas-1ece landed the real handler. The stub assertion is gone;
        // the dispatch itself may error in a sandbox (no cwd / no detection /
        // live-client placeholder), but parsing must succeed for all three
        // verbs.
        for action in ["init", "refresh", "verify"] {
            let cmd = parse(&["neon", action]);
            let _ = dispatch(&cmd);
        }
    }

    // GitHub handler is no longer a stub (cas-f425). Its full test coverage
    // lives in `super::github::tests`. We just sanity-check that the clap
    // surface still parses all three actions and that the `--repo` override
    // round-trips into the action enum.
    #[test]
    fn github_clap_surface_parses_all_actions() {
        for action in ["init", "refresh", "verify"] {
            let _ = parse(&["github", action]);
        }
    }

    #[test]
    fn github_init_accepts_repo_override_flag() {
        let cmd = parse(&["github", "init", "--repo", "Richards-LLC/gabber-studio"]);
        match cmd {
            IntegrateCommands::Github {
                action: github::GithubAction::Init { repo },
            } => assert_eq!(repo.as_deref(), Some("Richards-LLC/gabber-studio")),
            other => panic!("unexpected variant: {other:?}"),
        }
    }

    #[test]
    fn github_refresh_accepts_repo_override_flag() {
        let cmd = parse(&["github", "refresh", "--repo", "acme/widget"]);
        match cmd {
            IntegrateCommands::Github {
                action: github::GithubAction::Refresh { repo },
            } => assert_eq!(repo.as_deref(), Some("acme/widget")),
            other => panic!("unexpected variant: {other:?}"),
        }
    }
}
