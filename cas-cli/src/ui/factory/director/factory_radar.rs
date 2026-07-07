//! Factory dashboard panel - summary view of factory state
//!
//! Shows epic progress, worker status, and queue at a glance.

use cas_types::{EventType, TaskStatus};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};

use crate::ui::factory::director::EpicBranchStatus;
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
    focused_epic_id: Option<&str>,
    focused_epic_branch_status: Option<EpicBranchStatus<'_>>,
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

    // Layout: epic progress/placeholder, worker list, summary
    let epic_height = if focused_epic_branch_status.is_some() {
        3
    } else {
        2
    };
    let summary_height = 1;
    let worker_height = inner
        .height
        .saturating_sub(epic_height + summary_height + 1); // +1 for separator

    let constraints = vec![
        Constraint::Length(epic_height),
        Constraint::Length(1), // separator
        Constraint::Length(worker_height),
        Constraint::Length(summary_height),
    ];

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut chunk_idx = 0;

    // Epic progress or explicit unfocused placeholder
    render_epic_progress(
        frame,
        chunks[chunk_idx],
        data,
        theme,
        focused_epic_id,
        focused_epic_branch_status,
    );
    chunk_idx += 1;

    // Separator
    let sep = Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        styles.text_muted,
    ));
    frame.render_widget(Paragraph::new(sep), chunks[chunk_idx]);
    chunk_idx += 1;

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
fn render_epic_progress(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused_epic_id: Option<&str>,
    focused_epic_branch_status: Option<EpicBranchStatus<'_>>,
) {
    if area.height == 0 || area.width < 10 {
        return;
    }

    let styles = &theme.styles;
    let palette = &theme.palette;

    let Some(focused_epic_id) = focused_epic_id else {
        render_unfocused_epic_placeholder(frame, area, theme);
        return;
    };

    if !focused_epic_is_renderable_source_blind(data, focused_epic_id) {
        render_unfocused_epic_placeholder(frame, area, theme);
        return;
    }

    let Some(epic) = data.epic_tasks.iter().find(|e| e.id == focused_epic_id) else {
        render_unfocused_epic_placeholder(frame, area, theme);
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

    let branch_line = focused_epic_branch_status.map(|status| {
        Line::from(Span::styled(
            format_branch_status(status, area.width),
            styles.text_muted,
        ))
    });

    // Task counts with visual indicator
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

    let progress_line = Line::from(vec![
        Span::styled(bar, Style::default().fg(palette.agent_active)),
        Span::styled(counts, styles.text_muted),
    ]);

    let lines = if area.height >= 3 {
        if let Some(branch_line) = branch_line {
            vec![line1, branch_line, progress_line]
        } else {
            vec![line1, progress_line]
        }
    } else if area.height >= 2 {
        vec![line1, progress_line]
    } else {
        vec![line1]
    };

    frame.render_widget(Paragraph::new(lines), area);
}

fn format_branch_status(status: EpicBranchStatus<'_>, width: u16) -> String {
    let suffix = format!(" ↑{} ↓{}", status.ahead, status.behind);
    let prefix = "BRANCH: ";
    let branch_budget = (width as usize)
        .saturating_sub(prefix.len() + suffix.chars().count())
        .max(4);
    let branch = truncate_to_width(status.branch, branch_budget as u16, 0);
    format!("{prefix}{branch}{suffix}")
}

fn render_unfocused_epic_placeholder(frame: &mut Frame, area: Rect, theme: &ActiveTheme) {
    let styles = &theme.styles;
    let line = Line::from(Span::styled(
        "No focused epic - supervisor: coordination action=focus_epic id=<epic>",
        styles.text_muted,
    ));
    frame.render_widget(Paragraph::new(line), area);
}

fn focused_epic_is_renderable_source_blind(data: &DirectorData, epic_id: &str) -> bool {
    data.in_progress_tasks
        .iter()
        .chain(data.ready_tasks.iter())
        .filter(|task| task.epic.as_deref() == Some(epic_id))
        .all(|task| {
            task.assignee.is_none()
                || crate::ui::factory::director::tasks::task_assigned_to_session_agent(task, data)
        })
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
        let (status_char, status_color) =
            agent_helpers::agent_status_icon_simple(agent, palette, theme.is_minions());

        let is_selected = selected == Some(idx);
        let name_style = if is_selected {
            styles.text_info.add_modifier(Modifier::BOLD)
        } else {
            styles.text_primary
        };

        // Build task info
        let current_task = agent_helpers::find_agent_in_progress_task(agent, data);
        let task_info = if let Some(task) = current_task {
            format!("▸ {} {}", task.id, task.title)
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
                if current_task.is_some() {
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cas_factory::{AgentSummary, DirectorData, TaskSummary};
    use cas_types::{AgentStatus, Priority, TaskStatus, TaskType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    use crate::ui::theme::ActiveTheme;

    use super::render_with_focus;

    fn task(id: &str, title: &str, status: TaskStatus, task_type: TaskType) -> TaskSummary {
        TaskSummary {
            id: id.to_string(),
            title: title.to_string(),
            status,
            priority: Priority::MEDIUM,
            assignee: None,
            task_type,
            epic: None,
            branch: None,
            updated_at: None,
        }
    }

    fn data_with_unrelated_epic() -> DirectorData {
        DirectorData {
            ready_tasks: vec![TaskSummary {
                id: "cas-foreign-child".to_string(),
                title: "Foreign child".to_string(),
                status: TaskStatus::Open,
                priority: Priority::MEDIUM,
                assignee: Some("other-agent".to_string()),
                task_type: TaskType::Task,
                epic: Some("cas-foreign".to_string()),
                branch: None,
                updated_at: None,
            }],
            in_progress_tasks: Vec::new(),
            epic_tasks: vec![task(
                "cas-foreign",
                "Foreign in-progress epic",
                TaskStatus::InProgress,
                TaskType::Epic,
            )],
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::from([(
                "session-agent".to_string(),
                "worker-one".to_string(),
            )]),
            changes: Vec::new(),
            git_loaded: false,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
        }
    }

    fn agent(id: &str, name: &str) -> AgentSummary {
        AgentSummary {
            id: id.to_string(),
            name: name.to_string(),
            status: AgentStatus::Active,
            current_task: None,
            latest_activity: None,
            last_heartbeat: Some(chrono::Utc::now()),
            pending_messages: 0,
        }
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn factory_radar_renders_unfocused_placeholder_instead_of_foreign_epic() {
        let data = data_with_unrelated_epic();
        let backend = TestBackend::new(90, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();

        terminal
            .draw(|frame| {
                render_with_focus(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-foreign"),
                    None,
                    false,
                    None,
                    "supervisor",
                    false,
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("No focused epic"));
        assert!(text.contains("coordination action=focus_epic id=<epic>"));
        assert!(!text.contains("EPIC: cas-foreign"));
    }

    #[test]
    fn factory_radar_renders_session_owned_focused_epic_source_blind() {
        let mut data = data_with_unrelated_epic();
        data.ready_tasks[0].assignee = Some("session-agent".to_string());
        let backend = TestBackend::new(90, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();

        terminal
            .draw(|frame| {
                render_with_focus(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-foreign"),
                    None,
                    false,
                    None,
                    "supervisor",
                    false,
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("EPIC: cas-foreign"));
        assert!(!text.contains("No focused epic"));
    }

    #[test]
    fn factory_radar_renders_unassigned_focused_epic_source_blind() {
        let mut data = data_with_unrelated_epic();
        data.ready_tasks[0].assignee = None;
        let backend = TestBackend::new(90, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();

        terminal
            .draw(|frame| {
                render_with_focus(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-foreign"),
                    None,
                    false,
                    None,
                    "supervisor",
                    false,
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("EPIC: cas-foreign"));
        assert!(!text.contains("No focused epic"));
    }

    #[test]
    fn factory_radar_renders_epic_branch_ahead_behind() {
        let mut data = data_with_unrelated_epic();
        data.ready_tasks[0].assignee = Some("session-agent".to_string());
        let backend = TestBackend::new(110, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();

        terminal
            .draw(|frame| {
                render_with_focus(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-foreign"),
                    Some(crate::ui::factory::director::EpicBranchStatus {
                        branch: "epic/epic-factory-tui-visual-information-overhaul-osc-8-cas-ebc1",
                        ahead: 3,
                        behind: 1,
                    }),
                    false,
                    None,
                    "supervisor",
                    false,
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("EPIC: cas-foreign"));
        assert!(text.contains("BRANCH: epic/epic-factory-tui-visual-information-overhaul"));
        assert!(text.contains("↑3 ↓1"));
    }

    #[test]
    fn factory_radar_worker_rows_show_task_chips_for_id_and_display_name_assignees() {
        let mut data = data_with_unrelated_epic();
        data.agents = vec![
            agent("agent-id-1", "worker-one"),
            agent("agent-id-2", "worker-two"),
            agent("agent-id-3", "worker-three"),
        ];
        data.in_progress_tasks = vec![
            TaskSummary {
                id: "cas-id1".to_string(),
                title: "Assigned by agent id".to_string(),
                status: TaskStatus::InProgress,
                priority: Priority::MEDIUM,
                assignee: Some("agent-id-1".to_string()),
                task_type: TaskType::Task,
                epic: Some("cas-foreign".to_string()),
                branch: None,
                updated_at: None,
            },
            TaskSummary {
                id: "cas-name2".to_string(),
                title: "Assigned by display name".to_string(),
                status: TaskStatus::InProgress,
                priority: Priority::MEDIUM,
                assignee: Some("worker-two".to_string()),
                task_type: TaskType::Task,
                epic: Some("cas-foreign".to_string()),
                branch: None,
                updated_at: None,
            },
        ];
        let backend = TestBackend::new(120, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();

        terminal
            .draw(|frame| {
                render_with_focus(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-foreign"),
                    None,
                    false,
                    None,
                    "supervisor",
                    false,
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("worker-one: ▸ cas-id1 Assigned by agent id"));
        assert!(text.contains("worker-two: ▸ cas-name2 Assigned by display name"));
        assert!(text.contains("worker-three: idle"));
    }
}
