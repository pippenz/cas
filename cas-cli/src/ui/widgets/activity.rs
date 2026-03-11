//! Activity log widget for sidecar and factory TUI

use std::collections::{HashMap, HashSet};

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState};

use cas_types::Event;

use crate::ui::theme::{ActiveTheme, get_agent_color};

use crate::ui::widgets::utils::{event_type_color, format_relative, truncate};

/// Configuration for activity list rendering
#[derive(Debug, Default)]
pub struct ActivityConfig {
    /// Map from agent ID to agent name (for color lookup)
    pub agent_id_to_name: HashMap<String, String>,
    /// IDs of currently active agents
    pub active_agent_ids: HashSet<String>,
    /// Maximum number of events to show
    pub max_events: usize,
}

impl ActivityConfig {
    pub fn new() -> Self {
        Self {
            max_events: 20,
            ..Default::default()
        }
    }

    pub fn with_agent_maps(
        agent_id_to_name: HashMap<String, String>,
        active_agent_ids: HashSet<String>,
    ) -> Self {
        Self {
            agent_id_to_name,
            active_agent_ids,
            max_events: 20,
        }
    }
}

/// Render a stateless activity list (for factory director)
pub fn render_activity_list(
    frame: &mut Frame,
    area: Rect,
    events: &[Event],
    theme: &ActiveTheme,
    config: &ActivityConfig,
    title: Option<&str>,
) {
    let palette = &theme.palette;
    let block = if let Some(title) = title {
        Block::default()
            .title(format!(" {} ({}) ", title, events.len()))
            .borders(Borders::TOP)
            .border_style(Style::default().fg(palette.border_muted))
    } else {
        Block::default()
    };

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = build_activity_items(events, theme, config, inner.width, inner.height as usize);
    let list = List::new(items);
    frame.render_widget(list, inner);
}

/// Render a stateful activity list (for sidecar with selection)
pub fn render_activity_list_with_state(
    frame: &mut Frame,
    area: Rect,
    events: &[Event],
    theme: &ActiveTheme,
    config: &ActivityConfig,
    state: &mut ListState,
    block: Block,
) {
    let styles = &theme.styles;
    let inner = block.inner(area);
    let items = build_activity_items(events, theme, config, inner.width, config.max_events);
    let list = List::new(items)
        .block(block)
        .highlight_style(styles.bg_selection);
    frame.render_stateful_widget(list, area, state);
}

/// Build activity list items
fn build_activity_items(
    events: &[Event],
    theme: &ActiveTheme,
    config: &ActivityConfig,
    width: u16,
    max_items: usize,
) -> Vec<ListItem<'static>> {
    let palette = &theme.palette;
    let styles = &theme.styles;
    let items: Vec<ListItem> = events
        .iter()
        .take(max_items)
        .map(|event| {
            let time = format_relative(event.created_at);

            // Color by agent if present and active, otherwise by event type
            let event_color = event
                .session_id
                .as_ref()
                .filter(|id| config.active_agent_ids.contains(*id))
                .and_then(|id| config.agent_id_to_name.get(id))
                .map(|name| get_agent_color(name))
                .unwrap_or_else(|| event_type_color(&event.event_type, palette));

            let summary = truncate(&event.summary, width.saturating_sub(8) as usize);

            ListItem::new(Line::from(vec![
                Span::styled(format!("{time:>4} "), styles.text_muted),
                Span::styled(
                    format!("{} ", event.icon()),
                    Style::default().fg(event_color),
                ),
                Span::styled(summary, Style::default().fg(event_color)),
            ]))
        })
        .collect();

    if items.is_empty() {
        vec![ListItem::new(Line::from(vec![Span::styled(
            "No activity",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))]
    } else {
        items
    }
}

/// Render compact activity list for factory director
pub fn render_compact_activity_list(
    frame: &mut Frame,
    area: Rect,
    events: &[Event],
    agent_id_to_name: &HashMap<String, String>,
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
        .title(format!(" {} ACTIVITY ({}) ", focus_marker, events.len()))
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(border_style);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items: Vec<ListItem> = events
        .iter()
        .take(inner.height as usize)
        .map(|event| {
            let time = format_relative(event.created_at);

            // Color by agent if present
            let event_color = event
                .session_id
                .as_ref()
                .and_then(|id| agent_id_to_name.get(id))
                .map(|name| get_agent_color(name))
                .unwrap_or_else(|| event_type_color(&event.event_type, palette));

            let summary = truncate(&event.summary, inner.width.saturating_sub(8) as usize);

            ListItem::new(Line::from(vec![
                Span::styled(format!("{time:>4} "), styles.text_muted),
                Span::styled(
                    format!("{} ", event.icon()),
                    Style::default().fg(event_color),
                ),
                Span::styled(summary, Style::default().fg(event_color)),
            ]))
        })
        .collect();

    if items.is_empty() {
        let empty_list = List::new(vec![ListItem::new(Line::from(vec![Span::styled(
            "No activity",
            styles.text_muted.add_modifier(Modifier::ITALIC),
        )]))]);
        frame.render_widget(empty_list, inner);
    } else {
        let list = List::new(items);
        frame.render_widget(list, inner);
    }
}
