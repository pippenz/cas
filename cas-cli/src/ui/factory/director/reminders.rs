//! Reminders panel widget for the director sidebar
//!
//! Shows pending reminders set by the supervisor. Only displayed when
//! there are active reminders in the session.

use cas_store::{Reminder, ReminderTriggerType};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, ListState};

use crate::ui::factory::director::data::DirectorData;
use crate::ui::theme::ActiveTheme;

/// Render the reminders section with optional focus indicator.
///
/// Only call this when `data.reminders` is non-empty.
pub fn render_with_focus(
    frame: &mut Frame,
    area: Rect,
    data: &DirectorData,
    theme: &ActiveTheme,
    focused: bool,
    collapsed: bool,
    reminders_state: Option<&mut ListState>,
) {
    let styles = &theme.styles;
    let count = data.reminders.len();

    if collapsed {
        super::panel::render_collapsed_header(
            frame,
            area,
            styles,
            super::panel::CollapsedHeader {
                title: "REMINDERS",
                count,
                hotkey: None,
                focused,
                icon_style: None,
            },
        );
        return;
    }

    let inner = super::panel::render_panel_block(frame, area, "REMINDERS", count, focused, styles);

    if inner.height == 0 {
        return;
    }

    // Calculate available width for message (area width - icon - trigger - padding)
    let available_msg_width = inner.width as usize;

    let items: Vec<ListItem> = data
        .reminders
        .iter()
        .map(|r| format_reminder(r, styles, available_msg_width))
        .collect();

    let list = List::new(items).highlight_style(styles.bg_selection);

    if let Some(state) = reminders_state {
        frame.render_stateful_widget(list, inner, state);
    } else {
        frame.render_widget(list, inner);
    }
}

/// Format a single reminder as a ListItem
fn format_reminder(
    reminder: &Reminder,
    styles: &crate::ui::theme::Styles,
    available_width: usize,
) -> ListItem<'static> {
    use cas_store::ReminderStatus;

    let is_fired = matches!(reminder.status, ReminderStatus::Fired);

    let trigger_icon = if is_fired {
        "✓"
    } else {
        match reminder.trigger_type {
            ReminderTriggerType::Time => "⏱",
            ReminderTriggerType::Event => "⚡",
        }
    };

    let status_label = match reminder.status {
        ReminderStatus::Pending => match reminder.trigger_type {
            ReminderTriggerType::Time => {
                if let Some(at) = reminder.trigger_at {
                    let now = chrono::Utc::now();
                    if at > now {
                        let remaining = (at - now).num_seconds();
                        let mins = remaining / 60;
                        let secs = remaining % 60;
                        if mins > 0 {
                            format!("{mins}m {secs}s")
                        } else {
                            format!("{secs}s")
                        }
                    } else {
                        "due".to_string()
                    }
                } else {
                    "pending".to_string()
                }
            }
            ReminderTriggerType::Event => "pending".to_string(),
        },
        ReminderStatus::Fired => "fired".to_string(),
        ReminderStatus::Cancelled => "cancelled".to_string(),
        ReminderStatus::Expired => "expired".to_string(),
    };

    // Truncate message based on available width
    // Layout: " {icon} " (4) + "{status:<12} " (13) + message
    let prefix_width = 4 + 13;
    let max_msg_len = available_width.saturating_sub(prefix_width);
    let msg = if max_msg_len < 4 {
        String::new()
    } else if reminder.message.len() > max_msg_len {
        format!("{}...", &reminder.message[..max_msg_len.saturating_sub(3)])
    } else {
        reminder.message.clone()
    };

    let icon_style = if is_fired {
        styles.text_success
    } else {
        styles.text_info
    };
    let msg_style = if is_fired {
        styles.text_muted
    } else {
        styles.text_primary
    };

    let line = Line::from(vec![
        Span::styled(format!(" {trigger_icon} "), icon_style),
        Span::styled(format!("{status_label:<12} "), styles.text_muted),
        Span::styled(msg, msg_style),
    ]);

    ListItem::new(line)
}
