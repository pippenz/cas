//! Agent list widget for sidecar and factory TUI

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};

use cas_types::{AgentStatus, AgentType};
use chrono::{DateTime, Utc};

use crate::ui::theme::{ActiveTheme, Icons, get_agent_color};

use crate::ui::widgets::utils::{format_relative, truncate_to_width};

/// Display info for an agent
#[derive(Debug, Clone)]
pub struct AgentDisplayInfo {
    pub id: String,
    pub short_id: String,
    pub name: String,
    pub agent_type: AgentType,
    pub status: AgentStatus,
    /// Current task ID (if any)
    pub current_task_id: Option<String>,
    /// Current task title (if any)
    pub current_task_title: Option<String>,
    /// Latest activity (description, timestamp)
    pub latest_activity: Option<(String, DateTime<Utc>)>,
    /// Last heartbeat timestamp (for showing staleness)
    pub last_heartbeat: Option<DateTime<Utc>>,
}

/// Configuration for agent list rendering
#[derive(Debug, Default)]
pub struct AgentConfig {
    /// Whether to show the type badge (P/S/W/C)
    pub show_type_badge: bool,
    /// Whether to show current task under agent
    pub show_current_task: bool,
}

impl AgentConfig {
    pub fn new() -> Self {
        Self {
            show_type_badge: true,
            show_current_task: true,
        }
    }

    pub fn compact() -> Self {
        Self {
            show_type_badge: false,
            show_current_task: false,
        }
    }
}

/// Render a stateless agent list (for factory director)
pub fn render_agent_list(
    frame: &mut Frame,
    area: Rect,
    agents: &[AgentDisplayInfo],
    theme: &ActiveTheme,
    config: &AgentConfig,
    title: Option<&str>,
) {
    let palette = &theme.palette;
    let block = if let Some(title) = title {
        Block::default()
            .title(format!(" {} ({}) ", title, agents.len()))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(palette.border_muted))
    } else {
        Block::default()
    };

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = build_agent_items(agents, theme, config, inner.width);
    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render a stateful agent list (for sidecar with selection)
pub fn render_agent_list_with_state(
    frame: &mut Frame,
    area: Rect,
    agents: &[AgentDisplayInfo],
    theme: &ActiveTheme,
    config: &AgentConfig,
    state: &mut ListState,
    block: Block,
) {
    let styles = &theme.styles;
    let inner = block.inner(area);
    let items = build_agent_items(agents, theme, config, inner.width);
    let list = List::new(items)
        .block(block)
        .highlight_style(styles.bg_selection);
    frame.render_stateful_widget(list, area, state);
}

/// Build agent list items
fn build_agent_items(
    agents: &[AgentDisplayInfo],
    theme: &ActiveTheme,
    config: &AgentConfig,
    width: u16,
) -> Vec<ListItem<'static>> {
    let styles = &theme.styles;
    let items: Vec<ListItem> = agents
        .iter()
        .map(|agent| build_agent_item(agent, theme, config, width))
        .collect();

    if items.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No agents",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))]
    } else {
        items
    }
}

/// Build a single agent item (may be multiple lines if showing task)
fn build_agent_item(
    agent: &AgentDisplayInfo,
    theme: &ActiveTheme,
    config: &AgentConfig,
    width: u16,
) -> ListItem<'static> {
    let palette = &theme.palette;
    let agent_color = get_agent_color(&agent.name);

    let status_icon = match agent.status {
        AgentStatus::Active => Span::styled(
            Icons::CIRCLE_FILLED.to_string(),
            Style::default().fg(palette.agent_active),
        ),
        AgentStatus::Idle => Span::styled(
            Icons::CIRCLE_HALF.to_string(),
            Style::default().fg(palette.agent_idle),
        ),
        _ => Span::styled(
            Icons::CIRCLE_EMPTY.to_string(),
            Style::default().fg(palette.agent_dead),
        ),
    };

    let display_name = if agent.name.is_empty() {
        &agent.short_id
    } else {
        &agent.name
    };

    let mut lines = vec![];

    // First line: status icon + type badge + name
    let mut first_line = vec![Span::raw(" "), status_icon, Span::raw(" ")];

    if config.show_type_badge {
        let type_badge = match agent.agent_type {
            AgentType::Primary => "P",
            AgentType::SubAgent => "S",
            AgentType::Worker => "W",
            AgentType::CI => "C",
        };
        first_line.push(Span::styled(
            type_badge.to_string(),
            Style::default().fg(agent_color),
        ));
        first_line.push(Span::raw(" "));
    }

    first_line.push(Span::styled(
        display_name.to_string(),
        Style::default().fg(agent_color),
    ));

    lines.push(Line::from(first_line));

    // Second line: current task (if any and if showing)
    if config.show_current_task {
        if let (Some(task_id), Some(task_title)) =
            (&agent.current_task_id, &agent.current_task_title)
        {
            let title = truncate_to_width(task_title, width, 15 + task_id.len());
            lines.push(Line::from(vec![
                Span::raw("   └─ "),
                Span::styled(task_id.clone(), Style::default().fg(agent_color)),
                Span::raw(" "),
                Span::styled(title, Style::default().fg(agent_color)),
            ]));
        }
    }

    ListItem::new(lines)
}

/// Render compact agent list for factory director (simplified single-line version)
pub fn render_compact_agent_list(
    frame: &mut Frame,
    area: Rect,
    agents: &[AgentDisplayInfo],
    theme: &ActiveTheme,
    focused: bool,
) {
    let palette = &theme.palette;
    let styles = &theme.styles;
    let border_style = if focused {
        styles.border_focused
    } else {
        styles.border_default
    };

    let focus_marker = if focused { "▶" } else { " " };
    let block = Block::default()
        .title(format!(" {} AGENTS ({}) ", focus_marker, agents.len()))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = agents
        .iter()
        .map(|agent| {
            let agent_color = get_agent_color(&agent.name);

            // Compute heartbeat staleness for status icon coloring
            let heartbeat_secs = agent
                .last_heartbeat
                .map(|hb| chrono::Utc::now().signed_duration_since(hb).num_seconds())
                .unwrap_or(9999);

            // Status icon color: use agent color if healthy, yellow/red if stale
            let icon_color = if heartbeat_secs > 300 {
                palette.status_error // Very stale (>5 min)
            } else if heartbeat_secs > 60 {
                palette.status_warning // Stale (>1 min)
            } else {
                agent_color // Healthy
            };

            let status_icon = match agent.status {
                AgentStatus::Active => Span::styled(
                    Icons::CIRCLE_FILLED.to_string(),
                    Style::default().fg(icon_color),
                ),
                AgentStatus::Idle => Span::styled(
                    Icons::CIRCLE_HALF.to_string(),
                    Style::default().fg(icon_color),
                ),
                _ => Span::styled(
                    Icons::CIRCLE_EMPTY.to_string(),
                    Style::default().fg(palette.status_neutral),
                ),
            };

            let name = if agent.name.is_empty() {
                &agent.short_id
            } else {
                &agent.name
            };

            // Build line with optional activity info
            let mut spans = vec![
                status_icon,
                Span::raw(" "),
                Span::styled(name.to_string(), Style::default().fg(agent_color)),
            ];

            // Add latest activity if available, otherwise show heartbeat age
            if let Some((description, timestamp)) = &agent.latest_activity {
                let time_ago = format_relative(*timestamp);
                // Truncate description to fit in available width
                let max_desc_len = inner.width.saturating_sub(name.len() as u16 + 12) as usize;
                let desc = if description.len() > max_desc_len && max_desc_len > 3 {
                    let mut end = (max_desc_len - 3).min(description.len());
                    while end > 0 && !description.is_char_boundary(end) {
                        end -= 1;
                    }
                    format!("{}...", &description[..end])
                } else {
                    description.clone()
                };
                spans.push(Span::styled(format!("  {desc} "), styles.text_muted));
                spans.push(Span::styled(time_ago, styles.text_muted));
            } else if let Some(hb) = agent.last_heartbeat {
                // Show heartbeat age if no recent activity
                let time_ago = format_relative(hb);
                let hb_color = if heartbeat_secs > 300 {
                    palette.status_error
                } else if heartbeat_secs > 60 {
                    palette.status_warning
                } else {
                    palette.status_neutral
                };
                spans.push(Span::styled(
                    format!("  seen {time_ago}"),
                    Style::default().fg(hb_color),
                ));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    if items.is_empty() {
        let empty_list = List::new(vec![ListItem::new(Line::from(vec![Span::styled(
            "No agents",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))]);
        frame.render_widget(empty_list, inner);
    } else {
        let list = List::new(items);
        frame.render_widget(list, inner);
    }
}
