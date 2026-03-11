//! Queue CLI commands for prompt queue operations.
//!
//! Used by native extensions to poll for messages and acknowledge delivery.

use std::io;

use clap::{Args, Subcommand};

use crate::store::{find_cas_root, open_prompt_queue_store};
use crate::ui::components::{Formatter, Renderable, StatusLine};
use crate::ui::theme::ActiveTheme;

#[derive(Subcommand)]
pub enum QueueCommands {
    /// Poll for pending messages targeted at an agent (marks as processed)
    Poll(QueuePollArgs),

    /// Acknowledge a message by ID
    Ack(QueueAckArgs),
}

#[derive(Args)]
pub struct QueuePollArgs {
    /// Target agent name to poll messages for
    #[arg(long)]
    pub target: String,

    /// Output format (json or text)
    #[arg(long, default_value = "text")]
    pub format: String,

    /// Maximum number of messages to return
    #[arg(long, default_value = "10")]
    pub limit: usize,
}

#[derive(Args)]
pub struct QueueAckArgs {
    /// Message ID to acknowledge
    #[arg(long)]
    pub id: i64,
}

pub fn execute(cmd: &QueueCommands, _cli: &super::Cli) -> anyhow::Result<()> {
    let cas_root = find_cas_root()?;
    let queue = open_prompt_queue_store(&cas_root)?;

    match cmd {
        QueueCommands::Poll(args) => {
            let messages = queue.poll_for_target(&args.target, args.limit)?;

            if args.format == "json" {
                let json: Vec<serde_json::Value> = messages
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "id": m.id,
                            "source": m.source,
                            "target": m.target,
                            "prompt": m.prompt,
                            "createdAt": m.created_at.to_rfc3339(),
                        })
                    })
                    .collect();
                println!("{}", serde_json::to_string(&json)?);
            } else {
                let theme = ActiveTheme::default();
                let mut stdout = io::stdout();
                let mut fmt = Formatter::stdout(&mut stdout, theme);

                if messages.is_empty() {
                    StatusLine::info("No pending messages").render(&mut fmt)?;
                } else {
                    for m in &messages {
                        fmt.field(
                            &format!("[{}]", m.id),
                            &format!(
                                "from={} target={} created={}",
                                m.source,
                                m.target,
                                m.created_at.format("%H:%M:%S")
                            ),
                        )?;
                        fmt.write_muted(&format!(
                            "  {}",
                            m.prompt.chars().take(200).collect::<String>()
                        ))?;
                        fmt.newline()?;
                        fmt.newline()?;
                    }
                }
            }
        }
        QueueCommands::Ack(args) => {
            queue.mark_processed(args.id)?;

            let theme = ActiveTheme::default();
            let mut stdout = io::stdout();
            let mut fmt = Formatter::stdout(&mut stdout, theme);
            StatusLine::success(format!("Message {} acknowledged", args.id)).render(&mut fmt)?;
        }
    }

    Ok(())
}
