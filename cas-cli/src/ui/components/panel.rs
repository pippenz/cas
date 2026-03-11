//! Panel component — bordered box with title for grouping content
//!
//! An inline equivalent of ratatui's Block, using Unicode box-drawing
//! characters for styled output and ASCII for plain mode.
//!
//! ```ignore
//! Panel::new("Task Details")
//!     .content(KeyValue::new().entry("ID", "cas-1").entry("Status", "Open"))
//!     .render(&mut fmt)?;
//!
//! // Styled:
//! // ╭─ Task Details ──────────────────╮
//! // │ ID:     cas-1                   │
//! // │ Status: Open                    │
//! // ╰─────────────────────────────────╯
//! //
//! // Plain:
//! // +- Task Details -----------------+
//! // | ID:     cas-1                  |
//! // | Status: Open                   |
//! // +--------------------------------+
//! ```

use std::io;

use super::formatter::{Formatter, OutputMode};
use super::traits::Renderable;

/// A bordered panel that wraps other Renderable content.
pub struct Panel {
    title: String,
    children: Vec<Box<dyn Renderable>>,
}

impl Panel {
    /// Create a panel with a title.
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            children: Vec::new(),
        }
    }

    /// Add a renderable child component inside the panel.
    pub fn content(mut self, child: impl Renderable + 'static) -> Self {
        self.children.push(Box::new(child));
        self
    }

    /// Render the panel's inner content to a buffer, then wrap with borders.
    ///
    /// This two-pass approach lets us measure content width before drawing borders.
    fn render_inner(&self) -> Vec<String> {
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::new(
                &mut buf,
                OutputMode::Plain,
                crate::ui::theme::ActiveTheme::default(),
                // Use a wide width so content isn't pre-truncated
                500,
            );
            for child in &self.children {
                let _ = child.render(&mut fmt);
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines: Vec<String> = text.lines().map(|l| l.to_string()).collect();
        // Remove trailing empty lines
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        lines
    }
}

impl Renderable for Panel {
    fn render(&self, fmt: &mut Formatter) -> io::Result<()> {
        let content_lines = self.render_inner();
        let width = fmt.width() as usize;

        // Determine border characters
        let (tl, tr, bl, br, h, v) = if fmt.is_styled() {
            (
                "\u{256D}", "\u{256E}", "\u{2570}", "\u{256F}", "\u{2500}", "\u{2502}",
            )
        } else {
            ("+", "+", "+", "+", "-", "|")
        };

        // Calculate inner width: total width minus 2 border chars and 2 padding spaces
        let inner_width = width.saturating_sub(4).max(self.title.chars().count() + 2);

        // Top border: ╭─ Title ──────╮
        let title_display = if self.title.is_empty() {
            let fill = h.repeat(inner_width + 2);
            format!("{tl}{fill}{tr}")
        } else {
            let title_len = self.title.chars().count();
            let fill_after = inner_width.saturating_sub(title_len + 1);
            format!(
                "{tl}{h} {title} {fill}{tr}",
                title = self.title,
                fill = h.repeat(fill_after)
            )
        };
        fmt.write_muted(&title_display)?;
        fmt.newline()?;

        // Content lines
        if content_lines.is_empty() {
            // Empty panel — just show borders
            let padding = " ".repeat(inner_width);
            fmt.write_muted(v)?;
            fmt.write_raw(&format!(" {padding} "))?;
            fmt.write_muted(v)?;
            fmt.newline()?;
        } else {
            for line in &content_lines {
                let char_count = line.chars().count();
                let truncated = if char_count > inner_width {
                    let mut s: String = line.chars().take(inner_width.saturating_sub(1)).collect();
                    s.push('\u{2026}'); // …
                    s
                } else {
                    line.clone()
                };
                let pad_len = inner_width.saturating_sub(truncated.chars().count());
                let padding = " ".repeat(pad_len);

                fmt.write_muted(v)?;
                fmt.write_raw(" ")?;
                fmt.write_primary(&truncated)?;
                fmt.write_raw(&padding)?;
                fmt.write_raw(" ")?;
                fmt.write_muted(v)?;
                fmt.newline()?;
            }
        }

        // Bottom border: ╰──────────────╯
        let bottom_fill = h.repeat(inner_width + 2);
        fmt.write_muted(&format!("{bl}{bottom_fill}{br}"))?;
        fmt.newline()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::ActiveTheme;

    /// Simple renderable for testing panel content.
    struct TextBlock(String);

    impl Renderable for TextBlock {
        fn render(&self, fmt: &mut Formatter) -> io::Result<()> {
            fmt.write_primary(&self.0)?;
            fmt.newline()
        }
    }

    fn render_plain(panel: &Panel) -> String {
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            panel.render(&mut fmt).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn render_plain_width(panel: &Panel, width: u16) -> String {
        let mut buf = Vec::new();
        {
            let mut fmt =
                Formatter::new(&mut buf, OutputMode::Plain, ActiveTheme::default(), width);
            panel.render(&mut fmt).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_panel_with_title_plain() {
        let panel = Panel::new("Details").content(TextBlock("Hello World".to_string()));
        let output = render_plain(&panel);

        assert!(output.contains("Details"));
        assert!(output.contains("Hello World"));
        // Should have borders
        assert!(output.contains("+"));
        assert!(output.contains("|"));
    }

    #[test]
    fn test_panel_empty_content_plain() {
        let panel = Panel::new("Empty");
        let output = render_plain(&panel);

        assert!(output.contains("Empty"));
        // Top and bottom borders
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 3); // top + empty content + bottom
    }

    #[test]
    fn test_panel_multiline_content_plain() {
        let panel = Panel::new("Multi")
            .content(TextBlock("Line one".to_string()))
            .content(TextBlock("Line two".to_string()));

        let output = render_plain(&panel);
        assert!(output.contains("Line one"));
        assert!(output.contains("Line two"));
    }

    #[test]
    fn test_panel_no_title_plain() {
        let panel = Panel::new("").content(TextBlock("content".to_string()));
        let output = render_plain(&panel);

        assert!(output.contains("content"));
        // Should still have top/bottom borders
        let lines: Vec<&str> = output.lines().collect();
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_panel_width_aware() {
        let panel = Panel::new("Test").content(TextBlock("Short".to_string()));
        let output = render_plain_width(&panel, 30);

        for line in output.lines() {
            assert!(
                line.chars().count() <= 30,
                "Line too wide ({} chars): {:?}",
                line.chars().count(),
                line
            );
        }
    }

    #[test]
    fn test_panel_consistent_border_width() {
        let panel = Panel::new("Title")
            .content(TextBlock("abc".to_string()))
            .content(TextBlock("longer content here".to_string()));

        let output = render_plain(&panel);
        let lines: Vec<&str> = output.lines().collect();

        // All lines should have the same width
        let widths: Vec<usize> = lines.iter().map(|l| l.chars().count()).collect();
        let first_width = widths[0];
        for (i, &w) in widths.iter().enumerate() {
            assert_eq!(
                w, first_width,
                "Line {i} has width {w}, expected {first_width}: {:?}",
                lines[i]
            );
        }
    }

    #[test]
    fn test_panel_styled_has_ansi() {
        let theme = ActiveTheme::default_dark();
        let panel = Panel::new("Styled").content(TextBlock("content".to_string()));

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::styled(&mut buf, theme);
            panel.render(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("\x1b["));
        assert!(output.contains("Styled"));
        assert!(output.contains("content"));
    }

    #[test]
    fn test_panel_plain_uses_ascii_borders() {
        let panel = Panel::new("Test").content(TextBlock("x".to_string()));
        let output = render_plain(&panel);

        // ASCII mode should use + and | and -
        assert!(output.contains("+"));
        assert!(output.contains("|"));
        assert!(output.contains("-"));
        // Should NOT contain unicode box chars
        assert!(!output.contains("\u{256D}"));
        assert!(!output.contains("\u{2502}"));
    }

    #[test]
    fn test_panel_truncates_long_content() {
        let long_text = "A".repeat(200);
        let panel = Panel::new("Narrow").content(TextBlock(long_text));
        let output = render_plain_width(&panel, 40);

        for line in output.lines() {
            assert!(line.chars().count() <= 40, "Line too wide: {:?}", line);
        }
    }

    #[test]
    fn test_panel_top_border_contains_title() {
        let panel = Panel::new("My Panel");
        let output = render_plain(&panel);
        let first_line = output.lines().next().unwrap();
        assert!(first_line.contains("My Panel"));
        assert!(first_line.starts_with("+"));
        assert!(first_line.ends_with("+"));
    }

    #[test]
    fn test_panel_bottom_border() {
        let panel = Panel::new("Bottom").content(TextBlock("x".to_string()));
        let output = render_plain(&panel);
        let last_line = output.lines().last().unwrap();
        assert!(last_line.starts_with("+"));
        assert!(last_line.ends_with("+"));
        assert!(last_line.contains("-"));
    }
}
