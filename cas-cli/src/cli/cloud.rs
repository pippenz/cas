//! Cloud sync commands for CAS
//!
//! Enables syncing CAS data with CAS Cloud service.

use clap::{Parser, Subcommand};
use std::io;
use std::path::Path;
use std::time::Duration;

use crate::cli::Cli;
use crate::cloud::{CloudConfig, get_project_canonical_id};
use crate::ui::components::Formatter;
use crate::ui::theme::ActiveTheme;

use crate::store::{
    SqliteStore, open_commit_link_store, open_event_store, open_file_change_store,
    open_prompt_store, open_rule_store, open_skill_store, open_spec_store, open_store,
    open_task_store, open_worktree_store,
};

#[derive(Subcommand)]
pub enum CloudCommands {
    /// Show cloud sync status
    Status,
    /// Show sync queue (pending changes)
    Queue(CloudQueueArgs),
    /// Push local data to cloud
    Push(CloudPushArgs),
    /// Pull data from cloud
    Pull(CloudPullArgs),
    /// Full sync (push then pull)
    Sync(CloudSyncArgs),
    /// List team projects in cloud
    Projects(CloudProjectsArgs),
    /// Pull team memories for the current project
    TeamMemories(CloudTeamMemoriesArgs),
    /// Remove foreign-project entities from local DB and re-pull
    PurgeForeign(CloudPurgeForeignArgs),
}

#[derive(Parser)]
pub struct CloudPushArgs {
    /// Push only entries
    #[arg(long)]
    pub entries_only: bool,

    /// Push only tasks
    #[arg(long)]
    pub tasks_only: bool,

    /// Dry run (don't actually push)
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser)]
pub struct CloudPullArgs {
    /// Pull only entries
    #[arg(long)]
    pub entries_only: bool,

    /// Pull only tasks
    #[arg(long)]
    pub tasks_only: bool,

    /// Pull all data (ignore last sync time)
    #[arg(long)]
    pub full: bool,
}

#[derive(Parser)]
pub struct CloudSyncArgs {
    /// Dry run (don't actually sync)
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser)]
pub struct CloudProjectsArgs {
    /// Specify team slug (defaults to active team)
    #[arg(long)]
    pub team: Option<String>,
}

#[derive(Parser)]
pub struct CloudTeamMemoriesArgs {
    /// Show what would be pulled without merging
    #[arg(long)]
    pub dry_run: bool,

    /// Ignore last sync timestamp, pull everything
    #[arg(long)]
    pub full: bool,
}

#[derive(Parser)]
pub struct CloudPurgeForeignArgs {
    /// Preview what would be purged without deleting
    #[arg(long)]
    pub dry_run: bool,
}

#[derive(Parser)]
pub struct CloudQueueArgs {
    /// Show detailed list of queued items
    #[arg(long, short)]
    pub verbose: bool,

    /// Maximum items to show
    #[arg(long, default_value = "20")]
    pub limit: usize,

    /// Clear failed items older than N days
    #[arg(long)]
    pub prune: Option<i64>,

    /// Clear all items from the queue
    #[arg(long)]
    pub clear: bool,
}

pub fn execute(cmd: &CloudCommands, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    match cmd {
        CloudCommands::Status => execute_status(cli, cas_root),
        CloudCommands::Queue(args) => execute_queue(args, cli, cas_root),
        CloudCommands::Push(args) => execute_push(args, cli, cas_root),
        CloudCommands::Pull(args) => execute_pull(args, cli, cas_root),
        CloudCommands::Sync(args) => execute_sync(args, cli, cas_root),
        CloudCommands::Projects(args) => execute_projects(args, cli),
        CloudCommands::TeamMemories(args) => execute_team_memories(args, cli, cas_root),
        CloudCommands::PurgeForeign(args) => execute_purge_foreign(args, cli, cas_root),
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// LOGIN - Polished TUI with Device Flow
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_status(cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    let config = CloudConfig::load()?;

    if config.token.is_none() {
        if cli.json {
            println!(r#"{{"status":"not_logged_in"}}"#);
        } else {
            let theme = ActiveTheme::default();
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            let warning_color = fmt.theme().palette.status_warning;
            fmt.write_colored("  \u{25CF} ", warning_color)?;
            fmt.write_raw("Not logged in to CAS Cloud")?;
            fmt.newline()?;
            fmt.write_raw("  Run ")?;
            fmt.write_accent("cas login")?;
            fmt.write_raw(" to authenticate")?;
            fmt.newline()?;
        }
        return Ok(());
    }

    {
        let status_url = format!("{}/api/sync/status", config.endpoint);
        let token = config.token.as_ref().unwrap();

        match ureq::get(&status_url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()
        {
            Ok(resp) => {
                let body: serde_json::Value = resp.into_json()?;

                if cli.json {
                    println!("{}", serde_json::to_string(&body)?);
                } else {
                    let theme = ActiveTheme::default();
                    let mut out = io::stdout();
                    let mut fmt = Formatter::stdout(&mut out, theme);
                    let success_color = fmt.theme().palette.status_success;
                    let warning_color = fmt.theme().palette.status_warning;

                    fmt.newline()?;
                    fmt.write_colored("  \u{25CF} ", success_color)?;
                    fmt.write_raw("CAS Cloud")?;
                    fmt.newline()?;
                    fmt.newline()?;

                    if let Some(email) = &config.email {
                        fmt.write_muted("  Email:  ")?;
                        fmt.write_raw(email)?;
                        fmt.newline()?;
                    }
                    fmt.write_muted("  Server: ")?;
                    fmt.write_raw(&config.endpoint)?;
                    fmt.newline()?;

                    if let Some(state) = body.get("sync_state") {
                        fmt.newline()?;
                        fmt.write_muted("  Entries: ")?;
                        fmt.write_raw(
                            &state
                                .get("entry_count")
                                .unwrap_or(&serde_json::json!(0))
                                .to_string(),
                        )?;
                        fmt.newline()?;
                        fmt.write_muted("  Tasks:  ")?;
                        fmt.write_raw(
                            &state
                                .get("task_count")
                                .unwrap_or(&serde_json::json!(0))
                                .to_string(),
                        )?;
                        fmt.newline()?;
                    }

                    // Show local queue stats
                    if let Ok(queue) = crate::cloud::SyncQueue::open(cas_root) {
                        if queue.init().is_ok() {
                            if let Ok(stats) = queue.stats(5) {
                                if stats.total > 0 {
                                    fmt.newline()?;
                                    fmt.write_colored("  \u{25CF} ", warning_color)?;
                                    fmt.write_raw("Sync Queue")?;
                                    fmt.newline()?;
                                    fmt.write_raw(&format!(
                                        "    {} pending, {} failed",
                                        stats.pending, stats.failed
                                    ))?;
                                    fmt.newline()?;
                                    fmt.write_raw("    Run ")?;
                                    fmt.write_accent("cas cloud queue")?;
                                    fmt.write_raw(" for details")?;
                                    fmt.newline()?;
                                }
                            }
                        }
                    }
                    fmt.newline()?;
                }
            }
            Err(ureq::Error::Status(401, _)) => {
                if cli.json {
                    println!(r#"{{"status":"error","message":"Invalid token"}}"#);
                } else {
                    let theme = ActiveTheme::default();
                    let mut err = io::stderr();
                    let mut fmt = Formatter::stdout(&mut err, theme);
                    let error_color = fmt.theme().palette.status_error;
                    fmt.write_colored("  \u{2717} ", error_color)?;
                    fmt.write_raw("Session expired")?;
                    fmt.newline()?;
                    fmt.write_raw("  Run ")?;
                    fmt.write_accent("cas login")?;
                    fmt.write_raw(" to re-authenticate")?;
                    fmt.newline()?;
                }
            }
            Err(e) => {
                anyhow::bail!("Failed to connect: {e}");
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// QUEUE - View and manage sync queue
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_queue(args: &CloudQueueArgs, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    use crate::cloud::SyncQueue;

    let queue = SyncQueue::open(cas_root)?;
    queue.init()?;

    // Handle clear operation
    if args.clear {
        queue.clear()?;
        if cli.json {
            println!(r#"{{"status":"ok","message":"Queue cleared"}}"#);
        } else {
            let theme = ActiveTheme::default();
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            fmt.success("Queue cleared")?;
        }
        return Ok(());
    }

    // Handle prune operation
    if let Some(days) = args.prune {
        let max_retries = 5; // Default max retries
        let pruned = queue.prune_failed(days, max_retries)?;
        if cli.json {
            println!(r#"{{"status":"ok","pruned":{pruned}}}"#);
        } else {
            let theme = ActiveTheme::default();
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            fmt.success(&format!(
                "Pruned {} failed items older than {} days",
                pruned, days
            ))?;
        }
        return Ok(());
    }

    // Show queue stats
    let max_retries = 5;
    let stats = queue.stats(max_retries)?;

    if cli.json {
        if args.verbose {
            let items = queue.list_all(args.limit)?;
            println!(
                "{}",
                serde_json::json!({
                    "stats": stats,
                    "items": items
                })
            );
        } else {
            println!("{}", serde_json::to_string(&stats)?);
        }
    } else {
        let theme = ActiveTheme::default();
        let mut out = io::stdout();
        let mut fmt = Formatter::stdout(&mut out, theme);

        if stats.total == 0 {
            let success_color = fmt.theme().palette.status_success;
            fmt.write_colored("  \u{25CF} ", success_color)?;
            fmt.write_raw("Sync queue is empty")?;
            fmt.newline()?;
            return Ok(());
        }

        let accent_color = fmt.theme().palette.accent;
        let error_color = fmt.theme().palette.status_error;
        let warning_color = fmt.theme().palette.status_warning;

        fmt.newline()?;
        fmt.write_colored("  \u{25CF} ", accent_color)?;
        fmt.write_raw("Sync Queue")?;
        fmt.newline()?;
        fmt.newline()?;
        fmt.write_muted("  Total:   ")?;
        fmt.write_raw(&stats.total.to_string())?;
        fmt.newline()?;
        fmt.write_muted("  Pending: ")?;
        fmt.write_raw(&stats.pending.to_string())?;
        fmt.newline()?;
        fmt.write_muted("  Failed:  ")?;
        fmt.write_raw(&stats.failed.to_string())?;
        fmt.newline()?;

        if !stats.by_type.is_empty() {
            fmt.newline()?;
            fmt.write_muted("  By type:")?;
            fmt.newline()?;
            for (entity_type, count) in &stats.by_type {
                fmt.write_raw(&format!("    {entity_type}: {count}"))?;
                fmt.newline()?;
            }
        }

        if let Some(oldest) = &stats.oldest_item {
            fmt.newline()?;
            fmt.write_muted("  Oldest: ")?;
            fmt.write_raw(oldest)?;
            fmt.newline()?;
        }

        // Show detailed list if verbose
        if args.verbose {
            let items = queue.list_all(args.limit)?;
            if !items.is_empty() {
                fmt.newline()?;
                fmt.write_muted("  Queued items:")?;
                fmt.newline()?;
                for item in items {
                    fmt.write_raw("    ")?;
                    if item.retry_count >= max_retries {
                        fmt.write_colored("\u{2717}", error_color)?;
                    } else if item.retry_count > 0 {
                        fmt.write_colored("\u{21BB}", warning_color)?;
                    } else {
                        fmt.write_muted("\u{25CB}")?;
                    }
                    fmt.write_raw(&format!(
                        " {} {} ({})",
                        item.operation.as_str(),
                        item.entity_id,
                        item.entity_type.as_str()
                    ))?;
                    fmt.newline()?;

                    if item.retry_count > 0 {
                        fmt.write_muted("      ")?;
                        fmt.write_raw(&format!(" retries: {}", item.retry_count))?;
                        fmt.newline()?;
                    }
                    if let Some(err) = &item.last_error {
                        fmt.write_muted("      ")?;
                        fmt.write_raw(&format!(" error: {}", err))?;
                        fmt.newline()?;
                    }
                }
            }
        }
        fmt.newline()?;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// PUSH
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_push(args: &CloudPushArgs, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    let config = CloudConfig::load()?;
    let token = config
        .token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'cas login' first"))?;

    let store = open_store(cas_root)?;
    let task_store = open_task_store(cas_root)?;
    let rule_store = open_rule_store(cas_root)?;
    let skill_store = open_skill_store(cas_root)?;
    let sqlite_store = SqliteStore::open(cas_root)?;
    let spec_store = open_spec_store(cas_root)?;
    let event_store = open_event_store(cas_root)?;
    let prompt_store = open_prompt_store(cas_root)?;
    let file_change_store = open_file_change_store(cas_root)?;
    let commit_link_store = open_commit_link_store(cas_root)?;

    // Collect data to push
    let mut entries_json = Vec::new();
    let mut tasks_json = Vec::new();
    let mut rules_json = Vec::new();
    let mut skills_json = Vec::new();
    let mut sessions_json = Vec::new();
    let mut specs_json = Vec::new();
    let mut events_json = Vec::new();
    let mut prompts_json = Vec::new();
    let mut file_changes_json = Vec::new();
    let mut commit_links_json = Vec::new();

    if !args.tasks_only {
        let entries = store.list()?;
        for entry in entries {
            entries_json.push(serde_json::to_value(&entry)?);
        }
    }

    if !args.entries_only {
        let tasks = task_store.list(None)?;
        for task in tasks {
            tasks_json.push(serde_json::to_value(&task)?);
        }
    }

    // Always push rules and skills
    let rules = rule_store.list()?;
    for rule in rules {
        rules_json.push(serde_json::to_value(&rule)?);
    }

    let skills = skill_store.list(None)?;
    for skill in skills {
        skills_json.push(serde_json::to_value(&skill)?);
    }

    // Always push sessions (they're lightweight)
    let sessions = sqlite_store
        .list_sessions_since(chrono::Utc::now() - chrono::Duration::days(90))
        .unwrap_or_default();
    for session in sessions {
        sessions_json.push(serde_json::to_value(&session)?);
    }

    // Always push specs
    let specs = spec_store.list(None)?;
    for spec in specs {
        specs_json.push(serde_json::to_value(&spec)?);
    }

    // Push events (last 90 days)
    let events = event_store.list_recent(10000).unwrap_or_default();
    for event in events {
        events_json.push(serde_json::to_value(&event)?);
    }

    // Push prompts (last 90 days)
    let prompts = prompt_store.list_recent(10000).unwrap_or_default();
    for prompt in prompts {
        prompts_json.push(serde_json::to_value(&prompt)?);
    }

    // Push file changes (last 90 days)
    let file_changes = file_change_store.list_recent(10000).unwrap_or_default();
    for fc in file_changes {
        file_changes_json.push(serde_json::to_value(&fc)?);
    }

    // Push commit links (last 90 days)
    let commit_links = commit_link_store.list_recent(10000).unwrap_or_default();
    for cl in commit_links {
        commit_links_json.push(serde_json::to_value(&cl)?);
    }

    // Push worktrees
    let mut worktrees_json = Vec::new();
    if let Ok(worktree_store) = open_worktree_store(cas_root) {
        let worktrees = worktree_store.list().unwrap_or_default();
        for wt in worktrees {
            worktrees_json.push(serde_json::to_value(&wt)?);
        }
    }

    // Push task dependencies
    let mut task_deps_json = Vec::new();
    if !args.entries_only {
        let deps = task_store.list_dependencies(None).unwrap_or_default();
        for dep in deps {
            task_deps_json.push(serde_json::to_value(&dep)?);
        }
    }

    if args.dry_run {
        if cli.json {
            println!(
                "{}",
                serde_json::json!({
                    "dry_run": true,
                    "entries": entries_json.len(),
                    "tasks": tasks_json.len(),
                    "rules": rules_json.len(),
                    "skills": skills_json.len(),
                    "sessions": sessions_json.len(),
                    "specs": specs_json.len(),
                    "events": events_json.len(),
                    "prompts": prompts_json.len(),
                    "file_changes": file_changes_json.len(),
                    "commit_links": commit_links_json.len(),
                    "task_dependencies": task_deps_json.len(),
                    "worktrees": worktrees_json.len(),
                })
            );
        } else {
            let theme = ActiveTheme::default();
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            fmt.write_accent("  \u{2192} ")?;
            fmt.write_raw("Dry run - would push:")?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} entries", entries_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} tasks", tasks_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} rules", rules_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} skills", skills_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} sessions", sessions_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} specs", specs_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} events", events_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} prompts", prompts_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} file changes", file_changes_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} commit links", commit_links_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} task dependencies", task_deps_json.len()))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {} worktrees", worktrees_json.len()))?;
            fmt.newline()?;
        }
        return Ok(());
    }

    {
        use crate::ui::components::{
            Component, ProgressBar, ProgressBarMsg, clear_inline, render_inline_view,
            rerender_inline,
        };

        let push_url = format!("{}/api/sync/push", config.endpoint);

        // Build batches: split large collections into chunks to avoid 413 errors
        const BATCH_SIZE: usize = 50;

        let resource_types: Vec<(&str, &[serde_json::Value])> = vec![
            ("entries", &entries_json),
            ("tasks", &tasks_json),
            ("rules", &rules_json),
            ("skills", &skills_json),
            ("sessions", &sessions_json),
            ("specs", &specs_json),
            ("events", &events_json),
            ("prompts", &prompts_json),
            ("file_changes", &file_changes_json),
            ("commit_links", &commit_links_json),
            ("task_dependencies", &task_deps_json),
            ("worktrees", &worktrees_json),
        ];

        // Build list of batches: each batch is a JSON payload with chunked data
        let mut batches: Vec<serde_json::Value> = Vec::new();

        // Find the max number of chunks needed across all resource types
        let max_chunks = resource_types
            .iter()
            .map(|(_, items)| (items.len() + BATCH_SIZE - 1) / BATCH_SIZE.max(1))
            .max()
            .unwrap_or(1)
            .max(1);

        let project_id = get_project_canonical_id()
            .ok_or_else(|| anyhow::anyhow!("Cannot sync: not inside a CAS project directory"))?;

        for chunk_idx in 0..max_chunks {
            let start = chunk_idx * BATCH_SIZE;
            let mut payload = serde_json::Map::new();

            for (name, items) in &resource_types {
                let end = (start + BATCH_SIZE).min(items.len());
                let chunk = if start < items.len() {
                    &items[start..end]
                } else {
                    &[]
                };
                payload.insert(name.to_string(), serde_json::json!(chunk));
            }

            // Required by server for project scoping
            payload.insert(
                "project_canonical_id".to_string(),
                serde_json::json!(project_id),
            );
            // Client version for server-side compatibility checks
            payload.insert(
                "client_version".to_string(),
                serde_json::json!(env!("CARGO_PKG_VERSION")),
            );
            payload.insert(
                "client_build".to_string(),
                serde_json::json!(option_env!("CAS_GIT_HASH").unwrap_or("unknown")),
            );
            // Include team_id if configured
            if let Some(team_id) = &config.team_id {
                payload.insert("team_id".to_string(), serde_json::json!(team_id));
            }

            batches.push(serde_json::Value::Object(payload));
        }

        let total_items: usize = resource_types.iter().map(|(_, items)| items.len()).sum();
        let num_batches = batches.len();

        let theme = ActiveTheme::default();
        let (mut progress_bar, mut prev_lines) = if !cli.json {
            let bar = ProgressBar::new(total_items as u64).with_message("Pushing");
            let lines = render_inline_view(&bar, &theme)?;
            (Some(bar), lines)
        } else {
            (None, 0u16)
        };

        // Aggregate totals across batches
        let resource_names = [
            "entries",
            "tasks",
            "rules",
            "skills",
            "sessions",
            "specs",
            "events",
            "prompts",
            "file_changes",
            "commit_links",
            "task_dependencies",
            "worktrees",
        ];
        let mut totals: std::collections::HashMap<String, (u64, u64)> = resource_names
            .iter()
            .map(|n| (n.to_string(), (0u64, 0u64)))
            .collect();
        let mut items_pushed = 0u64;

        for (batch_idx, payload) in batches.iter().enumerate() {
            // Count items in this batch
            let batch_items: usize = resource_names
                .iter()
                .map(|name| {
                    payload
                        .get(name)
                        .and_then(|v| v.as_array())
                        .map_or(0, |a| a.len())
                })
                .sum();

            if let Some(ref mut bar) = progress_bar {
                if num_batches > 1 {
                    bar.update(ProgressBarMsg::SetMessage(format!(
                        "Pushing (batch {}/{})",
                        batch_idx + 1,
                        num_batches
                    )));
                }
                bar.update(ProgressBarMsg::Tick);
                prev_lines = rerender_inline(bar, prev_lines, &theme)?;
            }

            let response = ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(120))
                .build()
                .post(&push_url)
                .set("Authorization", &format!("Bearer {token}"))
                .set("Content-Type", "application/json")
                .send_json(payload);

            match response {
                Ok(resp) => {
                    let body: serde_json::Value = resp.into_json()?;

                    // Accumulate per-resource totals
                    for name in &resource_names {
                        if let Some(res) = body.get(name) {
                            let ins = res.get("inserted").and_then(|v| v.as_u64()).unwrap_or(0);
                            let upd = res.get("updated").and_then(|v| v.as_u64()).unwrap_or(0);
                            let entry = totals.entry(name.to_string()).or_insert((0, 0));
                            entry.0 += ins;
                            entry.1 += upd;
                        }
                    }

                    items_pushed += batch_items as u64;
                    if let Some(ref mut bar) = progress_bar {
                        bar.update(ProgressBarMsg::Set(items_pushed));
                        bar.update(ProgressBarMsg::Tick);
                        prev_lines = rerender_inline(bar, prev_lines, &theme)?;
                    }
                }
                Err(ureq::Error::Status(402, resp)) => {
                    if progress_bar.is_some() {
                        clear_inline(prev_lines)?;
                    }

                    let body: serde_json::Value = resp.into_json().unwrap_or_default();

                    if cli.json {
                        println!("{}", serde_json::to_string(&body)?);
                    } else {
                        let mut out = io::stdout();
                        let mut fmt = Formatter::stdout(&mut out, ActiveTheme::default());
                        let error_color = fmt.theme().palette.status_error;

                        let message = body
                            .get("message")
                            .and_then(|m| m.as_str())
                            .unwrap_or("Sync limit exceeded");
                        fmt.newline()?;
                        fmt.write_colored(&format!("  \u{2717} {message}"), error_color)?;
                        fmt.newline()?;
                        fmt.newline()?;
                    }
                    return Ok(());
                }
                Err(ureq::Error::Status(401, _)) => {
                    if progress_bar.is_some() {
                        clear_inline(prev_lines)?;
                    }
                    if cli.json {
                        println!(r#"{{"status":"error","message":"Invalid or expired token"}}"#);
                    } else {
                        let mut err = io::stderr();
                        let mut fmt = Formatter::stdout(&mut err, ActiveTheme::default());
                        let error_color = fmt.theme().palette.status_error;
                        fmt.write_colored("  \u{2717} ", error_color)?;
                        fmt.write_raw("Session expired")?;
                        fmt.newline()?;
                        fmt.write_raw("  Run ")?;
                        fmt.write_accent("cas login")?;
                        fmt.write_raw(" to re-authenticate")?;
                        fmt.newline()?;
                    }
                    return Ok(());
                }
                Err(e) => {
                    if progress_bar.is_some() {
                        clear_inline(prev_lines)?;
                    }
                    return Err(e.into());
                }
            }
        }

        if progress_bar.is_some() {
            clear_inline(prev_lines)?;
        }

        if cli.json {
            let json_totals: serde_json::Map<String, serde_json::Value> = totals
                .iter()
                .map(|(k, (ins, upd))| {
                    (
                        k.clone(),
                        serde_json::json!({"inserted": ins, "updated": upd}),
                    )
                })
                .collect();
            println!(
                "{}",
                serde_json::to_string(&serde_json::Value::Object(json_totals))?
            );
        } else {
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, ActiveTheme::default());
            fmt.success("Push complete")?;
            let display_order = [
                ("entries", "Entries"),
                ("tasks", "Tasks"),
                ("rules", "Rules"),
                ("skills", "Skills"),
                ("sessions", "Sessions"),
                ("specs", "Specs"),
                ("events", "Events"),
                ("prompts", "Prompts"),
                ("file_changes", "File changes"),
                ("commit_links", "Commit links"),
                ("worktrees", "Worktrees"),
            ];
            for (key, label) in &display_order {
                if let Some(&(ins, upd)) = totals.get(*key) {
                    if ins > 0 || upd > 0 {
                        fmt.write_raw(&format!("    {label}: {ins} inserted, {upd} updated"))?;
                        fmt.newline()?;
                    }
                }
            }
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// PULL
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_pull(args: &CloudPullArgs, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    let mut config = CloudConfig::load()?;
    let token = config
        .token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'cas login' first"))?
        .clone();

    let store = open_store(cas_root)?;
    let task_store = open_task_store(cas_root)?;
    let rule_store = open_rule_store(cas_root)?;
    let skill_store = open_skill_store(cas_root)?;
    let spec_store = open_spec_store(cas_root)?;
    let event_store = open_event_store(cas_root)?;
    let prompt_store = open_prompt_store(cas_root)?;
    let file_change_store = open_file_change_store(cas_root)?;
    let commit_link_store = open_commit_link_store(cas_root)?;

    {
        use crate::ui::components::{Spinner, clear_inline, render_inline_view};

        let theme = ActiveTheme::default();
        let prev_lines = if !cli.json {
            let spinner = Spinner::new("Pulling from cloud...");
            render_inline_view(&spinner, &theme)?
        } else {
            0u16
        };

        // Build pull URL with optional since parameter
        let mut pull_url = format!("{}/api/sync/pull", config.endpoint);
        if !args.full {
            if let Some(since) = &config.last_entry_sync {
                pull_url = format!("{pull_url}?since={since}");
            }
        }

        let response = ureq::get(&pull_url)
            .set("Authorization", &format!("Bearer {token}"))
            .call()?;

        let body: serde_json::Value = response.into_json()?;

        let mut entries_count = 0;
        let mut tasks_count = 0;
        let mut rules_count = 0;
        let mut skills_count = 0;
        let mut specs_count = 0;
        let mut events_count = 0;
        let mut prompts_count = 0;
        let mut file_changes_count = 0;
        let mut commit_links_count = 0;

        // Import entries
        if !args.tasks_only {
            if let Some(entries) = body.get("entries").and_then(|e| e.as_array()) {
                for entry_json in entries {
                    if let Ok(entry) =
                        serde_json::from_value::<crate::types::Entry>(entry_json.clone())
                    {
                        match store.get(&entry.id) {
                            Ok(_) => store.update(&entry)?,
                            Err(_) => store.add(&entry)?,
                        }
                        entries_count += 1;
                    }
                }
            }
        }

        // Import tasks
        if !args.entries_only {
            if let Some(tasks) = body.get("tasks").and_then(|t| t.as_array()) {
                for task_json in tasks {
                    if let Ok(task) =
                        serde_json::from_value::<crate::types::Task>(task_json.clone())
                    {
                        match task_store.get(&task.id) {
                            Ok(_) => task_store.update(&task)?,
                            Err(_) => task_store.add(&task)?,
                        }
                        tasks_count += 1;
                    }
                }
            }
        }

        // Import rules
        if let Some(rules) = body.get("rules").and_then(|r| r.as_array()) {
            for rule_json in rules {
                if let Ok(rule) = serde_json::from_value::<crate::types::Rule>(rule_json.clone()) {
                    match rule_store.get(&rule.id) {
                        Ok(_) => rule_store.update(&rule)?,
                        Err(_) => rule_store.add(&rule)?,
                    }
                    rules_count += 1;
                }
            }
        }

        // Import skills
        if let Some(skills) = body.get("skills").and_then(|s| s.as_array()) {
            for skill_json in skills {
                if let Ok(skill) = serde_json::from_value::<crate::types::Skill>(skill_json.clone())
                {
                    match skill_store.get(&skill.id) {
                        Ok(_) => skill_store.update(&skill)?,
                        Err(_) => skill_store.add(&skill)?,
                    }
                    skills_count += 1;
                }
            }
        }

        // Import specs
        if let Some(specs) = body.get("specs").and_then(|s| s.as_array()) {
            for spec_json in specs {
                if let Ok(spec) = serde_json::from_value::<crate::types::Spec>(spec_json.clone()) {
                    match spec_store.get(&spec.id) {
                        Ok(_) => spec_store.update(&spec)?,
                        Err(_) => spec_store.add(&spec)?,
                    }
                    specs_count += 1;
                }
            }
        }

        // Import events
        if let Some(events) = body.get("events").and_then(|e| e.as_array()) {
            for event_json in events {
                if let Ok(event) = serde_json::from_value::<crate::types::Event>(event_json.clone())
                {
                    let _ = event_store.record(&event);
                    events_count += 1;
                }
            }
        }

        // Import prompts
        if let Some(prompts) = body.get("prompts").and_then(|p| p.as_array()) {
            for prompt_json in prompts {
                if let Ok(prompt) =
                    serde_json::from_value::<crate::types::Prompt>(prompt_json.clone())
                {
                    let _ = prompt_store.add(&prompt);
                    prompts_count += 1;
                }
            }
        }

        // Import file changes
        if let Some(fcs) = body.get("file_changes").and_then(|f| f.as_array()) {
            for fc_json in fcs {
                if let Ok(fc) = serde_json::from_value::<crate::types::FileChange>(fc_json.clone())
                {
                    let _ = file_change_store.add(&fc);
                    file_changes_count += 1;
                }
            }
        }

        // Import commit links
        if let Some(cls) = body.get("commit_links").and_then(|c| c.as_array()) {
            for cl_json in cls {
                if let Ok(cl) = serde_json::from_value::<crate::types::CommitLink>(cl_json.clone())
                {
                    let _ = commit_link_store.add(&cl);
                    commit_links_count += 1;
                }
            }
        }

        // Update last sync time
        if let Some(pulled_at) = body.get("pulled_at").and_then(|p| p.as_str()) {
            config.last_entry_sync = Some(pulled_at.to_string());
            config.last_task_sync = Some(pulled_at.to_string());
            config.save()?;
        }

        if prev_lines > 0 {
            clear_inline(prev_lines)?;
        }

        if cli.json {
            println!(
                "{}",
                serde_json::json!({
                    "status": "ok",
                    "entries": entries_count,
                    "tasks": tasks_count,
                    "rules": rules_count,
                    "skills": skills_count,
                    "specs": specs_count,
                    "events": events_count,
                    "prompts": prompts_count,
                    "file_changes": file_changes_count,
                    "commit_links": commit_links_count,
                })
            );
        } else {
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, ActiveTheme::default());
            fmt.success("Pull complete")?;
            fmt.write_raw(&format!("    {entries_count} entries synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {tasks_count} tasks synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {rules_count} rules synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {skills_count} skills synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {specs_count} specs synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {events_count} events synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {prompts_count} prompts synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {file_changes_count} file changes synced"))?;
            fmt.newline()?;
            fmt.write_raw(&format!("    {commit_links_count} commit links synced"))?;
            fmt.newline()?;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// SYNC
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_sync(args: &CloudSyncArgs, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    execute_push(
        &CloudPushArgs {
            entries_only: false,
            tasks_only: false,
            dry_run: args.dry_run,
        },
        cli,
        cas_root,
    )?;

    if !args.dry_run {
        execute_pull(
            &CloudPullArgs {
                entries_only: false,
                tasks_only: false,
                full: false,
            },
            cli,
            cas_root,
        )?;
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// PROJECTS - List team projects
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_projects(args: &CloudProjectsArgs, cli: &Cli) -> anyhow::Result<()> {
    let config = CloudConfig::load()?;
    let token = config
        .token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'cas login' first"))?;

    // Resolve team_id: --team flag overrides config
    let team_id = args
        .team
        .as_deref()
        .or(config.team_id.as_deref())
        .or(config.team_slug.as_deref());

    let team_id = match team_id {
        Some(id) => id,
        None => {
            if cli.json {
                println!(r#"{{"status":"error","message":"No team configured"}}"#);
            } else {
                let theme = ActiveTheme::default();
                let mut out = io::stdout();
                let mut fmt = Formatter::stdout(&mut out, theme);
                let warning_color = fmt.theme().palette.status_warning;
                fmt.write_colored("  \u{25CF} ", warning_color)?;
                fmt.write_raw("No team configured. Run ")?;
                fmt.write_accent("cas cloud team set <slug>")?;
                fmt.write_raw(" first.")?;
                fmt.newline()?;
            }
            return Ok(());
        }
    };

    let url = format!("{}/api/teams/{}/projects", config.endpoint, team_id);

    match ureq::get(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .call()
    {
        Ok(resp) => {
            let body: crate::cloud::TeamProjectsResponse = resp.into_json()?;

            if cli.json {
                println!("{}", serde_json::to_string(&body.projects)?);
            } else {
                let theme = ActiveTheme::default();
                let mut out = io::stdout();
                let mut fmt = Formatter::stdout(&mut out, theme);

                fmt.newline()?;
                let team_display = args
                    .team
                    .as_deref()
                    .or(config.team_slug.as_deref())
                    .unwrap_or(team_id);
                fmt.write_muted("  Team: ")?;
                fmt.write_accent(team_display)?;
                fmt.newline()?;
                fmt.newline()?;

                if body.projects.is_empty() {
                    fmt.write_muted("  No projects found.")?;
                    fmt.newline()?;
                } else {
                    // Calculate column widths for aligned output
                    let max_name = body
                        .projects
                        .iter()
                        .map(|p| p.name.len())
                        .max()
                        .unwrap_or(0)
                        .max(4);
                    let max_canonical = body
                        .projects
                        .iter()
                        .map(|p| p.canonical_id.len())
                        .max()
                        .unwrap_or(0)
                        .max(4);

                    for project in &body.projects {
                        let contrib_label = if project.contributor_count == 1 {
                            "contributor"
                        } else {
                            "contributors"
                        };
                        let mem_label = if project.memory_count == 1 {
                            "memory"
                        } else {
                            "memories"
                        };
                        fmt.write_raw(&format!(
                            "    {:<name_w$}   {:<canonical_w$}   {} {:<14}  {} {}",
                            project.name,
                            project.canonical_id,
                            project.contributor_count,
                            contrib_label,
                            project.memory_count,
                            mem_label,
                            name_w = max_name,
                            canonical_w = max_canonical,
                        ))?;
                        fmt.newline()?;
                    }
                }
                fmt.newline()?;
            }
        }
        Err(ureq::Error::Status(401, _)) => {
            if cli.json {
                println!(r#"{{"status":"error","message":"Invalid or expired token"}}"#);
            } else {
                let theme = ActiveTheme::default();
                let mut err = io::stderr();
                let mut fmt = Formatter::stdout(&mut err, theme);
                let error_color = fmt.theme().palette.status_error;
                fmt.write_colored("  \u{2717} ", error_color)?;
                fmt.write_raw("Session expired")?;
                fmt.newline()?;
                fmt.write_raw("  Run ")?;
                fmt.write_accent("cas login")?;
                fmt.write_raw(" to re-authenticate")?;
                fmt.newline()?;
            }
        }
        Err(ureq::Error::Status(403, _)) => {
            anyhow::bail!("You're not a member of this team.");
        }
        Err(e) => {
            anyhow::bail!("Failed to fetch projects: {e}");
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// TEAM MEMORIES
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_team_memories(
    args: &CloudTeamMemoriesArgs,
    cli: &Cli,
    cas_root: &Path,
) -> anyhow::Result<()> {
    use crate::cloud::{TeamMemoriesResponse, TeamProjectsResponse};
    use crate::ui::components::{Spinner, clear_inline, render_inline_view};

    let mut config = CloudConfig::load()?;

    let team_id = config
        .team_id
        .as_ref()
        .ok_or_else(|| {
            anyhow::anyhow!("No team configured. Run `cas cloud team set <slug>` first.")
        })?
        .clone();

    let canonical_id = crate::cloud::get_project_canonical_id().ok_or_else(|| {
        anyhow::anyhow!("Not inside a CAS project directory.")
    })?;

    let token = config
        .token
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run 'cas login' first."))?
        .clone();

    let theme = ActiveTheme::default();
    let prev_lines = if !cli.json {
        let spinner = Spinner::new("Pulling team memories...");
        render_inline_view(&spinner, &theme)?
    } else {
        0u16
    };

    // Step 1: Find the project UUID by listing team projects
    let projects_url = format!("{}/api/teams/{}/projects", config.endpoint, team_id);
    let projects_resp = ureq::get(&projects_url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(30))
        .call();

    let projects_body: TeamProjectsResponse = match projects_resp {
        Ok(resp) => resp.into_json()?,
        Err(ureq::Error::Status(401, _)) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("Session expired. Run `cas login` to re-authenticate.");
        }
        Err(ureq::Error::Status(403, _)) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("You're not a member of this team.");
        }
        Err(e) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("Failed to list team projects: {e}");
        }
    };

    let project = projects_body
        .projects
        .iter()
        .find(|p| p.canonical_id == canonical_id);

    let project_uuid = match project {
        Some(p) => p.id.clone(),
        None => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!(
                "This project hasn't been synced to the team yet. Run `cas cloud sync --team` to register it."
            );
        }
    };

    // Step 2: Fetch team memories for this project
    let mut memories_url = format!(
        "{}/api/teams/{}/projects/{}/memories",
        config.endpoint, team_id, project_uuid
    );

    if !args.full {
        if let Some(since) = config.get_team_memory_sync(&canonical_id) {
            memories_url = format!("{memories_url}?since={since}");
        }
    }

    let memories_resp = ureq::get(&memories_url)
        .set("Authorization", &format!("Bearer {token}"))
        .timeout(Duration::from_secs(60))
        .call();

    let body: TeamMemoriesResponse = match memories_resp {
        Ok(resp) => resp.into_json()?,
        Err(ureq::Error::Status(401, _)) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("Session expired. Run `cas login` to re-authenticate.");
        }
        Err(ureq::Error::Status(403, _)) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("You're not a member of this team.");
        }
        Err(ureq::Error::Status(404, _)) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("Project not found in this team.");
        }
        Err(e) => {
            if prev_lines > 0 {
                clear_inline(prev_lines)?;
            }
            anyhow::bail!("Failed to fetch team memories: {e}");
        }
    };

    let entry_count = body.memories.entries.len();
    let rule_count = body.memories.rules.len();
    let skill_count = body.memories.skills.len();
    let contributor_count = body.contributors.len();

    // Dry run: just show counts
    if args.dry_run {
        if prev_lines > 0 {
            clear_inline(prev_lines)?;
        }

        if cli.json {
            println!(
                "{}",
                serde_json::json!({
                    "dry_run": true,
                    "entries": entry_count,
                    "rules": rule_count,
                    "skills": skill_count,
                    "contributors": contributor_count,
                })
            );
        } else {
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            fmt.write_accent("  \u{2192} ")?;
            fmt.write_raw(&format!(
                "Would pull: {} entries, {} rules, {} skills from {} contributors",
                entry_count, rule_count, skill_count, contributor_count
            ))?;
            fmt.newline()?;
        }
        return Ok(());
    }

    // Check if there's anything to merge
    if entry_count == 0 && rule_count == 0 && skill_count == 0 {
        if prev_lines > 0 {
            clear_inline(prev_lines)?;
        }
        if cli.json {
            println!(r#"{{"status":"ok","message":"up_to_date"}}"#);
        } else {
            let mut out = io::stdout();
            let mut fmt = Formatter::stdout(&mut out, theme);
            let success_color = fmt.theme().palette.status_success;
            fmt.write_colored("  \u{2713} ", success_color)?;
            fmt.write_raw("Team memories are up to date.")?;
            fmt.newline()?;
        }
        return Ok(());
    }

    // Merge into local stores using LWW
    let store = open_store(cas_root)?;
    let rule_store = open_rule_store(cas_root)?;
    let skill_store = open_skill_store(cas_root)?;

    let mut entries_merged = 0usize;
    let mut entries_skipped = 0usize;
    let mut rules_merged = 0usize;
    let mut rules_skipped = 0usize;
    let mut skills_merged = 0usize;
    let mut skills_skipped = 0usize;

    // Merge entries (LWW by last_accessed or created)
    for entry in body.memories.entries {
        match store.get(&entry.id) {
            Ok(local) => {
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = entry.last_accessed.unwrap_or(entry.created);
                if remote_time > local_time {
                    store.update(&entry)?;
                    entries_merged += 1;
                } else {
                    entries_skipped += 1;
                }
            }
            Err(_) => {
                store.add(&entry)?;
                entries_merged += 1;
            }
        }
    }

    // Merge rules (LWW by last_accessed or created)
    for rule in body.memories.rules {
        match rule_store.get(&rule.id) {
            Ok(local) => {
                let local_time = local.last_accessed.unwrap_or(local.created);
                let remote_time = rule.last_accessed.unwrap_or(rule.created);
                if remote_time > local_time {
                    rule_store.update(&rule)?;
                    rules_merged += 1;
                } else {
                    rules_skipped += 1;
                }
            }
            Err(_) => {
                rule_store.add(&rule)?;
                rules_merged += 1;
            }
        }
    }

    // Merge skills (LWW by updated_at)
    for skill in body.memories.skills {
        match skill_store.get(&skill.id) {
            Ok(local) => {
                if skill.updated_at > local.updated_at {
                    skill_store.update(&skill)?;
                    skills_merged += 1;
                } else {
                    skills_skipped += 1;
                }
            }
            Err(_) => {
                skill_store.add(&skill)?;
                skills_merged += 1;
            }
        }
    }

    // Save sync timestamp
    if let Some(pulled_at) = &body.pulled_at {
        config.set_team_memory_sync(&canonical_id, pulled_at);
        config.save()?;
    }

    if prev_lines > 0 {
        clear_inline(prev_lines)?;
    }

    if cli.json {
        println!(
            "{}",
            serde_json::json!({
                "status": "ok",
                "entries": { "merged": entries_merged, "skipped": entries_skipped },
                "rules": { "merged": rules_merged, "skipped": rules_skipped },
                "skills": { "merged": skills_merged, "skipped": skills_skipped },
                "contributors": contributor_count,
            })
        );
    } else {
        let mut out = io::stdout();
        let mut fmt = Formatter::stdout(&mut out, theme);
        fmt.success("Team memories synced")?;
        if entries_merged > 0 {
            fmt.write_raw(&format!("    {} entries merged", entries_merged))?;
            fmt.newline()?;
        }
        if rules_merged > 0 {
            fmt.write_raw(&format!("    {} rules merged", rules_merged))?;
            fmt.newline()?;
        }
        if skills_merged > 0 {
            fmt.write_raw(&format!("    {} skills merged", skills_merged))?;
            fmt.newline()?;
        }
        if entries_skipped + rules_skipped + skills_skipped > 0 {
            fmt.write_muted(&format!(
                "    {} skipped (local newer)",
                entries_skipped + rules_skipped + skills_skipped
            ))?;
            fmt.newline()?;
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════════
// PURGE-FOREIGN - Remove foreign-project entities and re-pull
// ═══════════════════════════════════════════════════════════════════════════════

fn execute_purge_foreign(
    args: &CloudPurgeForeignArgs,
    cli: &Cli,
    cas_root: &Path,
) -> anyhow::Result<()> {
    use std::sync::Arc;

    use crate::cloud::{CloudSyncer, CloudSyncerConfig, SyncQueue, get_project_canonical_id};

    let config = CloudConfig::load()?;
    if config.token.is_none() {
        anyhow::bail!("Not logged in. Run 'cas login' first");
    }

    let project_id = get_project_canonical_id()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine project ID. Not inside a CAS project?"))?;

    let store = open_store(cas_root)?;
    let task_store = open_task_store(cas_root)?;
    let rule_store = open_rule_store(cas_root)?;
    let skill_store = open_skill_store(cas_root)?;

    // Count entities before purge
    let entries_before = store.list().map(|v| v.len()).unwrap_or(0);
    let tasks_before = task_store.list(None).map(|v| v.len()).unwrap_or(0);
    let rules_before = rule_store.list().map(|v| v.len()).unwrap_or(0);
    let skills_before = skill_store.list(None).map(|v| v.len()).unwrap_or(0);
    let total_before = entries_before + tasks_before + rules_before + skills_before;

    if cli.json {
        if args.dry_run {
            println!(
                r#"{{"dry_run":true,"project_id":"{}","entities_before":{{"entries":{},"tasks":{},"rules":{},"skills":{},"total":{}}}}}"#,
                project_id, entries_before, tasks_before, rules_before, skills_before, total_before,
            );
            return Ok(());
        }
    } else {
        let theme = ActiveTheme::default();
        let mut out = io::stdout();
        let mut fmt = Formatter::stdout(&mut out, theme);
        fmt.newline()?;
        fmt.write_accent("  Purge Foreign Entities")?;
        fmt.newline()?;
        fmt.newline()?;
        fmt.write_muted("  Project: ")?;
        fmt.write_raw(&project_id)?;
        fmt.newline()?;
        fmt.write_muted("  Before:  ")?;
        fmt.write_raw(&format!(
            "{} entries, {} tasks, {} rules, {} skills ({} total)",
            entries_before, tasks_before, rules_before, skills_before, total_before,
        ))?;
        fmt.newline()?;

        if args.dry_run {
            fmt.newline()?;
            fmt.write_muted("  (dry run — no changes made)")?;
            fmt.newline()?;
            fmt.write_raw("  Run without --dry-run to purge and re-pull.")?;
            fmt.newline()?;
            return Ok(());
        }
    }

    // Step 1: Back up the database
    let db_path = cas_root.join("cas.db");
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = cas_root.join(format!("cas.db.pre-purge-{timestamp}"));
    if db_path.exists() {
        std::fs::copy(&db_path, &backup_path)?;
    }

    // Step 2: Delete all content entities via direct SQL
    // (Preserves: sync_queue, sync_metadata, agents, sessions, verifications,
    //  events, prompts, file_changes, commit_links, worktrees, dependencies, task_leases)
    {
        let conn = rusqlite::Connection::open(&db_path)?;
        conn.execute_batch(
            "DELETE FROM entries;
             DELETE FROM tasks;
             DELETE FROM dependencies;
             DELETE FROM rules;
             DELETE FROM skills;",
        )?;
        // Reset last_pull_at so re-pull fetches everything
        conn.execute(
            "DELETE FROM sync_metadata WHERE key = 'last_pull_at'",
            [],
        )?;
    }

    // Step 3: Re-pull from cloud with project-scoped filtering
    let queue = SyncQueue::open(cas_root)?;
    queue.init()?;
    let syncer = CloudSyncer::new(
        Arc::new(queue),
        config,
        CloudSyncerConfig::default(),
    );

    let pull_result = syncer.pull(
        store.as_ref(),
        task_store.as_ref(),
        rule_store.as_ref(),
        skill_store.as_ref(),
    )?;

    // Count entities after re-pull
    let entries_after = store.list().map(|v| v.len()).unwrap_or(0);
    let tasks_after = task_store.list(None).map(|v| v.len()).unwrap_or(0);
    let rules_after = rule_store.list().map(|v| v.len()).unwrap_or(0);
    let skills_after = skill_store.list(None).map(|v| v.len()).unwrap_or(0);
    let total_after = entries_after + tasks_after + rules_after + skills_after;

    let purged = total_before.saturating_sub(total_after);

    if cli.json {
        println!(
            r#"{{"project_id":"{}","backup":"{}","entities_before":{{"entries":{},"tasks":{},"rules":{},"skills":{},"total":{}}},"entities_after":{{"entries":{},"tasks":{},"rules":{},"skills":{},"total":{}}},"purged":{},"pull_errors":{}}}"#,
            project_id,
            backup_path.display(),
            entries_before, tasks_before, rules_before, skills_before, total_before,
            entries_after, tasks_after, rules_after, skills_after, total_after,
            purged,
            serde_json::to_string(&pull_result.errors).unwrap_or_default(),
        );
    } else {
        let theme = ActiveTheme::default();
        let mut out = io::stdout();
        let mut fmt = Formatter::stdout(&mut out, theme);
        fmt.write_muted("  After:   ")?;
        fmt.write_raw(&format!(
            "{} entries, {} tasks, {} rules, {} skills ({} total)",
            entries_after, tasks_after, rules_after, skills_after, total_after,
        ))?;
        fmt.newline()?;
        fmt.write_muted("  Purged:  ")?;
        fmt.write_raw(&format!("{} foreign entities removed", purged))?;
        fmt.newline()?;
        fmt.write_muted("  Backup:  ")?;
        fmt.write_raw(&backup_path.to_string_lossy())?;
        fmt.newline()?;

        if !pull_result.errors.is_empty() {
            fmt.newline()?;
            let warning_color = fmt.theme().palette.status_warning;
            fmt.write_colored("  \u{26A0} ", warning_color)?;
            fmt.write_raw(&format!("{} pull errors:", pull_result.errors.len()))?;
            fmt.newline()?;
            for err in &pull_result.errors {
                fmt.write_muted("    - ")?;
                fmt.write_raw(err)?;
                fmt.newline()?;
            }
        }

        fmt.newline()?;
        let success_color = fmt.theme().palette.status_success;
        fmt.write_colored("  \u{2713} ", success_color)?;
        fmt.write_raw("Purge complete. Pending local changes in sync queue are preserved.")?;
        fmt.newline()?;
    }

    Ok(())
}
