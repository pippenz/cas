//! Shared sidecar panel state and rendering helpers.
//!
//! All sidecar panels (Factory, Tasks, Reminders, Changes, Activity) share
//! the same scroll/select/collapse mechanics. This module provides a
//! `PanelState` struct that owns that logic and a `PanelRegistry` that
//! groups all five panel states together.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, ListState, Paragraph};

use super::SidecarFocus;
use crate::ui::theme::Styles;

/// Per-panel state: scroll position and collapse flag.
#[derive(Debug, Default)]
pub struct PanelState {
    pub list_state: ListState,
    pub collapsed: bool,
}

impl PanelState {
    /// Initialize selection to 0 if nothing is selected and items exist.
    pub fn init_selection(&mut self, item_count: usize) {
        if self.list_state.selected().is_none() && item_count > 0 {
            self.list_state.select(Some(0));
        }
    }

    /// Scroll up by one position.
    pub fn scroll_up(&mut self) {
        if let Some(i) = self.list_state.selected() {
            if i > 0 {
                self.list_state.select(Some(i - 1));
            }
        }
    }

    /// Scroll down by one, bounded by `item_count`.
    pub fn scroll_down(&mut self, item_count: usize) {
        if item_count > 0 {
            if let Some(i) = self.list_state.selected() {
                if i < item_count - 1 {
                    self.list_state.select(Some(i + 1));
                }
            } else {
                self.list_state.select(Some(0));
            }
        }
    }

    /// Toggle the collapsed state.
    pub fn toggle_collapsed(&mut self) {
        self.collapsed = !self.collapsed;
    }
}

/// Registry of all sidecar panel states.
#[derive(Debug, Default)]
pub struct PanelRegistry {
    pub factory: PanelState,
    pub tasks: PanelState,
    pub reminders: PanelState,
    pub changes: PanelState,
    pub activity: PanelState,
}

impl PanelRegistry {
    /// Get a mutable reference to the panel for the given focus.
    pub fn get_mut(&mut self, focus: SidecarFocus) -> Option<&mut PanelState> {
        match focus {
            SidecarFocus::None => None,
            SidecarFocus::Factory => Some(&mut self.factory),
            SidecarFocus::Tasks => Some(&mut self.tasks),
            SidecarFocus::Reminders => Some(&mut self.reminders),
            SidecarFocus::Changes => Some(&mut self.changes),
            SidecarFocus::Activity => Some(&mut self.activity),
        }
    }

    /// Get an immutable reference to the panel for the given focus.
    pub fn get(&self, focus: SidecarFocus) -> Option<&PanelState> {
        match focus {
            SidecarFocus::None => None,
            SidecarFocus::Factory => Some(&self.factory),
            SidecarFocus::Tasks => Some(&self.tasks),
            SidecarFocus::Reminders => Some(&self.reminders),
            SidecarFocus::Changes => Some(&self.changes),
            SidecarFocus::Activity => Some(&self.activity),
        }
    }
}

// =========================================================================
// Shared rendering helpers
// =========================================================================

/// Content description for a collapsed panel header.
pub struct CollapsedHeader<'a> {
    pub title: &'a str,
    pub count: usize,
    pub hotkey: Option<&'a str>,
    pub focused: bool,
    pub icon_style: Option<Style>,
}

/// Render a collapsed panel header: `▸ TITLE (count) [hotkey]`
pub fn render_collapsed_header(
    frame: &mut Frame,
    area: Rect,
    styles: &Styles,
    header: CollapsedHeader<'_>,
) {
    let title_style = if header.focused {
        styles.text_info
    } else {
        styles.text_primary
    };

    let icon_style = header.icon_style.unwrap_or(styles.text_info);
    let mut spans = vec![
        Span::styled("▸ ", icon_style),
        Span::styled(header.title, title_style.add_modifier(Modifier::BOLD)),
        Span::styled(format!(" ({})", header.count), styles.text_muted),
    ];
    if let Some(key) = header.hotkey {
        spans.push(Span::styled(format!(" [{key}]"), styles.text_muted));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Render an expanded panel block with `▾ TITLE (count)` title.
///
/// Returns the inner `Rect` for content rendering.
pub fn render_panel_block(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    count: usize,
    focused: bool,
    styles: &Styles,
) -> Rect {
    let title_style = if focused {
        styles.text_info
    } else {
        styles.text_primary
    };

    let block = Block::default()
        .title(Line::from(vec![
            Span::styled("▾ ", styles.text_info),
            Span::styled(title, title_style.add_modifier(Modifier::BOLD)),
            Span::styled(format!(" ({count})"), styles.text_muted),
        ]))
        .borders(Borders::NONE);

    let inner = block.inner(area);
    frame.render_widget(block, area);
    inner
}
