//! Factory session management
//!
//! Spawns a native terminal multiplexer TUI with:
//! - Director (native panel) - monitors events, displays tasks/agents/activity
//! - Supervisor - plans epics, assigns tasks, handles merges
//! - Workers - execute tasks (read-only, inject only)

mod cloud_attach;
mod daemon;
mod lifecycle;
mod queries;
mod remote_attach;
mod worktree_ops;

use crate::cli::Cli;
use crate::config::Config;
use crate::orchestration::names::generate_unique;
use crate::ui::factory::{
    FactoryConfig, NotifyBackend, NotifyConfig, attach, find_session_for_project,
    generate_session_name,
};
use crate::worktree::GitOperations;
use anyhow::{Result, bail};
use clap::{Args, Subcommand};
use std::io::IsTerminal;

pub use lifecycle::{execute_kill, execute_kill_all};
pub use queries::execute_list;

/// Launch hierarchical multi-agent factory session
#[derive(Args, Debug, Clone)]
pub struct FactoryArgs {
    /// Factory subcommand
    #[command(subcommand)]
    pub command: Option<FactoryCommands>,

    /// Number of worker agents (default: 0 for supervisor-only startup)
    #[arg(long, short = 'w', default_value = "0", global = true)]
    pub workers: u8,

    /// Custom session name
    #[arg(long, short = 'n', global = true)]
    pub name: Option<String>,

    /// Start a new session instead of auto-attaching an existing one
    #[arg(long = "new", global = true)]
    pub start_new: bool,

    /// Attach to an existing session without prompting (skip confirmation)
    #[arg(long, global = true)]
    pub attach: bool,

    /// Disable worktree-based worker isolation (all agents share the same directory)
    #[arg(long, global = true)]
    pub no_worktrees: bool,

    /// Custom directory for worker worktrees (default: .cas/worktrees under project)
    #[arg(long, global = true)]
    pub worktree_root: Option<std::path::PathBuf>,

    /// Remove all worker worktree directories
    #[arg(long, conflicts_with = "workers")]
    pub cleanup: bool,

    /// Show what would be removed without actually deleting (use with --cleanup)
    #[arg(long, requires = "cleanup")]
    pub dry_run: bool,

    /// Force cleanup without confirmation (use with --cleanup)
    #[arg(long, short = 'f', requires = "cleanup")]
    pub force: bool,

    /// Run daemon in foreground (no fork) instead of attaching
    #[arg(long, hide = true)]
    pub legacy: bool,

    /// Enable desktop/terminal notifications for task events
    #[arg(long)]
    pub notify: bool,

    /// Also ring terminal bell on notifications (use with --notify)
    #[arg(long, requires = "notify")]
    pub bell: bool,

    /// Use tabbed worker view instead of side-by-side (default: side-by-side)
    #[arg(long, global = true)]
    pub tabbed: bool,

    /// Record terminal sessions for time-travel playback (requires factory-recording feature)
    #[arg(long, global = true)]
    pub record: bool,

    /// Supervisor CLI to use (claude, codex, or pi)
    #[arg(long, default_value = "claude")]
    pub supervisor_cli: String,

    /// Worker CLI to use (claude, codex, or pi)
    #[arg(long, default_value = "claude")]
    pub worker_cli: String,

    /// Disable cloud phone-home (push factory state to CAS Cloud)
    #[arg(long, global = true)]
    pub no_phone_home: bool,
}

impl Default for FactoryArgs {
    fn default() -> Self {
        Self {
            command: None,
            workers: 0,
            name: None,
            start_new: false,
            attach: false,
            no_worktrees: false,
            worktree_root: None,
            cleanup: false,
            dry_run: false,
            force: false,
            legacy: false,
            notify: false,
            bell: false,
            tabbed: false,
            record: false,
            supervisor_cli: "claude".to_string(),
            worker_cli: "claude".to_string(),
            no_phone_home: false,
        }
    }
}

/// Arguments for `cas attach`
#[derive(Args, Debug, Clone)]
pub struct AttachArgs {
    /// Session name to attach to (default: most recent)
    pub name: Option<String>,

    /// Remote target: `device:factory-id` (SSH) or `factory-id` (cloud relay)
    #[arg(long)]
    pub remote: Option<String>,

    /// Specific worker to focus on (used with --remote)
    #[arg(long)]
    pub worker: Option<String>,
}

/// Arguments for `cas kill`
#[derive(Args, Debug, Clone)]
pub struct KillArgs {
    /// Session name to kill (interactive picker if omitted)
    pub name: Option<String>,

    /// Force kill without confirmation
    #[arg(long, short = 'f')]
    pub force: bool,
}

/// Arguments for `cas kill-all`
#[derive(Args, Debug, Clone)]
pub struct KillAllArgs {
    /// Force kill without confirmation
    #[arg(long, short = 'f')]
    pub force: bool,
}

/// Internal factory subcommands (hidden from help)
#[derive(Subcommand, Debug, Clone)]
pub enum FactoryCommands {
    /// Run as a factory daemon (internal use)
    #[command(hide = true)]
    Daemon {
        /// Session name
        #[arg(long)]
        session: String,

        /// Working directory
        #[arg(long)]
        cwd: std::path::PathBuf,

        /// Number of workers
        #[arg(long, default_value = "0")]
        workers: u8,

        /// Disable worktree-based worker isolation
        #[arg(long)]
        no_worktrees: bool,

        /// Custom directory for worker worktrees
        #[arg(long)]
        worktree_root: Option<std::path::PathBuf>,

        /// Enable notifications
        #[arg(long)]
        notify: bool,

        /// Supervisor CLI to use (claude, codex, or pi)
        #[arg(long, default_value = "claude")]
        supervisor_cli: String,

        /// Worker CLI to use (claude, codex, or pi)
        #[arg(long, default_value = "claude")]
        worker_cli: String,

        /// Use tabbed worker view
        #[arg(long)]
        tabbed: bool,

        /// Record terminal sessions
        #[arg(long)]
        record: bool,

        /// Run in foreground
        #[arg(long)]
        foreground: bool,

        /// Disable cloud phone-home
        #[arg(long)]
        no_phone_home: bool,

        /// Stream boot initialization progress via socket before attach (internal use)
        #[arg(long, hide = true)]
        boot_progress: bool,

        /// Explicit supervisor name (internal use)
        #[arg(long, hide = true)]
        supervisor_name: Option<String>,

        /// Explicit worker names (internal use, repeat per worker)
        #[arg(long = "worker-name", hide = true)]
        worker_names: Vec<String>,
    },

    /// Check if worktree is behind its sync target (used as SessionStart hook)
    CheckStaleness {
        /// Target branch to check against (auto-detected if not specified)
        #[arg(long, short = 'b')]
        branch: Option<String>,

        /// Fetch from remote before checking
        #[arg(long)]
        fetch: bool,
    },

    /// Sync worktree to its sync target (fetches when target is remote-tracking)
    Sync {
        /// Target branch to sync to (auto-detected if not specified)
        #[arg(long, short = 'b')]
        branch: Option<String>,
    },

    /// List known sessions (JSON-friendly; prefer `cas list --json`)
    Sessions {
        /// Only show sessions that can currently be attached to
        #[arg(long)]
        attachable_only: bool,
    },

    /// Show agent status for a session (reads CAS AgentStore; does not attach)
    Agents {
        /// Session name (default: most recent attachable session for this project)
        #[arg(long)]
        session: Option<String>,

        /// Project directory to scope session discovery (default: current directory)
        #[arg(long)]
        project_dir: Option<std::path::PathBuf>,

        /// Include all active agents in the project store (not just this session's agents)
        #[arg(long)]
        all: bool,

        /// Explicit CAS root (.cas directory) to use instead of resolving from project_dir/session metadata
        #[arg(long)]
        cas_root: Option<std::path::PathBuf>,
    },

    /// Show recent activity events for a session (reads CAS EventStore)
    Activity {
        /// Session name (default: most recent attachable session for this project)
        #[arg(long)]
        session: Option<String>,

        /// Project directory to scope session discovery (default: current directory)
        #[arg(long)]
        project_dir: Option<std::path::PathBuf>,

        /// Include all recent events in the project store (not just this session's agents)
        #[arg(long)]
        all: bool,

        /// Max events to return
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Explicit CAS root (.cas directory) to use instead of resolving from project_dir/session metadata
        #[arg(long)]
        cas_root: Option<std::path::PathBuf>,
    },

    /// Aggregated status snapshot for a session (ideal for external tools)
    Status {
        /// Session name (default: most recent attachable session for this project)
        #[arg(long)]
        session: Option<String>,

        /// Project directory to scope session discovery (default: current directory)
        #[arg(long)]
        project_dir: Option<std::path::PathBuf>,

        /// Max activity events to return
        #[arg(long, default_value = "20")]
        activity_limit: usize,

        /// Explicit CAS root (.cas directory) to use instead of resolving from project_dir/session metadata
        #[arg(long)]
        cas_root: Option<std::path::PathBuf>,
    },

    /// List valid messaging targets for a session (supervisor/workers/all_workers)
    Targets {
        /// Session name (default: most recent attachable session for this project)
        #[arg(long)]
        session: Option<String>,

        /// Project directory to scope session discovery (default: current directory)
        #[arg(long)]
        project_dir: Option<std::path::PathBuf>,
    },

    /// Inject a message into supervisor/workers via the prompt queue (no PTY attach)
    Message {
        /// Session name (default: most recent attachable session for this project)
        #[arg(long)]
        session: Option<String>,

        /// Project directory to scope session discovery (default: current directory)
        #[arg(long)]
        project_dir: Option<std::path::PathBuf>,

        /// Target: supervisor | all_workers | <worker-name>
        #[arg(long)]
        target: String,

        /// Message text to enqueue
        #[arg(long)]
        message: String,

        /// Source label for attribution in wrapped message (default: openclaw)
        #[arg(long, default_value = "openclaw")]
        from: String,

        /// Enqueue the raw message without wrapping in an XML `<message>` tag
        #[arg(long)]
        no_wrap: bool,

        /// Wait until the factory daemon records an injection event for this message ID
        #[arg(long)]
        wait_ack: bool,

        /// Timeout in milliseconds for --wait-ack
        #[arg(long, default_value = "5000")]
        timeout_ms: u64,

        /// Explicit CAS root (.cas directory) to use instead of resolving from project_dir/session metadata
        #[arg(long)]
        cas_root: Option<std::path::PathBuf>,
    },
}

pub fn execute(args: &FactoryArgs, cli: &Cli, cas_root: Option<&std::path::Path>) -> Result<()> {
    if let Some(ref cmd) = args.command {
        return match cmd {
            FactoryCommands::Daemon {
                session,
                cwd,
                workers,
                no_worktrees,
                worktree_root,
                notify,
                supervisor_cli,
                worker_cli,
                tabbed,
                record,
                no_phone_home,
                foreground,
                boot_progress,
                supervisor_name,
                worker_names,
            } => daemon::execute_daemon(
                session,
                cwd,
                *workers,
                *no_worktrees,
                worktree_root.clone(),
                *notify,
                *tabbed,
                *record,
                !*no_phone_home,
                parse_supervisor_cli(supervisor_cli)?,
                parse_supervisor_cli(worker_cli)?,
                *foreground,
                *boot_progress,
                supervisor_name.clone(),
                worker_names.clone(),
            ),
            FactoryCommands::CheckStaleness { branch, fetch } => {
                worktree_ops::execute_check_staleness(branch.as_deref(), *fetch)
            }
            FactoryCommands::Sync { branch } => worktree_ops::execute_sync(branch.as_deref()),
            FactoryCommands::Sessions { attachable_only } => {
                queries::execute_sessions(cli, *attachable_only)
            }
            FactoryCommands::Agents {
                session,
                project_dir,
                all,
                cas_root,
            } => queries::execute_agents(
                cli,
                session.as_deref(),
                project_dir.as_deref(),
                *all,
                cas_root.as_deref(),
            ),
            FactoryCommands::Activity {
                session,
                project_dir,
                all,
                limit,
                cas_root,
            } => queries::execute_activity(
                cli,
                session.as_deref(),
                project_dir.as_deref(),
                *all,
                *limit,
                cas_root.as_deref(),
            ),
            FactoryCommands::Status {
                session,
                project_dir,
                activity_limit,
                cas_root,
            } => queries::execute_status(
                cli,
                session.as_deref(),
                project_dir.as_deref(),
                *activity_limit,
                cas_root.as_deref(),
            ),
            FactoryCommands::Targets {
                session,
                project_dir,
            } => queries::execute_targets(cli, session.as_deref(), project_dir.as_deref()),
            FactoryCommands::Message {
                session,
                project_dir,
                target,
                message,
                from,
                no_wrap,
                wait_ack,
                timeout_ms,
                cas_root,
            } => queries::execute_message(
                cli,
                session.as_deref(),
                project_dir.as_deref(),
                target,
                message,
                from,
                *no_wrap,
                *wait_ack,
                *timeout_ms,
                cas_root.as_deref(),
            ),
        };
    }

    if args.cleanup {
        return worktree_ops::execute_cleanup(args);
    }

    if args.workers > 6 {
        bail!("Maximum 6 workers supported in factory mode");
    }

    let cwd = std::env::current_dir()?;

    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        let hints = noninteractive_factory_hints(args, &cwd, cas_root);
        let mut msg = String::from(
            "Factory mode requires an interactive terminal.\n\n\
             Run this command in a terminal (not a non-interactive shell/pipe).\n\
             For automation, use non-interactive commands like `cas list`, `cas status`, or `cas factory status`.",
        );
        if !hints.is_empty() {
            msg.push_str("\n\nBefore launching factory interactively:");
            for hint in hints {
                msg.push_str(&format!("\n  - {hint}"));
            }
        }
        bail!(msg);
    }

    // Apply [llm] config harness defaults when CLI args are at their defaults.
    // CLI args explicitly set by the user take precedence over config values.
    let mut effective_args = args.clone();
    let cas_dir_buf = cwd.join(".cas");
    let effective_cas_dir = cas_root.or_else(|| {
        if cas_dir_buf.exists() {
            Some(cas_dir_buf.as_path())
        } else {
            None
        }
    });
    if let Some(cas_dir) = effective_cas_dir {
        if let Ok(cfg) = Config::load(cas_dir) {
            let llm = cfg.llm();
            if effective_args.supervisor_cli == "claude" {
                effective_args.supervisor_cli = llm.harness_for_role("supervisor").to_string();
            }
            if effective_args.worker_cli == "claude" {
                effective_args.worker_cli = llm.harness_for_role("worker").to_string();
            }
        }
    }
    let args = &effective_args;

    let preflight = preflight_factory_launch(args, &cwd, cas_root)?;
    if !preflight.notices.is_empty() {
        let theme = crate::ui::theme::ActiveTheme::default();
        let mut stdout = std::io::stdout();
        let mut fmt = crate::ui::components::Formatter::stdout(&mut stdout, theme);
        for note in &preflight.notices {
            fmt.info(note)?;
        }
    }

    // Auto-attach to existing session, or kill it if --new
    if !args.legacy && args.name.is_none() {
        let project_dir = cwd.to_string_lossy();
        if let Ok(Some(session)) = find_session_for_project(&project_dir, None) {
            if session.can_attach() {
                if args.start_new {
                    // --new: kill existing session before starting fresh
                    let theme = crate::ui::theme::ActiveTheme::default();
                    let mut stdout = std::io::stdout();
                    let mut fmt = crate::ui::components::Formatter::stdout(&mut stdout, theme);
                    fmt.info(&format!(
                        "Found running session: {} (workers: {}, pid: {})",
                        session.name,
                        session.worker_count(),
                        session.metadata.daemon_pid
                    ))?;
                    if crate::cli::interactive::confirm(
                        "Kill existing session and start fresh?",
                        true,
                    )? {
                        lifecycle::kill_session_if_running(&session.name)?;
                        fmt.info("Killed. Starting new session...")?;
                        fmt.newline()?;
                    } else {
                        fmt.info("Attaching to existing session instead.")?;
                        fmt.newline()?;
                        return attach(Some(session.name));
                    }
                } else if args.attach {
                    // --attach: skip prompt, attach directly
                    let theme = crate::ui::theme::ActiveTheme::default();
                    let mut stdout = std::io::stdout();
                    let mut fmt = crate::ui::components::Formatter::stdout(&mut stdout, theme);
                    fmt.info(&format!(
                        "Attaching to running session: {}",
                        session.name
                    ))?;
                    fmt.newline()?;
                    return attach(Some(session.name));
                } else {
                    // Default: prompt user
                    let theme = crate::ui::theme::ActiveTheme::default();
                    let mut stdout = std::io::stdout();
                    let mut fmt = crate::ui::components::Formatter::stdout(&mut stdout, theme);
                    fmt.info(&format!(
                        "Found running session: {} (workers: {}, pid: {})",
                        session.name,
                        session.worker_count(),
                        session.metadata.daemon_pid
                    ))?;
                    if crate::cli::interactive::confirm(
                        "Attach to existing session?",
                        true,
                    )? {
                        fmt.newline()?;
                        return attach(Some(session.name));
                    } else {
                        fmt.info("Starting new session... (use --new to skip this prompt)")?;
                        fmt.newline()?;
                    }
                }
            }
        }
    }

    // Determine theme variant early so we can use themed names
    let theme_variant = {
        let cd = cwd.join(".cas");
        let cr = cas_root.or_else(|| if cd.exists() { Some(cd.as_path()) } else { None });
        cr.and_then(|r| Config::load(r).ok())
            .and_then(|c| c.theme.as_ref().map(|t| t.variant))
            .unwrap_or_default()
    };
    let is_minions = theme_variant == crate::ui::theme::ThemeVariant::Minions;

    let (supervisor_name, worker_names) = if is_minions {
        use crate::orchestration::names::{generate_minion_supervisor, generate_minion_unique};
        let sup = generate_minion_supervisor();
        let workers = generate_minion_unique(args.workers as usize);
        (sup, workers)
    } else {
        let all_names = generate_unique(args.workers as usize + 1);
        let sup = all_names[0].clone();
        let workers: Vec<String> = all_names[1..].to_vec();
        (sup, workers)
    };

    let session_name = args
        .name
        .clone()
        .unwrap_or_else(|| generate_session_name(Some(&cwd.to_string_lossy())));

    let orphans_killed = lifecycle::cleanup_orphaned_daemons();
    if orphans_killed > 0 {
        tracing::info!("Cleaned up {} orphaned daemon(s)", orphans_killed);
    }

    if args.name.is_some() {
        match lifecycle::kill_session_if_running(&session_name) {
            Ok(true) => tracing::info!("Killed existing session: {}", session_name),
            Ok(false) => {}
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "Failed to kill existing session '{session_name}': {e}"
                ));
            }
        }
    }

    let notify_config = NotifyConfig {
        enabled: args.notify,
        backend: NotifyBackend::detect(),
        also_bell: args.bell,
    };

    let cas_config = Config::load(&preflight.cas_root).unwrap_or_default();
    let auto_prompt = cas_config.orchestration().auto_prompt;
    let llm = cas_config.llm();

    // cas-0bf4: bridge factory.cargo_build_jobs + factory.nice_cargo
    // config knobs through the supervisor's process env so worker PTY
    // spawns (cas-pty::PtyConfig::{claude,codex}) can read them. Env
    // is the least-invasive transport — the alternative of threading
    // these fields through cas-cli → cas-mux → cas-pty signatures
    // would touch 3 crates for two booleans.
    //
    // Only set when the config says non-default, so external overrides
    // at the shell level (`CAS_FACTORY_CARGO_BUILD_JOBS=N cas factory ...`)
    // still win.
    {
        let fc = cas_config.factory();
        // SAFETY: std::env::set_var is only unsafe in multi-threaded
        // contexts; this executes during factory startup before any
        // worker spawn, still on the main thread. Same contract as
        // ScopedSupervisorEnv in the test harness.
        unsafe {
            if std::env::var("CAS_FACTORY_CARGO_BUILD_JOBS").is_err()
                && fc.cargo_build_jobs.trim() != "auto"
                && !fc.cargo_build_jobs.trim().is_empty()
            {
                std::env::set_var("CAS_FACTORY_CARGO_BUILD_JOBS", fc.cargo_build_jobs.trim());
            }
            if std::env::var("CAS_FACTORY_NICE_WORKER").is_err() && fc.nice_cargo {
                std::env::set_var("CAS_FACTORY_NICE_WORKER", "1");
            }
        }
    }

    // Build native Agent Teams spawn configs so agents start with Teams CLI flags.
    let (teams_configs, lead_session_id) = {
        use crate::ui::factory::daemon::runtime::teams::TeamsManager;
        TeamsManager::build_configs_for_mux(&session_name, &supervisor_name, &worker_names)
    };

    let config = FactoryConfig {
        cwd: cwd.clone(),
        workers: args.workers as usize,
        worker_names: worker_names.clone(),
        supervisor_name: Some(supervisor_name),
        supervisor_cli: preflight.supervisor_cli,
        worker_cli: preflight.worker_cli,
        supervisor_model: llm.model_for_role("supervisor").map(String::from),
        worker_model: llm.model_for_role("worker").map(String::from),
        enable_worktrees: preflight.enable_worktrees,
        worktree_root: args.worktree_root.clone(),
        notify: notify_config,
        tabbed_workers: args.tabbed,
        auto_prompt,
        record: args.record,
        session_id: if args.record {
            Some(session_name.clone())
        } else {
            None
        },
        teams_configs,
        lead_session_id: Some(lead_session_id),
        minions_theme: is_minions,
    };

    let phone_home = !args.no_phone_home;

    if args.legacy {
        daemon::execute_legacy_daemon(session_name, config, phone_home)
    } else {
        daemon::run_factory_with_daemon(session_name, config, phone_home)
    }
}

/// Attach to an existing factory session (local or remote via SSH)
pub fn execute_attach(args: &AttachArgs) -> Result<()> {
    if let Some(ref remote_target) = args.remote {
        {
            // If target contains ':', use SSH mode (device:factory-id)
            if remote_target.contains(':') {
                return remote_attach::execute_remote_attach(remote_target, args.worker.as_deref());
            }
            // Otherwise use cloud relay (just factory-id)
            return cloud_attach::execute_cloud_attach(remote_target);
        }
    }

    // If no name given and multiple sessions exist, show interactive picker
    let name = match &args.name {
        Some(n) => Some(n.clone()),
        None => {
            let manager = crate::ui::factory::SessionManager::new();
            let sessions = manager.list_sessions()?;
            let attachable: Vec<_> = sessions.iter().filter(|s| s.can_attach()).collect();
            if attachable.len() > 1 {
                let items: Vec<_> = attachable
                    .iter()
                    .map(|s| lifecycle::session_picker_item(s))
                    .collect();
                match crate::cli::interactive::pick("Attach to session", &items)? {
                    Some(idx) => Some(attachable[idx].name.clone()),
                    None => return Ok(()),
                }
            } else {
                None // let attach() handle single/zero sessions
            }
        }
    };

    attach(name)
}

/// Validate CAS is initialized in the current project
fn validate_cas_root(
    cwd: &std::path::Path,
    cas_root: Option<&std::path::Path>,
) -> Result<std::path::PathBuf> {
    match cas_root {
        Some(root) => {
            let root = root.to_path_buf();
            let cas_parent = root.parent().unwrap_or(&root);
            let is_in_cwd = cas_parent == cwd;
            let is_git_root_ancestor = {
                let mut check = cwd.to_path_buf();
                loop {
                    if check.join(".git").exists() {
                        break check == cas_parent;
                    }
                    if !check.pop() {
                        break false;
                    }
                }
            };

            if !is_in_cwd && !is_git_root_ancestor {
                bail!(
                    "CAS is not initialized in this project.\n\n\
                    Found CAS at: {}\n\
                    Current directory: {}\n\n\
                    Run 'cas init' in this project first.",
                    root.display(),
                    cwd.display()
                );
            }
            Ok(root)
        }
        None => {
            bail!(
                "CAS is not initialized in this directory.\n\n\
                Factory mode requires CAS for task coordination.\n\n\
                Run 'cas init' first to initialize CAS."
            );
        }
    }
}

fn noninteractive_factory_hints(
    args: &FactoryArgs,
    cwd: &std::path::Path,
    cas_root: Option<&std::path::Path>,
) -> Vec<String> {
    let mut hints = Vec::new();

    if validate_cas_root(cwd, cas_root).is_err() {
        hints.push("Initialize CAS first with `cas doctor --fix` (or `cas init`).".to_string());
    }

    if args.workers > 0 && !args.no_worktrees {
        if !GitOperations::is_git_available() {
            hints.push("Install git to enable worker worktree isolation.".to_string());
        } else {
            match GitOperations::detect_repo_root(cwd) {
                Ok(repo_root) => {
                    let git = GitOperations::new(repo_root);
                    if !git.has_commits().unwrap_or(false) {
                        hints.push(
                            "Create an initial commit before launching workers: `git add . && git commit -m \"Initial commit\"`."
                                .to_string(),
                        );
                    }
                }
                Err(_) => hints.push(
                    "Initialize git for worker worktrees: `git init && git add . && git commit -m \"Initial commit\"` (or use `--no-worktrees`)."
                        .to_string(),
                ),
            }
        }
    }

    hints
}

fn is_claude_installed() -> bool {
    std::process::Command::new("claude")
        .arg("--version")
        .output()
        .is_ok()
}

fn is_codex_installed() -> bool {
    std::process::Command::new("codex")
        .arg("--version")
        .output()
        .is_ok()
}

fn parse_supervisor_cli(value: &str) -> Result<cas_mux::SupervisorCli> {
    value
        .parse::<cas_mux::SupervisorCli>()
        .map_err(|_| anyhow::anyhow!("Invalid CLI '{value}'. Use 'claude' or 'codex'."))
}

struct FactoryPreflight {
    cas_root: std::path::PathBuf,
    supervisor_cli: cas_mux::SupervisorCli,
    worker_cli: cas_mux::SupervisorCli,
    enable_worktrees: bool,
    notices: Vec<String>,
}

fn resolve_cli_choice(
    role: &str,
    requested: &str,
    allow_default_fallback: bool,
    claude_installed: bool,
    codex_installed: bool,
    notices: &mut Vec<String>,
) -> Result<cas_mux::SupervisorCli> {
    let parsed = parse_supervisor_cli(requested)?;

    // Check if the requested CLI binary is available on PATH
    let is_installed = |cli: cas_mux::SupervisorCli| -> bool {
        match cli {
            cas_mux::SupervisorCli::Claude => claude_installed,
            cas_mux::SupervisorCli::Codex => codex_installed,
        }
    };

    if is_installed(parsed) {
        return Ok(parsed);
    }

    // Fallback logic for Claude <-> Codex (existing behavior)
    match parsed {
        cas_mux::SupervisorCli::Claude if allow_default_fallback && codex_installed => {
            notices.push(format!(
                "{role} defaulted from 'claude' to 'codex' because Claude CLI is not installed."
            ));
            Ok(cas_mux::SupervisorCli::Codex)
        }
        cas_mux::SupervisorCli::Codex if allow_default_fallback && claude_installed => {
            notices.push(format!(
                "{role} defaulted from 'codex' to 'claude' because Codex CLI is not installed."
            ));
            Ok(cas_mux::SupervisorCli::Claude)
        }
        cas_mux::SupervisorCli::Claude => bail!(
            "{role} 'claude' is not installed. Install with: npm install -g @anthropic-ai/claude-cli"
        ),
        cas_mux::SupervisorCli::Codex => bail!(
            "{role} 'codex' is not installed. Install from https://developers.openai.com/codex"
        ),
    }
}

fn preflight_factory_launch(
    args: &FactoryArgs,
    cwd: &std::path::Path,
    cas_root: Option<&std::path::Path>,
) -> Result<FactoryPreflight> {
    let mut failures: Vec<String> = Vec::new();
    let mut notices: Vec<String> = Vec::new();
    let mut missing_cas = false;
    let mut missing_git_repo = false;
    let mut missing_initial_commit = false;
    let mut missing_claude_commit = false;
    let mut missing_mcp_commit = false;

    let resolved_cas_root = match validate_cas_root(cwd, cas_root) {
        Ok(path) => Some(path),
        Err(_) => {
            failures.push("CAS is not initialized in this project. Run `cas init`.".to_string());
            missing_cas = true;
            None
        }
    };

    let claude_installed = is_claude_installed();
    let codex_installed = is_codex_installed();

    let supervisor_cli = match resolve_cli_choice(
        "Supervisor CLI",
        &args.supervisor_cli,
        args.supervisor_cli == "claude",
        claude_installed,
        codex_installed,
        &mut notices,
    ) {
        Ok(cli) => Some(cli),
        Err(e) => {
            failures.push(e.to_string());
            None
        }
    };
    let worker_cli = if args.workers > 0 {
        match resolve_cli_choice(
            "Worker CLI",
            &args.worker_cli,
            args.worker_cli == "claude",
            claude_installed,
            codex_installed,
            &mut notices,
        ) {
            Ok(cli) => Some(cli),
            Err(e) => {
                failures.push(e.to_string());
                None
            }
        }
    } else {
        resolve_cli_choice(
            "Worker CLI",
            &args.worker_cli,
            args.worker_cli == "claude",
            claude_installed,
            codex_installed,
            &mut notices,
        )
        .ok()
        .or(supervisor_cli)
    };

    let mut enable_worktrees = !args.no_worktrees;
    if enable_worktrees {
        if !GitOperations::is_git_available() {
            if args.workers == 0 {
                enable_worktrees = false;
                notices.push(
                    "Git not found; starting supervisor-only in shared-directory mode. Install git to enable worktree isolation."
                        .to_string(),
                );
            } else {
                failures.push(
                    "Git is required for default factory worktrees. Install git.".to_string(),
                );
            }
        } else {
            match GitOperations::detect_repo_root(cwd) {
                Ok(repo_root) => {
                    let git = GitOperations::new(repo_root);
                    if !git.has_commits().unwrap_or(false) {
                        if args.workers == 0 {
                            enable_worktrees = false;
                            notices.push(
                                "No initial commit detected; starting supervisor-only in shared-directory mode. Create a first commit to enable worktree isolation."
                                    .to_string(),
                            );
                        } else {
                            failures.push(
                                "Repository has no commits. Create an initial commit before starting factory."
                                    .to_string(),
                            );
                            missing_initial_commit = true;
                        }
                    }
                }
                Err(_) => {
                    if args.workers == 0 {
                        enable_worktrees = false;
                        notices.push(
                            "Not in a git repository; starting supervisor-only in shared-directory mode. Run `git init` + first commit to enable worktree isolation."
                                .to_string(),
                        );
                    } else {
                        failures.push(
                            "Default factory mode requires a git repository. Run `git init` or use `cas factory --no-worktrees`."
                                .to_string(),
                        );
                        missing_git_repo = true;
                    }
                }
            }
        }
    }

    // Check if .claude/ is committed (required for worktree-based workers)
    if enable_worktrees && !missing_git_repo && !missing_initial_commit {
        let claude_tracked = std::process::Command::new("git")
            .args(["ls-files", "--error-unmatch", ".claude/settings.json"])
            .current_dir(cwd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !claude_tracked {
            if args.workers > 0 {
                failures.push(
                    ".claude/ directory is not committed. Workers need it in their worktrees."
                        .to_string(),
                );
                missing_claude_commit = true;
            } else {
                notices.push(
                    ".claude/ directory is not committed. Commit it before spawning workers: git add .claude/ CLAUDE.md .mcp.json .gitignore && git commit -m \"Configure CAS\""
                        .to_string(),
                );
            }
        }
    }

    // Check if .mcp.json is committed (required for worktree-based workers)
    if enable_worktrees && !missing_git_repo && !missing_initial_commit {
        let mcp_tracked = std::process::Command::new("git")
            .args(["ls-files", "--error-unmatch", ".mcp.json"])
            .current_dir(cwd)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);

        if !mcp_tracked {
            if args.workers > 0 {
                failures.push(
                    ".mcp.json is not committed. Workers need it for MCP tool access in their worktrees."
                        .to_string(),
                );
                missing_mcp_commit = true;
            } else {
                notices.push(
                    ".mcp.json is not committed. Commit it before spawning workers: git add .mcp.json && git commit -m \"Configure CAS MCP\""
                        .to_string(),
                );
            }
        }
    }

    if !failures.is_empty() {
        let details = failures
            .iter()
            .map(|f| format!("  - {f}"))
            .collect::<Vec<_>>()
            .join("\n");

        let mut msg = String::from("Factory preflight failed:\n");
        msg.push_str(&details);

        let mut steps: Vec<String> = Vec::new();
        if missing_git_repo {
            steps.push("git init".to_string());
        }
        if missing_git_repo || missing_initial_commit {
            steps.push("git add .".to_string());
            steps.push("git commit -m \"Initial commit\"".to_string());
        }
        if missing_cas {
            steps.push("cas init".to_string());
        }
        if missing_claude_commit {
            steps.push("git add .claude/ CLAUDE.md .mcp.json .gitignore".to_string());
            steps.push("git commit -m \"Configure CAS\"".to_string());
        }
        if missing_mcp_commit && !missing_claude_commit {
            steps.push("git add .mcp.json".to_string());
            steps.push("git commit -m \"Configure CAS MCP\"".to_string());
        }
        let launch = if args.no_worktrees {
            "cas factory --no-worktrees"
        } else {
            "cas"
        };
        if !steps.is_empty() {
            steps.push(launch.to_string());
            msg.push_str("\n\nQuick start:");
            for (i, step) in steps.iter().enumerate() {
                msg.push_str(&format!("\n  {}) {}", i + 1, step));
            }
        }
        bail!(msg);
    }

    Ok(FactoryPreflight {
        cas_root: resolved_cas_root.expect("preflight must set cas_root on success"),
        supervisor_cli: supervisor_cli.expect("preflight must parse supervisor_cli on success"),
        worker_cli: worker_cli.expect("preflight must parse worker_cli on success"),
        enable_worktrees,
        notices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::factory::FactoryConfig;

    #[test]
    fn test_factory_config_default() {
        let config = FactoryConfig::default();
        assert_eq!(config.workers, 0);
        assert!(config.worker_names.is_empty());
        assert!(config.supervisor_name.is_none());
        assert_eq!(config.supervisor_cli, cas_mux::SupervisorCli::Claude);
        assert_eq!(config.worker_cli, cas_mux::SupervisorCli::Claude);
        assert!(config.enable_worktrees);
        assert!(config.worktree_root.is_none());
    }

    #[test]
    fn test_factory_args_default_has_attach_false() {
        let args = FactoryArgs::default();
        assert!(!args.attach, "--attach should default to false");
        assert!(!args.start_new, "--new should default to false");
    }

    #[test]
    fn test_factory_args_attach_and_new_are_independent() {
        // Both flags can be set independently
        let mut args = FactoryArgs::default();
        args.attach = true;
        assert!(args.attach);
        assert!(!args.start_new);

        let mut args = FactoryArgs::default();
        args.start_new = true;
        assert!(!args.attach);
        assert!(args.start_new);
    }

    #[test]
    fn test_session_manager_no_sessions() {
        // Characterization: find_session_for_project returns None when no sessions exist
        let manager = crate::ui::factory::SessionManager::new();
        let result = manager
            .find_session_for_project(None, "/nonexistent/project/path")
            .unwrap();
        assert!(result.is_none());
    }
}
