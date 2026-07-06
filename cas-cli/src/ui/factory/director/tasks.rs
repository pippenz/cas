//! Task list widget for the director panel

use std::collections::HashSet;

use cas_types::{Priority, TaskStatus};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};

use crate::ui::factory::director::data::{DirectorData, EpicGroup, TaskSummary};
use crate::ui::theme::{ActiveTheme, Icons, Palette, get_agent_color};

#[derive(Debug, Clone)]
pub(crate) struct ScopedTaskView {
    pub epic_groups: Vec<EpicGroup>,
    pub standalone: Vec<TaskSummary>,
}

impl ScopedTaskView {
    pub(crate) fn new(data: &DirectorData, focused_epic_id: Option<&str>) -> Self {
        let (epic_groups, standalone) = data.tasks_by_epic();
        let epic_groups = match focused_epic_id {
            Some(epic_id) => epic_groups
                .into_iter()
                .filter(|group| {
                    group.epic.id == epic_id
                        && group
                            .subtasks
                            .iter()
                            .any(|task| task_assigned_to_session_agent(task, data))
                })
                .collect(),
            None => Vec::new(),
        };

        let standalone = standalone
            .into_iter()
            .filter(|task| task_assigned_to_session_agent(task, data))
            .collect();

        Self {
            epic_groups,
            standalone,
        }
    }

    pub(crate) fn visible_row_count(
        &self,
        agent_filter: Option<&str>,
        collapsed_epics: &HashSet<String>,
    ) -> usize {
        let mut count = 0;

        for group in &self.epic_groups {
            let visible_subtasks = visible_subtask_count(group, agent_filter);
            if agent_filter.is_some() && visible_subtasks == 0 {
                continue;
            }

            count += 1;
            if !collapsed_epics.contains(&group.epic.id) {
                count += visible_subtasks;
            }
        }

        let standalone_count = filtered_standalone_count(&self.standalone, agent_filter);
        if count > 0 && standalone_count > 0 {
            count += 1;
        }
        count + standalone_count
    }
}

pub(crate) fn task_assigned_to_session_agent(task: &TaskSummary, data: &DirectorData) -> bool {
    task.assignee.as_ref().is_some_and(|assignee| {
        data.agent_id_to_name.contains_key(assignee)
            || data.agent_id_to_name.values().any(|name| name == assignee)
    })
}

pub(crate) fn epic_has_session_agent_subtask(data: &DirectorData, epic_id: &str) -> bool {
    data.in_progress_tasks
        .iter()
        .chain(data.ready_tasks.iter())
        .any(|task| {
            task.epic.as_deref() == Some(epic_id) && task_assigned_to_session_agent(task, data)
        })
}

fn task_matches_agent_filter(task: &TaskSummary, agent_filter: Option<&str>) -> bool {
    match agent_filter {
        None => true,
        Some(filter) => task.assignee.as_deref() == Some(filter),
    }
}

fn visible_subtask_count(group: &EpicGroup, agent_filter: Option<&str>) -> usize {
    group
        .subtasks
        .iter()
        .filter(|task| task_matches_agent_filter(task, agent_filter))
        .count()
}

fn filtered_standalone_count(standalone: &[TaskSummary], agent_filter: Option<&str>) -> usize {
    standalone
        .iter()
        .filter(|task| task_matches_agent_filter(task, agent_filter))
        .count()
}

/// Get color for priority level
fn priority_color(priority: Priority, palette: &Palette) -> Color {
    match priority.0 {
        0 => palette.priority_critical,
        1 => palette.priority_high,
        2 => palette.priority_medium,
        3 => palette.priority_low,
        _ => palette.priority_backlog,
    }
}

/// Render the tasks section with optional focus indicator, agent filter, and epic collapse
#[allow(clippy::too_many_arguments)]
pub fn render_with_focus(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused: bool,
    _selected: Option<usize>,
    agent_filter: Option<&str>,
    focused_epic_id: Option<&str>,
    collapsed: bool,
    collapsed_epics: &HashSet<String>,
    tasks_state: Option<&mut ListState>,
) {
    let palette = &theme.palette;
    let styles = &theme.styles;
    let scoped = ScopedTaskView::new(data, focused_epic_id);
    let task_count = scoped.visible_row_count(agent_filter, collapsed_epics);
    let border_style = if focused {
        styles.border_focused
    } else {
        styles.border_default
    };

    // If collapsed, just render the header
    if collapsed {
        super::panel::render_collapsed_header(
            frame,
            area,
            styles,
            super::panel::CollapsedHeader {
                title: "TASKS",
                count: task_count,
                hotkey: Some("t"),
                focused,
                icon_style: None,
            },
        );
        return;
    }

    // Filter standalone by agent if needed
    let filtered_standalone: Vec<_> = scoped
        .standalone
        .iter()
        .filter(|t| task_matches_agent_filter(t, agent_filter))
        .collect();

    // Build list items with epic grouping
    let mut items: Vec<ListItem> = Vec::new();

    for group in &scoped.epic_groups {
        // Filter subtasks by agent if needed
        let filtered_subtasks: Vec<_> = group
            .subtasks
            .iter()
            .filter(|t| task_matches_agent_filter(t, agent_filter))
            .collect();

        // Skip epic if no visible subtasks after filtering
        if agent_filter.is_some() && filtered_subtasks.is_empty() {
            continue;
        }

        let is_collapsed = collapsed_epics.contains(&group.epic.id);
        let active_indicator = if group.has_active {
            Icons::CIRCLE_FILLED
        } else {
            Icons::CIRCLE_EMPTY
        };
        let active_color = if group.has_active {
            palette.status_warning
        } else {
            palette.status_neutral
        };
        let subtask_count = filtered_subtasks.len();
        let collapse_icon = if is_collapsed {
            Icons::TRIANGLE_RIGHT
        } else {
            Icons::TRIANGLE_DOWN
        };

        // Overhead: active_indicator(2) + collapse_icon(2) + count(~5) + border(2) = ~11
        let epic_title = truncate(&group.epic.title, area.width.saturating_sub(12) as usize);
        items.push(ListItem::new(Line::from(vec![
            Span::styled(
                format!("{active_indicator} "),
                Style::default().fg(active_color),
            ),
            Span::styled(format!("{collapse_icon} "), styles.text_info),
            Span::styled(epic_title, styles.text_info.add_modifier(Modifier::BOLD)),
            Span::styled(format!(" ({subtask_count})"), styles.text_muted),
        ])));

        // Subtasks under this epic (only if not collapsed)
        if !is_collapsed {
            for task in filtered_subtasks {
                items.push(render_task_item(
                    task,
                    area.width,
                    &data.agent_id_to_name,
                    true,
                    palette,
                ));
            }
        }
    }

    // Standalone tasks
    if !filtered_standalone.is_empty() {
        // Add separator if we had epics
        if !scoped.epic_groups.is_empty() && !items.is_empty() {
            items.push(ListItem::new(Line::from(vec![Span::styled(
                "─ Standalone ",
                styles.text_muted,
            )])));
        }

        for task in filtered_standalone {
            items.push(render_task_item(
                task,
                area.width,
                &data.agent_id_to_name,
                false,
                palette,
            ));
        }
    }

    let title = format!(" {} TASKS ({}) [t] ", Icons::TRIANGLE_DOWN, task_count);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style);

    let list = if items.is_empty() {
        let empty_message = match agent_filter {
            Some(agent) => format!("No tasks for {agent}"),
            None => "No tasks".to_string(),
        };
        List::new(vec![ListItem::new(Line::from(vec![Span::styled(
            empty_message,
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))])
        .block(block)
    } else {
        List::new(items)
            .block(block)
            .highlight_style(styles.bg_selection)
    };

    if let Some(state) = tasks_state {
        frame.render_stateful_widget(list, area, state);
    } else {
        frame.render_widget(list, area);
    }
}

/// Render a single task item
fn render_task_item(
    task: &TaskSummary,
    width: u16,
    _agent_id_to_name: &std::collections::HashMap<String, String>,
    indented: bool,
    palette: &Palette,
) -> ListItem<'static> {
    // Task assignees store agent names directly (not IDs)
    let agent_name = task.assignee.clone();

    // Color by assignee agent's name
    let task_color = agent_name
        .as_ref()
        .map(|name| get_agent_color(name))
        .unwrap_or(palette.text_primary);

    let status_icon = match task.status {
        TaskStatus::InProgress => Icons::SPINNER_STATIC,
        TaskStatus::Open => Icons::CIRCLE_EMPTY,
        TaskStatus::Blocked => Icons::CIRCLE_X,
        TaskStatus::Closed => Icons::CHECK,
        // cas-b51a: awaiting supervisor code-review
        TaskStatus::PendingSupervisorReview => Icons::CLOCK,
    };

    let status_color = match task.status {
        TaskStatus::InProgress => palette.task_in_progress,
        TaskStatus::Blocked => palette.task_blocked,
        TaskStatus::Closed => palette.task_closed,
        TaskStatus::Open => palette.task_open,
        // cas-b51a: reuse warning color (same as blocked) — task is "waiting"
        TaskStatus::PendingSupervisorReview => palette.task_blocked,
    };

    // Priority indicator (P0, P1, etc.)
    let priority_str = format!("P{}", task.priority.0);
    let priority_col = priority_color(task.priority, palette);

    // Assignee badge — shown for all tasks with an assignee, not just in-progress
    let worker_badge = agent_name.map(|name| format!(" [{name}]"));

    let indent = if indented { "  " } else { "" };
    let indent_len = if indented { 2 } else { 0 };
    // Calculate overhead: indent + icon(2) + space + P#(2) + space + task_id + space + worker_badge + border(2)
    let worker_badge_len = worker_badge.as_ref().map(|b| b.len()).unwrap_or(0) as u16;
    let overhead = indent_len + 2 + 1 + 2 + 1 + task.id.len() as u16 + 1 + worker_badge_len + 2;
    let title_truncated = truncate(&task.title, width.saturating_sub(overhead) as usize);

    let mut spans = vec![
        Span::raw(indent.to_string()),
        Span::styled(status_icon.to_string(), Style::default().fg(status_color)),
        Span::raw(" "),
        Span::styled(priority_str, Style::default().fg(priority_col)),
        Span::raw(" "),
        Span::styled(task.id.clone(), Style::default().fg(task_color)),
        Span::raw(" "),
        Span::styled(title_truncated, Style::default().fg(task_color)),
    ];

    // Add assignee badge
    if let Some(badge) = worker_badge {
        spans.push(Span::styled(badge, Style::default().fg(palette.text_muted)));
    }

    ListItem::new(Line::from(spans))
}

/// Truncate text to max_len characters (UTF-8 safe)
fn truncate(text: &str, max_len: usize) -> String {
    let char_count = text.chars().count();
    if char_count <= max_len {
        text.to_string()
    } else if max_len <= 3 {
        "...".to_string()
    } else {
        let truncated: String = text.chars().take(max_len - 3).collect();
        format!("{truncated}...")
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use cas_factory::DirectorData;
    use cas_types::{Priority, TaskStatus, TaskType};

    use super::{ScopedTaskView, TaskSummary};

    fn task(
        id: &str,
        title: &str,
        task_type: TaskType,
        epic: Option<&str>,
        assignee: Option<&str>,
    ) -> TaskSummary {
        TaskSummary {
            id: id.to_string(),
            title: title.to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: assignee.map(str::to_string),
            task_type,
            epic: epic.map(str::to_string),
            branch: None,
            updated_at: None,
        }
    }

    fn data_for_scoping() -> DirectorData {
        DirectorData {
            ready_tasks: vec![
                task(
                    "cas-focused-1",
                    "Focused worker-name subtask",
                    TaskType::Task,
                    Some("cas-focused"),
                    Some("worker-one"),
                ),
                task(
                    "cas-focused-2",
                    "Focused worker-id subtask",
                    TaskType::Task,
                    Some("cas-focused"),
                    Some("agent-1"),
                ),
                task(
                    "cas-foreign-1",
                    "Foreign epic subtask",
                    TaskType::Task,
                    Some("cas-foreign"),
                    Some("other-worker"),
                ),
                task(
                    "cas-standalone-name",
                    "Session standalone by name",
                    TaskType::Task,
                    None,
                    Some("worker-one"),
                ),
                task(
                    "cas-standalone-id",
                    "Session standalone by id",
                    TaskType::Task,
                    None,
                    Some("agent-1"),
                ),
                task(
                    "cas-standalone-foreign",
                    "Foreign standalone",
                    TaskType::Task,
                    None,
                    Some("other-worker"),
                ),
            ],
            in_progress_tasks: Vec::new(),
            epic_tasks: vec![
                task("cas-focused", "Focused epic", TaskType::Epic, None, None),
                task("cas-foreign", "Foreign epic", TaskType::Epic, None, None),
            ],
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::from([("agent-1".to_string(), "worker-one".to_string())]),
            changes: Vec::new(),
            git_loaded: false,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
        }
    }

    #[test]
    fn scoped_task_view_only_keeps_focused_epic_and_session_standalone_tasks() {
        let data = data_for_scoping();
        let scoped = ScopedTaskView::new(&data, Some("cas-focused"));

        assert_eq!(scoped.epic_groups.len(), 1);
        assert_eq!(scoped.epic_groups[0].epic.id, "cas-focused");
        assert_eq!(scoped.epic_groups[0].subtasks.len(), 2);
        assert!(
            scoped
                .epic_groups
                .iter()
                .all(|group| group.epic.id != "cas-foreign")
        );

        let standalone_ids: Vec<_> = scoped
            .standalone
            .iter()
            .map(|task| task.id.as_str())
            .collect();
        assert_eq!(
            standalone_ids,
            vec!["cas-standalone-id", "cas-standalone-name"]
        );
    }

    #[test]
    fn scoped_task_view_has_no_foreign_epic_groups_when_unfocused() {
        let data = data_for_scoping();
        let scoped = ScopedTaskView::new(&data, None);

        assert!(scoped.epic_groups.is_empty());
        assert_eq!(scoped.standalone.len(), 2);
    }

    #[test]
    fn scoped_task_view_rejects_focus_without_session_agent_subtasks() {
        let data = data_for_scoping();
        let scoped = ScopedTaskView::new(&data, Some("cas-foreign"));

        assert!(scoped.epic_groups.is_empty());
        assert_eq!(scoped.standalone.len(), 2);
    }

    #[test]
    fn visible_row_count_tracks_agent_filter_and_epic_collapse() {
        let data = data_for_scoping();
        let scoped = ScopedTaskView::new(&data, Some("cas-focused"));

        assert_eq!(scoped.visible_row_count(None, &HashSet::new()), 6);
        assert_eq!(
            scoped.visible_row_count(None, &HashSet::from(["cas-focused".to_string()])),
            4
        );
        assert_eq!(
            scoped.visible_row_count(Some("worker-one"), &HashSet::new()),
            4
        );
    }
}
