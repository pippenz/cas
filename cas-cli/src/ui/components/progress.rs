//! ProgressBar — determinate progress indicator with percentage
//!
//! Implements the Component trait for interactive use with the Program runner.
//! In plain mode, prints milestone percentages (25%, 50%, 75%, 100%).

use std::cell::Cell;
use std::io;
use std::time::Instant;

use super::formatter::{Formatter, OutputMode};
use super::traits::{Action, Component, Renderable};
use crate::ui::theme::Icons;

/// Messages the ProgressBar component responds to
pub enum ProgressBarMsg {
    /// Set the current progress value
    Set(u64),
    /// Increment progress by a given amount
    Increment(u64),
    /// Update the displayed message
    SetMessage(String),
    /// Advance animation frame (for the spinner in the bar)
    Tick,
    /// Mark as finished
    Finish,
}

/// Determinate progress bar with percentage display.
///
/// Styled mode: `  ⠋ Pushing 5/10 items [████████░░░░░░░░░░░░] 50%`
/// Plain mode: prints milestone percentages (25%, 50%, 75%, 100%)
///
/// ```ignore
/// let mut bar = ProgressBar::new(100).with_message("Downloading");
/// bar.update(ProgressBarMsg::Set(50));
/// ```
pub struct ProgressBar {
    current: u64,
    total: u64,
    message: String,
    bar_width: usize,
    show_eta: bool,
    started_at: Instant,
    finished: bool,
    spinner_frame: usize,
    /// Tracks which plain-mode milestone was last printed by view().
    /// Uses Cell for interior mutability since view() takes &self.
    printed_milestone: Cell<u8>,
}

impl ProgressBar {
    /// Create a new progress bar with the given total.
    pub fn new(total: u64) -> Self {
        Self {
            current: 0,
            total,
            message: String::new(),
            bar_width: 20,
            show_eta: false,
            started_at: Instant::now(),
            finished: false,
            spinner_frame: 0,
            printed_milestone: Cell::new(0),
        }
    }

    /// Set the progress message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = message.into();
        self
    }

    /// Set the visual bar width in characters (default: 20).
    pub fn with_bar_width(mut self, width: usize) -> Self {
        self.bar_width = width;
        self
    }

    /// Enable ETA display.
    pub fn with_eta(mut self) -> Self {
        self.show_eta = true;
        self
    }

    /// Get the current progress value.
    pub fn current(&self) -> u64 {
        self.current
    }

    /// Get the total value.
    pub fn total(&self) -> u64 {
        self.total
    }

    /// Get the progress as a percentage (0-100).
    pub fn percentage(&self) -> u8 {
        if self.total == 0 {
            return 100;
        }
        ((self.current as f64 / self.total as f64) * 100.0).min(100.0) as u8
    }

    /// Whether the progress bar is complete.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    /// Calculate estimated time remaining.
    fn eta(&self) -> Option<std::time::Duration> {
        if !self.show_eta || self.current == 0 || self.current >= self.total {
            return None;
        }
        let elapsed = self.started_at.elapsed();
        let rate = self.current as f64 / elapsed.as_secs_f64();
        if rate <= 0.0 {
            return None;
        }
        let remaining = (self.total - self.current) as f64 / rate;
        Some(std::time::Duration::from_secs_f64(remaining))
    }

    fn format_eta(duration: std::time::Duration) -> String {
        let secs = duration.as_secs();
        if secs < 60 {
            format!("{secs}s")
        } else if secs < 3600 {
            format!("{}m{}s", secs / 60, secs % 60)
        } else {
            format!("{}h{}m", secs / 3600, (secs % 3600) / 60)
        }
    }

    /// Current milestone (0, 25, 50, 75, 100) based on percentage.
    fn current_milestone(&self) -> u8 {
        let pct = self.percentage();
        if pct >= 100 {
            100
        } else if pct >= 75 {
            75
        } else if pct >= 50 {
            50
        } else if pct >= 25 {
            25
        } else {
            0
        }
    }

    fn spinner_char(&self) -> &'static str {
        Icons::SPINNER[self.spinner_frame % Icons::SPINNER.len()]
    }
}

impl Component for ProgressBar {
    type Msg = ProgressBarMsg;
    type Output = bool; // true = completed

    fn update(&mut self, msg: ProgressBarMsg) -> Action<ProgressBarMsg> {
        match msg {
            ProgressBarMsg::Set(value) => {
                self.current = value.min(self.total);
                if self.current >= self.total {
                    self.finished = true;
                    return Action::Quit;
                }
                Action::None
            }
            ProgressBarMsg::Increment(amount) => {
                self.current = (self.current + amount).min(self.total);
                if self.current >= self.total {
                    self.finished = true;
                    return Action::Quit;
                }
                Action::None
            }
            ProgressBarMsg::SetMessage(new_msg) => {
                self.message = new_msg;
                Action::None
            }
            ProgressBarMsg::Tick => {
                self.spinner_frame = (self.spinner_frame + 1) % Icons::SPINNER.len();
                Action::None
            }
            ProgressBarMsg::Finish => {
                self.current = self.total;
                self.finished = true;
                Action::Quit
            }
        }
    }

    fn view(&self, fmt: &mut Formatter) -> io::Result<()> {
        if fmt.mode() == OutputMode::Styled {
            self.render_styled(fmt)
        } else {
            self.render_plain(fmt)
        }
    }

    fn output(&self) -> bool {
        self.finished
    }
}

impl ProgressBar {
    fn render_styled(&self, fmt: &mut Formatter) -> io::Result<()> {
        let pct = self.percentage();
        let filled = if self.total == 0 {
            self.bar_width
        } else {
            (self.current as usize * self.bar_width) / self.total as usize
        };
        let empty = self.bar_width - filled;

        // Spinner + message
        let spinner = if self.finished {
            Icons::CHECK
        } else {
            self.spinner_char()
        };

        let msg_color = if self.finished {
            fmt.theme().palette.status_success
        } else {
            fmt.theme().palette.accent
        };

        fmt.write_colored(&format!("  {spinner} "), msg_color)?;

        // Message with pos/total
        if !self.message.is_empty() {
            fmt.write_primary(&self.message)?;
            fmt.write_raw(" ")?;
        }

        if !self.finished {
            fmt.write_muted(&format!("{}/{} ", self.current, self.total))?;
        }

        // Bar: [████░░░░]
        fmt.write_raw("[")?;
        let filled_str = Icons::PROGRESS_FULL.repeat(filled);
        let filled_color = fmt.theme().palette.accent;
        fmt.write_colored(&filled_str, filled_color)?;
        let empty_str = Icons::PROGRESS_EMPTY.repeat(empty);
        let empty_color = fmt.theme().palette.border_muted;
        fmt.write_colored(&empty_str, empty_color)?;
        fmt.write_raw("] ")?;

        // Percentage
        fmt.write_primary(&format!("{pct}%"))?;

        // ETA
        if let Some(eta) = self.eta() {
            fmt.write_muted(&format!(" ({})", Self::format_eta(eta)))?;
        }

        fmt.newline()
    }

    fn render_plain(&self, fmt: &mut Formatter) -> io::Result<()> {
        if self.finished {
            let msg = if self.message.is_empty() {
                "Complete".to_string()
            } else {
                format!("{} - Complete", self.message)
            };
            // Only print completion once
            if self.printed_milestone.get() < 100 {
                self.printed_milestone.set(100);
                fmt.write_raw(&format!("  100% {msg}"))?;
                fmt.newline()
            } else {
                Ok(())
            }
        } else {
            let milestone = self.current_milestone();
            if milestone > 0 && milestone > self.printed_milestone.get() {
                self.printed_milestone.set(milestone);
                let msg = if self.message.is_empty() {
                    String::new()
                } else {
                    format!(" {}", self.message)
                };
                fmt.write_raw(&format!("  {milestone}%{msg}"))?;
                fmt.newline()
            } else {
                Ok(())
            }
        }
    }
}

/// Static rendering for composed layouts.
impl Renderable for ProgressBar {
    fn render(&self, fmt: &mut Formatter) -> io::Result<()> {
        self.view(fmt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_bar_new() {
        let bar = ProgressBar::new(100);
        assert_eq!(bar.current(), 0);
        assert_eq!(bar.total(), 100);
        assert_eq!(bar.percentage(), 0);
        assert!(!bar.is_finished());
    }

    #[test]
    fn test_progress_bar_with_message() {
        let bar = ProgressBar::new(50).with_message("Uploading");
        assert_eq!(bar.message, "Uploading");
    }

    #[test]
    fn test_progress_bar_with_bar_width() {
        let bar = ProgressBar::new(100).with_bar_width(30);
        assert_eq!(bar.bar_width, 30);
    }

    #[test]
    fn test_progress_bar_set() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Set(50));
        assert_eq!(bar.current(), 50);
        assert_eq!(bar.percentage(), 50);
    }

    #[test]
    fn test_progress_bar_increment() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Increment(25));
        assert_eq!(bar.current(), 25);
        bar.update(ProgressBarMsg::Increment(25));
        assert_eq!(bar.current(), 50);
    }

    #[test]
    fn test_progress_bar_clamps_to_total() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Set(200));
        assert_eq!(bar.current(), 100);
        assert!(bar.is_finished());
    }

    #[test]
    fn test_progress_bar_increment_completes() {
        let mut bar = ProgressBar::new(10);
        bar.update(ProgressBarMsg::Set(5));
        let action = bar.update(ProgressBarMsg::Increment(5));
        assert!(matches!(action, Action::Quit));
        assert!(bar.is_finished());
        assert!(bar.output());
    }

    #[test]
    fn test_progress_bar_finish() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Set(30));
        let action = bar.update(ProgressBarMsg::Finish);
        assert!(matches!(action, Action::Quit));
        assert_eq!(bar.current(), 100);
        assert!(bar.is_finished());
    }

    #[test]
    fn test_progress_bar_set_message() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::SetMessage("New message".to_string()));
        assert_eq!(bar.message, "New message");
    }

    #[test]
    fn test_progress_bar_tick() {
        let mut bar = ProgressBar::new(100);
        assert_eq!(bar.spinner_frame, 0);
        bar.update(ProgressBarMsg::Tick);
        assert_eq!(bar.spinner_frame, 1);
    }

    #[test]
    fn test_progress_bar_percentage_zero_total() {
        let bar = ProgressBar::new(0);
        assert_eq!(bar.percentage(), 100);
    }

    #[test]
    fn test_progress_bar_milestones() {
        let mut bar = ProgressBar::new(100);
        assert_eq!(bar.current_milestone(), 0);

        bar.update(ProgressBarMsg::Set(25));
        assert_eq!(bar.current_milestone(), 25);

        bar.update(ProgressBarMsg::Set(50));
        assert_eq!(bar.current_milestone(), 50);

        bar.update(ProgressBarMsg::Set(75));
        assert_eq!(bar.current_milestone(), 75);
    }

    #[test]
    fn test_progress_bar_view_styled() {
        use crate::ui::theme::ActiveTheme;

        let mut bar = ProgressBar::new(10).with_message("Pushing");
        bar.update(ProgressBarMsg::Set(5));

        let theme = ActiveTheme::default_dark();
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::styled(&mut buf, theme);
            bar.view(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("Pushing"));
        assert!(output.contains("5/10"));
        assert!(output.contains("50%"));
        assert!(output.contains("\x1b[")); // ANSI codes
    }

    #[test]
    fn test_progress_bar_view_styled_finished() {
        use crate::ui::theme::ActiveTheme;

        let mut bar = ProgressBar::new(10).with_message("Pushing");
        bar.update(ProgressBarMsg::Finish);

        let theme = ActiveTheme::default_dark();
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::styled(&mut buf, theme);
            bar.view(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("100%"));
    }

    #[test]
    fn test_progress_bar_view_plain_milestone() {
        let mut bar = ProgressBar::new(100).with_message("Syncing");
        bar.update(ProgressBarMsg::Set(50));

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("50%"));
        assert!(output.contains("Syncing"));
    }

    #[test]
    fn test_progress_bar_view_plain_finished() {
        let mut bar = ProgressBar::new(100).with_message("Syncing");
        bar.update(ProgressBarMsg::Finish);

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("100%"));
        assert!(output.contains("Complete"));
    }

    #[test]
    fn test_progress_bar_view_plain_no_message_finished() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Finish);

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("100% Complete"));
    }

    #[test]
    fn test_progress_bar_renderable() {
        let bar = ProgressBar::new(100).with_message("test");
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.render(&mut fmt).unwrap();
        }
        // At 0% with milestone 0, no output until first milestone
        let output = String::from_utf8(buf).unwrap();
        assert!(output.is_empty());
    }

    #[test]
    fn test_progress_bar_plain_no_duplicate_milestones() {
        let mut bar = ProgressBar::new(100).with_message("Syncing");
        bar.update(ProgressBarMsg::Set(50));

        // First view should print the 50% milestone
        let mut buf1 = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf1);
            bar.view(&mut fmt).unwrap();
        }
        let output1 = String::from_utf8(buf1).unwrap();
        assert!(output1.contains("50%"));

        // Second view at same progress should produce no output
        let mut buf2 = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf2);
            bar.view(&mut fmt).unwrap();
        }
        let output2 = String::from_utf8(buf2).unwrap();
        assert!(
            output2.is_empty(),
            "Expected no output on duplicate view, got: {output2:?}"
        );
    }

    #[test]
    fn test_progress_bar_plain_sequential_milestones() {
        let mut bar = ProgressBar::new(100).with_message("Syncing");

        // Advance to 25%
        bar.update(ProgressBarMsg::Set(25));
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        assert!(String::from_utf8(buf).unwrap().contains("25%"));

        // Advance to 50%
        bar.update(ProgressBarMsg::Set(50));
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        assert!(String::from_utf8(buf).unwrap().contains("50%"));

        // Advance to 75%
        bar.update(ProgressBarMsg::Set(75));
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            bar.view(&mut fmt).unwrap();
        }
        assert!(String::from_utf8(buf).unwrap().contains("75%"));
    }

    #[test]
    fn test_progress_bar_plain_finish_no_duplicate() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Finish);

        // First view prints completion
        let mut buf1 = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf1);
            bar.view(&mut fmt).unwrap();
        }
        assert!(String::from_utf8(buf1).unwrap().contains("100%"));

        // Second view produces nothing
        let mut buf2 = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf2);
            bar.view(&mut fmt).unwrap();
        }
        assert!(String::from_utf8(buf2).unwrap().is_empty());
    }

    #[test]
    fn test_format_eta_seconds() {
        let eta = std::time::Duration::from_secs(45);
        assert_eq!(ProgressBar::format_eta(eta), "45s");
    }

    #[test]
    fn test_format_eta_minutes() {
        let eta = std::time::Duration::from_secs(125);
        assert_eq!(ProgressBar::format_eta(eta), "2m5s");
    }

    #[test]
    fn test_format_eta_hours() {
        let eta = std::time::Duration::from_secs(3665);
        assert_eq!(ProgressBar::format_eta(eta), "1h1m");
    }

    #[test]
    fn test_progress_bar_with_eta_enabled() {
        let bar = ProgressBar::new(100).with_eta();
        assert!(bar.show_eta);
    }

    #[test]
    fn test_progress_bar_eta_none_at_zero() {
        let bar = ProgressBar::new(100).with_eta();
        assert!(bar.eta().is_none());
    }

    #[test]
    fn test_progress_bar_eta_none_when_disabled() {
        let mut bar = ProgressBar::new(100);
        bar.update(ProgressBarMsg::Set(50));
        assert!(bar.eta().is_none());
    }
}
