//! Mission Control: Workers Panel
//!
//! Full-width table showing two rows per agent: status/task line and detail line.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use super::DirectorData;
use super::agent_helpers::{self, find_agent_task, task_status_icon};
use crate::ui::theme::{ActiveTheme, get_agent_color};
use crate::ui::widgets::format_relative;

/// Render the WORKERS table panel with focus and selection support.
pub fn render_workers_panel_with_focus(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused: bool,
    selected: Option<usize>,
) {
    let palette = &theme.palette;
    let styles = &theme.styles;

    let agent_count = data.agents.len();

    let border_style = if focused {
        styles.border_focused
    } else {
        Style::default().fg(palette.border_muted)
    };
    let focus_marker = if focused { "\u{25b6}" } else { " " };
    let block = Block::default()
        .title(format!(" {focus_marker} WORKERS ({agent_count}) "))
        .title_style(
            Style::default()
                .fg(if focused {
                    palette.status_info
                } else {
                    palette.accent
                })
                .add_modifier(Modifier::BOLD),
        )
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    if inner.height == 0 || inner.width < 10 {
        frame.render_widget(block, area);
        return;
    }

    if data.agents.is_empty() {
        let line = Line::from(Span::styled(" No agents connected", styles.text_muted));
        frame.render_widget(Paragraph::new(vec![line]).block(block), area);
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(data.agents.len() * 2);

    for (agent_idx, agent) in data.agents.iter().enumerate() {
        let is_selected = selected == Some(agent_idx);
        let agent_color = get_agent_color(&agent.name);

        // Status icon and color
        let is_disconnected = agent_helpers::is_disconnected(agent);
        let (status_icon, icon_color) =
            agent_helpers::agent_status_icon(agent, palette, theme.is_minions());

        let selection_marker = if is_selected { "\u{25b8} " } else { "  " };
        let name_width = agent.name.len();
        let name_pad = 12usize.saturating_sub(name_width).max(1);
        let pad_str = " ".repeat(name_pad);

        // === LINE 1: status + name + task/idle info + elapsed ===
        let (task_spans, elapsed_span) = if is_disconnected {
            let elapsed = agent
                .last_heartbeat
                .map(format_relative)
                .unwrap_or_else(|| "unknown".to_string());
            (
                vec![Span::styled(
                    format!("disconnected (last seen {elapsed})"),
                    styles.text_error,
                )],
                None,
            )
        } else if let Some(ref task_id) = agent.current_task {
            let found_task = find_agent_task(agent, data);
            let task_title = found_task.map(|t| t.title.as_str()).unwrap_or("");
            let task_status_icon = found_task.map(|t| task_status_icon(t.status)).unwrap_or("");

            // Elapsed time from latest activity
            let elapsed = agent
                .latest_activity
                .as_ref()
                .map(|(_, ts)| format_relative(*ts));

            // Truncate task title to fit
            let prefix_len =
                2 + 2 + 1 + name_width + name_pad + task_status_icon.len() + 1 + task_id.len() + 1;
            let elapsed_len = elapsed.as_ref().map(|e| e.len() + 2).unwrap_or(0);
            let max_title = (inner.width as usize).saturating_sub(prefix_len + elapsed_len + 2);
            let title = truncate(task_title, max_title);

            let mut spans = Vec::new();
            if !task_status_icon.is_empty() {
                spans.push(Span::styled(
                    format!("{task_status_icon} "),
                    Style::default().fg(palette.task_in_progress),
                ));
            }
            spans.push(Span::styled(
                task_id.to_string(),
                Style::default().fg(palette.text_secondary),
            ));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(title, styles.text_primary));

            let elapsed_sp = elapsed.map(|e| Span::styled(format!("  {e}"), styles.text_muted));

            (spans, elapsed_sp)
        } else if let Some((ref desc, ref ts)) = agent.latest_activity {
            let elapsed = format_relative(*ts);
            (
                vec![Span::styled(
                    format!("idle {elapsed} \u{2502} last: {desc}"),
                    styles.text_muted,
                )],
                None,
            )
        } else {
            (vec![Span::styled("idle", styles.text_muted)], None)
        };

        // Build line 1
        let mut row1_spans = vec![
            Span::styled(
                selection_marker,
                if is_selected {
                    Style::default().fg(palette.status_info)
                } else {
                    Style::default()
                },
            ),
            Span::styled(status_icon, Style::default().fg(icon_color)),
            Span::raw(" "),
            Span::styled(
                agent.name.clone(),
                Style::default()
                    .fg(agent_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(pad_str.clone()),
        ];
        row1_spans.extend(task_spans);
        if let Some(elapsed) = elapsed_span {
            row1_spans.push(elapsed);
        }

        let mut row1 = Line::from(row1_spans);
        if is_selected {
            row1 = row1.style(Style::default().bg(palette.bg_elevated));
        }
        lines.push(row1);

        // === LINE 2: detail line (activity + file changes) ===
        let indent = " ".repeat(2 + 2 + 1 + name_width + name_pad); // align under task info
        let mut detail_spans: Vec<Span> = vec![Span::raw(indent)];

        // File change stats for this worker
        let worker_changes = data
            .changes
            .iter()
            .filter(|s| s.agent_name.as_deref() == Some(&agent.name));

        let mut total_added = 0usize;
        let mut total_removed = 0usize;
        let mut total_files = 0usize;
        for source in worker_changes {
            total_added += source.total_added;
            total_removed += source.total_removed;
            total_files += source.changes.len();
        }

        if !is_disconnected {
            // Latest activity description
            if let Some((ref desc, _)) = agent.latest_activity {
                let max_desc = if total_files > 0 {
                    (inner.width as usize)
                        .saturating_sub(2 + 2 + 1 + name_width + name_pad + 2 + 20)
                } else {
                    (inner.width as usize).saturating_sub(2 + 2 + 1 + name_width + name_pad + 2)
                };
                let desc_truncated = truncate(desc, max_desc);
                detail_spans.push(Span::styled(
                    format!("\u{258E} {desc_truncated}"),
                    Style::default().fg(palette.border_muted),
                ));
            }

            // File changes inline
            if total_files > 0 {
                detail_spans.push(Span::styled("  ", styles.text_muted));
                detail_spans.push(Span::styled(
                    format!("+{total_added}"),
                    Style::default().fg(palette.status_success),
                ));
                detail_spans.push(Span::styled(
                    format!(" -{total_removed}"),
                    Style::default().fg(palette.status_error),
                ));
                let file_label = if total_files == 1 { "file" } else { "files" };
                detail_spans.push(Span::styled(
                    format!(" ({total_files} {file_label})"),
                    styles.text_muted,
                ));
            }
        }

        let mut row2 = Line::from(detail_spans);
        if is_selected {
            row2 = row2.style(Style::default().bg(palette.bg_elevated));
        }
        lines.push(row2);
    }

    // Clamp to available height
    lines.truncate(inner.height as usize);

    frame.render_widget(Paragraph::new(lines).block(block), area);
}

/// Truncate text to max_len characters (UTF-8 safe)
fn truncate(text: &str, max_len: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_len {
        text.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        let truncated: String = text.chars().take(max_len - 1).collect();
        format!("{truncated}\u{2026}")
    }
}
