//! Factory dashboard panel - summary view of factory state
//!
//! Shows epic progress, worker status, and queue at a glance.

use cas_types::{EventType, TaskStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::ui::factory::director::agent_helpers;
use crate::ui::factory::director::data::DirectorData;
use crate::ui::theme::{ActiveTheme, Icons};
use crate::ui::widgets::truncate_to_width;

/// Render the factory dashboard with optional focus indicator
#[allow(clippy::too_many_arguments)]
pub fn render_with_focus(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused: bool,
    selected_agent: Option<usize>,
    supervisor_name: &str,
    collapsed: bool,
) {
    let styles = &theme.styles;

    let missing_supervisor = !supervisor_name.is_empty()
        && !data
            .agents
            .iter()
            .any(|agent| agent.name == supervisor_name);
    let agent_count = data.agents.len() + usize::from(missing_supervisor);

    // Collapsed view: single line header
    if collapsed {
        super::panel::render_collapsed_header(
            frame,
            area,
            styles,
            super::panel::CollapsedHeader {
                title: "FACTORY",
                count: agent_count,
                hotkey: Some("f"),
                focused,
                icon_style: None,
            },
        );
        return;
    }

    // Full view with border
    let border_style = if focused {
        styles.border_focused
    } else {
        styles.border_default
    };

    let focus_marker = if focused { "▶" } else { " " };
    let title = format!(" {focus_marker} FACTORY ({agent_count}) ");
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width < 10 || inner.height < 3 {
        return;
    }

    // Layout: epic progress, worker list, summary
    let has_epic = !data.epic_tasks.is_empty();
    let epic_height = if has_epic { 2 } else { 0 };
    let summary_height = 1;
    let worker_height = inner
        .height
        .saturating_sub(epic_height + summary_height + 1); // +1 for separator

    let constraints = if has_epic {
        vec![
            Constraint::Length(epic_height),
            Constraint::Length(1), // separator
            Constraint::Length(worker_height),
            Constraint::Length(summary_height),
        ]
    } else {
        vec![
            Constraint::Length(worker_height + 1), // no epic, more space for workers
            Constraint::Length(summary_height),
        ]
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut chunk_idx = 0;

    // Epic progress (if any)
    if has_epic {
        render_epic_progress(frame, chunks[chunk_idx], data, theme);
        chunk_idx += 1;

        // Separator
        let sep = Line::from(Span::styled(
            "─".repeat(inner.width as usize),
            styles.text_muted,
        ));
        frame.render_widget(Paragraph::new(sep), chunks[chunk_idx]);
        chunk_idx += 1;
    }

    // Worker list
    render_worker_list(
        frame,
        chunks[chunk_idx],
        data,
        theme,
        selected_agent,
        supervisor_name,
    );
    chunk_idx += 1;

    // Summary bar
    render_summary_bar(frame, chunks[chunk_idx], data, theme, supervisor_name);
}

/// Render epic status bar
fn render_epic_progress(frame: &mut Frame, area: Rect, data: &DirectorData, theme: &ActiveTheme) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let styles = &theme.styles;
    let palette = &theme.palette;

    // Find active epic (in_progress status)
    let active_epic = data
        .epic_tasks
        .iter()
        .find(|e| e.status == TaskStatus::InProgress)
        .or_else(|| data.epic_tasks.first());

    let Some(epic) = active_epic else {
        return;
    };

    // Count tasks for this epic
    let in_progress_count = data
        .in_progress_tasks
        .iter()
        .filter(|t| t.epic.as_ref() == Some(&epic.id))
        .count();

    let queued_count = data
        .ready_tasks
        .iter()
        .filter(|t| t.epic.as_ref() == Some(&epic.id))
        .count();

    let total_visible = in_progress_count + queued_count;

    // Line 1: Epic title with status
    let epic_label = format!("EPIC: {}", epic.id);
    let status_indicator = if epic.status == TaskStatus::InProgress {
        format!(" {}", Icons::TRIANGLE_RIGHT)
    } else {
        String::new()
    };

    let line1 = Line::from(vec![
        Span::styled(epic_label, styles.text_info.add_modifier(Modifier::BOLD)),
        Span::styled(
            status_indicator,
            Style::default().fg(palette.status_success),
        ),
    ]);

    // Line 2: Task counts with visual indicator
    // Show: "▓▓▓░░░░░  3 active, 5 queued"
    let bar_width = (area.width as usize).saturating_sub(22).max(4); // Space for counts
    let active_width = if total_visible > 0 {
        ((in_progress_count as f32 / total_visible as f32) * bar_width as f32).round() as usize
    } else {
        0
    };
    let queued_width = bar_width.saturating_sub(active_width);

    let bar = format!(
        "{}{}",
        Icons::PROGRESS_MEDIUM.repeat(active_width),
        Icons::PROGRESS_EMPTY.repeat(queued_width)
    );

    let counts = format!(" {in_progress_count} active, {queued_count} queued");

    let line2 = Line::from(vec![
        Span::styled(bar, Style::default().fg(palette.agent_active)),
        Span::styled(counts, styles.text_muted),
    ]);

    let lines = if area.height >= 2 {
        vec![line1, line2]
    } else {
        vec![line1]
    };

    frame.render_widget(Paragraph::new(lines), area);
}

/// Render worker list with current tasks
fn render_worker_list(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    selected: Option<usize>,
    supervisor_name: &str,
) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let styles = &theme.styles;
    let palette = &theme.palette;

    let mut lines: Vec<Line> = Vec::new();
    let max_lines = area.height as usize;

    for (idx, agent) in data.agents.iter().enumerate() {
        if lines.len() >= max_lines {
            break;
        }

        // Status indicator
        let (status_char, status_color) = agent_helpers::agent_status_icon_simple(agent, palette);

        let is_selected = selected == Some(idx);
        let name_style = if is_selected {
            styles.text_info.add_modifier(Modifier::BOLD)
        } else {
            styles.text_primary
        };

        // Build task info
        let task_info = if let Some(task_id) = &agent.current_task {
            if let Some(t) = agent_helpers::find_agent_task(agent, data) {
                format!("{}: {}", task_id, t.title)
            } else {
                task_id.clone()
            }
        } else if let Some((activity, _)) = &agent.latest_activity {
            activity.clone()
        } else {
            "idle".to_string()
        };

        // Calculate available width for task info
        // Format: "[●] name: task_info"
        let prefix_len = 4 + agent.name.len() + 2; // "[●] " + name + ": "
        let task_display = truncate_to_width(&task_info, area.width, prefix_len);

        let line = Line::from(vec![
            Span::raw("["),
            Span::styled(status_char, Style::default().fg(status_color)),
            Span::raw("] "),
            Span::styled(&agent.name, name_style),
            Span::styled(": ", styles.text_muted),
            Span::styled(
                task_display,
                if agent.current_task.is_some() {
                    styles.text_primary
                } else {
                    styles.text_muted
                },
            ),
        ]);

        lines.push(line);
    }

    // Show missing supervisor explicitly so factory state doesn't silently hide it.
    let supervisor_missing = !supervisor_name.is_empty()
        && !data
            .agents
            .iter()
            .any(|agent| agent.name == supervisor_name);
    if supervisor_missing && lines.len() < max_lines {
        let line = Line::from(vec![
            Span::raw("["),
            Span::styled("⊘", Style::default().fg(palette.agent_dead)),
            Span::raw("] "),
            Span::styled(
                supervisor_name,
                styles.text_info.add_modifier(Modifier::BOLD),
            ),
            Span::styled(": ", styles.text_muted),
            Span::styled("not registered", styles.text_muted),
        ]);
        lines.push(line);
    }

    // Fill remaining space with empty lines or "no workers" message
    if lines.is_empty() {
        lines.push(Line::from(Span::styled(
            "No agents registered yet",
            styles.text_muted,
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Render summary bar with counts
fn render_summary_bar(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    supervisor_name: &str,
) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let styles = &theme.styles;
    let palette = &theme.palette;

    // Count agent statuses
    let status_counts = agent_helpers::count_agent_statuses(&data.agents);
    let active = status_counts.active;
    let idle = status_counts.idle;
    let mut dead = status_counts.dead;

    // Include missing supervisor as dead
    if !supervisor_name.is_empty()
        && !data
            .agents
            .iter()
            .any(|agent| agent.name == supervisor_name)
    {
        dead += 1;
    }

    let queue_count = data
        .ready_tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Open)
        .count();
    let blocked_count = data
        .in_progress_tasks
        .iter()
        .chain(data.ready_tasks.iter())
        .filter(|t| t.status == TaskStatus::Blocked)
        .count();
    let verification_debt = data
        .activity
        .iter()
        .filter(|e| e.event_type == EventType::WorkerVerificationBlocked)
        .map(|e| e.entity_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .len();

    let mut spans = vec![
        Span::styled("Active ", styles.text_muted),
        Span::styled(
            active.to_string(),
            Style::default().fg(palette.agent_active),
        ),
        Span::raw(" │ "),
        Span::styled("Idle ", styles.text_muted),
        Span::styled(idle.to_string(), Style::default().fg(palette.agent_idle)),
        Span::raw(" │ "),
        Span::styled("Queue ", styles.text_muted),
        Span::styled(
            queue_count.to_string(),
            Style::default().fg(palette.status_info),
        ),
    ];

    // Show blocked count if any
    if blocked_count > 0 {
        spans.extend([
            Span::raw(" │ "),
            Span::styled("Blocked ", styles.text_muted),
            Span::styled(
                blocked_count.to_string(),
                Style::default().fg(palette.status_warning),
            ),
        ]);
    }

    if verification_debt > 0 {
        spans.extend([
            Span::raw(" │ "),
            Span::styled("VerifDebt ", styles.text_muted),
            Span::styled(
                verification_debt.to_string(),
                Style::default().fg(palette.status_warning),
            ),
        ]);
    }

    // Show errors if any
    if dead > 0 {
        spans.extend([
            Span::raw(" │ "),
            Span::styled("Errors ", styles.text_muted),
            Span::styled(dead.to_string(), Style::default().fg(palette.status_error)),
        ]);
    }

    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line), area);
}
