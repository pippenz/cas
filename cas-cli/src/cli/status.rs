//! Status command

use std::path::Path;

use clap::Parser;

use crate::config::Config;
use crate::store::{open_code_store, open_rule_store, open_store};
use crate::ui::components::Formatter;
use crate::ui::theme::ActiveTheme;
use cas_core::Syncer;

use crate::cli::Cli;

#[derive(Parser)]
pub struct StatusArgs {
    /// Verbose output
    #[arg(short, long)]
    pub verbose: bool,
}

pub fn execute(args: &StatusArgs, cli: &Cli, cas_root: &Path) -> anyhow::Result<()> {
    let store = open_store(cas_root)?;
    let rule_store = open_rule_store(cas_root)?;
    let config = Config::load(cas_root)?;

    let entries = store.list()?;
    let archived = store.list_archived()?;
    let rules = rule_store.list()?;

    // Get code stats (optional - may not have indexed code)
    let (code_files, code_symbols) = if let Ok(code_store) = open_code_store(cas_root) {
        let symbols = code_store
            .search_symbols("%", None, None, 100000)
            .unwrap_or_default();
        let files = code_store.count_files().unwrap_or(0);
        (files, symbols.len())
    } else {
        (0, 0)
    };

    let total_entries = entries.len();
    let total_archived = archived.len();
    let total_rules = rules.len();

    // Count high-value entries
    let high_value = entries.iter().filter(|e| e.feedback_score() > 0).count();

    // Count proven rules
    let project_root = cas_root.parent().unwrap_or(std::path::Path::new("."));
    let syncer = Syncer::new(
        project_root.join(&config.sync.target),
        config.sync.min_helpful,
    );
    let proven_rules = rules.iter().filter(|r| syncer.is_proven(r)).count();

    if cli.json {
        let status = serde_json::json!({
            "entries": total_entries,
            "archived": total_archived,
            "high_value": high_value,
            "rules": total_rules,
            "proven_rules": proven_rules,
            "code_files": code_files,
            "code_symbols": code_symbols,
            "sync_enabled": config.sync.enabled && !Config::is_sync_disabled()
        });
        println!("{}", serde_json::to_string(&status)?);
    } else if args.verbose || cli.verbose {
        let theme = ActiveTheme::default();
        let mut out = std::io::stdout();
        let mut fmt = Formatter::stdout(&mut out, theme);

        fmt.subheading("cas status")?;
        fmt.write_muted(&"─".repeat(40))?;
        fmt.newline()?;
        fmt.field(
            "  Entries",
            &format!("{total_entries} ({total_archived} archived)"),
        )?;
        fmt.field("  High-value", &format!("{high_value} (positive feedback)"))?;
        fmt.field("  Rules", &format!("{total_rules} ({proven_rules} proven)"))?;
        if code_files > 0 || code_symbols > 0 {
            fmt.field(
                "  Code",
                &format!("{code_files} files, {code_symbols} symbols"),
            )?;
        }
        fmt.newline()?;
        fmt.subheading("Configuration")?;
        fmt.write_muted(&"─".repeat(40))?;
        fmt.newline()?;
        fmt.field("  Sync enabled", &config.sync.enabled.to_string())?;
        fmt.field("  Sync target", &config.sync.target)?;
        fmt.field("  Min helpful", &config.sync.min_helpful.to_string())?;

        if Config::is_sync_disabled() {
            fmt.newline()?;
            fmt.warning("Sync disabled via environment")?;
        }

        // Show recent entries
        if !entries.is_empty() {
            fmt.newline()?;
            fmt.subheading("Recent entries")?;
            fmt.write_muted(&"─".repeat(40))?;
            fmt.newline()?;
            for entry in entries.iter().take(5) {
                let score = entry.feedback_score();
                let score_str = if score > 0 {
                    format!("+{score}")
                } else {
                    score.to_string()
                };
                let line = format!("{} [{}] {}", entry.id, score_str, entry.preview(40));
                if score > 0 {
                    let color = fmt.theme().palette.status_success;
                    fmt.write_colored(&format!("  {line}"), color)?;
                } else if score < 0 {
                    let color = fmt.theme().palette.status_error;
                    fmt.write_colored(&format!("  {line}"), color)?;
                } else {
                    fmt.write_raw(&format!("  {line}"))?;
                }
                fmt.newline()?;
            }
        }
    } else {
        // One-line summary
        let code_part = if code_symbols > 0 {
            format!(", {code_symbols} code symbols")
        } else {
            String::new()
        };
        println!(
            "cas: {total_entries} entries, {total_rules} rules ({proven_rules} proven), {high_value} high-value{code_part}"
        );
    }

    Ok(())
}
