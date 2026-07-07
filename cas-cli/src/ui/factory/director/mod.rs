//! Sidecar panels - displays CAS tasks, agents, changes, and activity
//!
//! This module provides native ratatui widgets for the factory TUI sidecar.
//! The panels are rendered directly without a containing block wrapper,
//! each section has its own header via the compact widget functions.

pub(crate) mod activity;
pub mod agent_helpers;
pub(crate) mod changes;
mod data;
mod events;
mod factory_radar;
pub mod mission_epic;
pub mod mission_workers;
pub mod panel;
mod prompts;
mod reminders;
pub(crate) mod tasks;

pub use data::{AgentSummary, DirectorData, DirectorStores, TaskSummary};
pub(crate) use events::pick_best_open_branch_epic;
pub use events::{DirectorEvent, DirectorEventDetector};
pub use panel::PanelRegistry;
pub use prompts::{Prompt, generate_prompt, with_response_instructions};
// PanelAreas, SidecarFocus, SidecarState, ViewMode, DiffLine, DiffLineType, render, render_with_state are already public in this module

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::ListState;

use crate::ui::theme::ActiveTheme;
use crate::ui::widgets::TreeItemType;

/// Which sidecar panel has focus
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum SidecarFocus {
    #[default]
    None,
    Factory,
    Tasks,
    Reminders,
    Changes,
    Activity,
}

/// View mode for the sidecar panel
#[derive(Debug, Clone, PartialEq, Default)]
pub enum ViewMode {
    /// Overview showing all panels
    #[default]
    Overview,
    /// Full task detail view
    TaskDetail(String),
    /// Full activity log view
    ActivityLog,
    /// File diff view (source_path, file_path)
    FileDiff(std::path::PathBuf, String),
}

/// Type of diff line for coloring
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DiffLineType {
    Context,
    Added,
    Removed,
    HunkHeader,
    FileHeader,
}

/// A processed diff line with line numbers
#[derive(Debug, Clone)]
pub struct DiffLine {
    pub old_line: Option<usize>,
    pub new_line: Option<usize>,
    pub content: String,
    pub line_type: DiffLineType,
}

impl SidecarFocus {
    /// Cycle to the next panel
    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Factory,
            Self::Factory => Self::Tasks,
            Self::Tasks => Self::Reminders,
            Self::Reminders => Self::Changes,
            Self::Changes => Self::Activity,
            Self::Activity => Self::None,
        }
    }

    /// Cycle to the previous panel
    pub fn prev(self) -> Self {
        match self {
            Self::None => Self::Activity,
            Self::Factory => Self::None,
            Self::Tasks => Self::Factory,
            Self::Reminders => Self::Tasks,
            Self::Changes => Self::Reminders,
            Self::Activity => Self::Changes,
        }
    }

    /// Cycle to the next panel, skipping Reminders if there are none
    pub fn next_with_reminders(self, has_reminders: bool) -> Self {
        let next = self.next();
        if next == Self::Reminders && !has_reminders {
            next.next()
        } else {
            next
        }
    }

    /// Cycle to the previous panel, skipping Reminders if there are none
    pub fn prev_with_reminders(self, has_reminders: bool) -> Self {
        let prev = self.prev();
        if prev == Self::Reminders && !has_reminders {
            prev.prev()
        } else {
            prev
        }
    }
}

/// Mutable state for sidecar rendering
pub struct SidecarState<'a> {
    pub focus: SidecarFocus,
    pub tasks_state: &'a mut ListState,
    pub agents_state: &'a mut ListState,
    pub reminders_state: &'a mut ListState,
    pub changes_state: &'a mut ListState,
    pub activity_state: &'a mut ListState,
    /// Optional agent filter (filter tasks/activity by this agent name)
    pub agent_filter: Option<&'a str>,
    /// Current display-focused epic ID.
    pub focused_epic_id: Option<&'a str>,
    /// Cached branch/ahead-behind status for the current display-focused epic.
    pub focused_epic_branch_status: Option<EpicBranchStatus<'a>>,
    /// Section collapse flags
    pub factory_collapsed: bool,
    pub tasks_collapsed: bool,
    pub reminders_collapsed: bool,
    pub changes_collapsed: bool,
    pub activity_collapsed: bool,
    /// Collapsed epic IDs
    pub collapsed_epics: &'a std::collections::HashSet<String>,
    /// Collapsed directory paths in changes panel
    pub collapsed_dirs: &'a std::collections::HashSet<String>,
    /// Output: tree item types from changes panel (for scroll bounds)
    pub changes_item_types: &'a mut Vec<TreeItemType>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EpicBranchStatus<'a> {
    pub branch: &'a str,
    pub ahead: u32,
    pub behind: u32,
}

/// Panel areas for click detection
#[derive(Debug, Clone, Copy, Default)]
#[allow(dead_code)]
pub struct PanelAreas {
    pub factory: Rect,
    pub tasks: Rect,
    pub reminders: Rect,
    pub changes: Rect,
    pub activity: Rect,
}

/// Render the sidecar panels with optional navigation state
///
/// Returns the panel areas for click detection.
pub fn render_with_state(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused_epic_id: Option<&str>,
    supervisor_name: &str,
    mut state: Option<&mut SidecarState>,
) -> PanelAreas {
    let factory_collapsed = state.as_ref().map(|s| s.factory_collapsed).unwrap_or(false);
    // Get collapse flags
    let tasks_collapsed = state.as_ref().map(|s| s.tasks_collapsed).unwrap_or(false);
    let reminders_collapsed = state
        .as_ref()
        .map(|s| s.reminders_collapsed)
        .unwrap_or(false);
    let changes_collapsed = state.as_ref().map(|s| s.changes_collapsed).unwrap_or(false);
    let activity_collapsed = state
        .as_ref()
        .map(|s| s.activity_collapsed)
        .unwrap_or(false);

    let has_reminders = !data.reminders.is_empty();

    let focus = state
        .as_ref()
        .map(|s| s.focus)
        .unwrap_or(SidecarFocus::None);
    tracing::debug!("render_with_state: focus={:?}, area={:?}", focus, area);

    // Calculate constraints based on collapse state (collapsed = 1 line header only)
    // Reminders panel is only included when there are active reminders
    let mut constraints: Vec<Constraint> = vec![
        if factory_collapsed {
            Constraint::Length(1)
        } else {
            Constraint::Percentage(if has_reminders { 25 } else { 28 })
        },
        if tasks_collapsed {
            Constraint::Length(1)
        } else {
            Constraint::Percentage(if has_reminders { 23 } else { 26 })
        },
    ];

    if has_reminders {
        constraints.push(if reminders_collapsed {
            Constraint::Length(1)
        } else {
            Constraint::Percentage(14)
        });
    }

    constraints.push(if changes_collapsed {
        Constraint::Length(1)
    } else {
        Constraint::Percentage(if has_reminders { 19 } else { 23 })
    });
    constraints.push(if activity_collapsed {
        Constraint::Length(1)
    } else {
        Constraint::Percentage(if has_reminders { 19 } else { 23 })
    });

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let agent_filter = state.as_ref().and_then(|s| s.agent_filter);
    let focused_epic_id = state
        .as_ref()
        .and_then(|s| s.focused_epic_id)
        .or(focused_epic_id);
    let focused_epic_branch_status = state.as_ref().and_then(|s| s.focused_epic_branch_status);

    // Get collapsed_epics from state (or empty set if no state)
    #[allow(clippy::incompatible_msrv)]
    static EMPTY_SET: std::sync::LazyLock<std::collections::HashSet<String>> =
        std::sync::LazyLock::new(std::collections::HashSet::new);
    let collapsed_epics = state
        .as_ref()
        .map(|s| s.collapsed_epics)
        .unwrap_or(&EMPTY_SET);

    // Track chunk indices (reminders panel shifts subsequent indices)
    let factory_idx = 0;
    let tasks_idx = 1;
    let reminders_idx = if has_reminders { Some(2) } else { None };
    let changes_idx = if has_reminders { 3 } else { 2 };
    let activity_idx = if has_reminders { 4 } else { 3 };

    // Render each section with focus indicator and collapse state
    factory_radar::render_with_focus(
        frame,
        chunks[factory_idx],
        data,
        theme,
        focused_epic_id,
        focused_epic_branch_status,
        focus == SidecarFocus::Factory,
        state.as_ref().and_then(|s| s.agents_state.selected()),
        supervisor_name,
        factory_collapsed,
    );
    tasks::render_with_focus(
        frame,
        chunks[tasks_idx],
        data,
        theme,
        focus == SidecarFocus::Tasks,
        state.as_ref().and_then(|s| s.tasks_state.selected()),
        agent_filter,
        focused_epic_id,
        tasks_collapsed,
        collapsed_epics,
        state.as_mut().map(|s| &mut *s.tasks_state),
    );

    // Render reminders panel (only when reminders exist)
    let reminders_area = if let Some(idx) = reminders_idx {
        reminders::render_with_focus(
            frame,
            chunks[idx],
            data,
            theme,
            focus == SidecarFocus::Reminders,
            reminders_collapsed,
            state.as_mut().map(|s| &mut *s.reminders_state),
        );
        chunks[idx]
    } else {
        Rect::default()
    };

    // Get collapsed_dirs from state (or empty set if no state)
    let collapsed_dirs = state
        .as_ref()
        .map(|s| s.collapsed_dirs)
        .unwrap_or(&EMPTY_SET);

    let item_types = changes::render_with_focus(
        frame,
        chunks[changes_idx],
        data,
        theme,
        focus == SidecarFocus::Changes,
        state.as_ref().and_then(|s| s.changes_state.selected()),
        changes_collapsed,
        state.as_mut().map(|s| &mut *s.changes_state),
        collapsed_dirs,
    );
    // Store item types for scroll bounds calculation
    if let Some(ref mut s) = state {
        *s.changes_item_types = item_types;
    }
    activity::render_with_focus(
        frame,
        chunks[activity_idx],
        data,
        theme,
        focus == SidecarFocus::Activity,
        state.as_ref().and_then(|s| s.activity_state.selected()),
        activity_collapsed,
    );

    // Return panel areas for click detection
    PanelAreas {
        factory: chunks[factory_idx],
        tasks: chunks[tasks_idx],
        reminders: reminders_area,
        changes: chunks[changes_idx],
        activity: chunks[activity_idx],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use cas_factory::{DirectorData, TaskSummary};
    use cas_types::{Priority, TaskStatus, TaskType};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::widgets::ListState;

    use crate::ui::theme::ActiveTheme;

    use super::{SidecarFocus, SidecarState, render_with_state};

    fn task(id: &str, task_type: TaskType, epic: Option<&str>) -> TaskSummary {
        TaskSummary {
            id: id.to_string(),
            title: id.to_string(),
            status: TaskStatus::Open,
            priority: Priority::MEDIUM,
            assignee: None,
            task_type,
            epic: epic.map(str::to_string),
            branch: Some(format!("epic/{id}")).filter(|_| task_type == TaskType::Epic),
            updated_at: None,
        }
    }

    fn data_with_two_epics() -> DirectorData {
        DirectorData {
            ready_tasks: vec![
                task("cas-state-child", TaskType::Task, Some("cas-state")),
                task("cas-param-child", TaskType::Task, Some("cas-param")),
            ],
            in_progress_tasks: Vec::new(),
            epic_tasks: vec![
                task("cas-state", TaskType::Epic, None),
                task("cas-param", TaskType::Epic, None),
            ],
            agents: Vec::new(),
            activity: Vec::new(),
            agent_id_to_name: HashMap::new(),
            changes: Vec::new(),
            git_loaded: false,
            reminders: Vec::new(),
            epic_closed_counts: HashMap::new(),
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
    fn render_with_state_prefers_state_focused_epic_id_over_parameter() {
        let data = data_with_two_epics();
        let backend = TestBackend::new(90, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ActiveTheme::default();
        let mut tasks_state = ListState::default();
        let mut agents_state = ListState::default();
        let mut reminders_state = ListState::default();
        let mut changes_state = ListState::default();
        let mut activity_state = ListState::default();
        let collapsed_epics = HashSet::new();
        let collapsed_dirs = HashSet::new();
        let mut changes_item_types = Vec::new();

        terminal
            .draw(|frame| {
                let mut state = SidecarState {
                    focus: SidecarFocus::Factory,
                    tasks_state: &mut tasks_state,
                    agents_state: &mut agents_state,
                    reminders_state: &mut reminders_state,
                    changes_state: &mut changes_state,
                    activity_state: &mut activity_state,
                    agent_filter: None,
                    focused_epic_id: Some("cas-state"),
                    focused_epic_branch_status: None,
                    factory_collapsed: false,
                    tasks_collapsed: false,
                    reminders_collapsed: false,
                    changes_collapsed: false,
                    activity_collapsed: false,
                    collapsed_epics: &collapsed_epics,
                    collapsed_dirs: &collapsed_dirs,
                    changes_item_types: &mut changes_item_types,
                };
                render_with_state(
                    frame,
                    frame.area(),
                    &data,
                    &theme,
                    Some("cas-param"),
                    "supervisor",
                    Some(&mut state),
                );
            })
            .unwrap();

        let text = buffer_text(&terminal);
        assert!(text.contains("EPIC: cas-state"));
        assert!(!text.contains("EPIC: cas-param"));
    }
}
