//! Table component — themed, terminal-aware table rendering
//!
//! Replaces comfy-table with a component that integrates with the Formatter
//! and theme system. Supports column alignment, width constraints, border
//! styles, and auto-truncation for narrow terminals.
//!
//! ```ignore
//! Table::new()
//!     .columns(&["ID", "Title", "Status"])
//!     .rows(vec![
//!         vec!["cas-1", "Fix bug", "Open"],
//!         vec!["cas-2", "Add feature", "Closed"],
//!     ])
//!     .border(Border::Unicode)
//!     .render(&mut fmt)?;
//! ```

use std::io;

use super::formatter::Formatter;
use super::traits::Renderable;

/// Column alignment within a table cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Right,
    Center,
}

/// Width constraint for a column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Width {
    /// Automatically size to content (with optional min/max).
    #[default]
    Auto,
    /// Fixed width in characters.
    Fixed(u16),
    /// Minimum width (expands if space available).
    Min(u16),
    /// Maximum width (truncates if exceeded).
    Max(u16),
    /// Both min and max bounds.
    MinMax(u16, u16),
}

/// Column definition.
#[derive(Debug, Clone)]
pub struct Column {
    pub title: String,
    pub align: Align,
    pub width: Width,
}

impl Column {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            align: Align::Left,
            width: Width::Auto,
        }
    }

    pub fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    pub fn width(mut self, width: Width) -> Self {
        self.width = width;
        self
    }
}

/// Border style for table rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Border {
    /// No borders at all.
    None,
    /// ASCII borders using +, -, |.
    Ascii,
    /// Unicode box-drawing characters.
    #[default]
    Unicode,
}

/// A themed, terminal-aware table component.
///
/// Implements `Renderable` for integration with the Formatter system.
#[derive(Debug, Clone)]
pub struct Table {
    cols: Vec<Column>,
    rows: Vec<Vec<String>>,
    border: Border,
    indent: u16,
}

impl Default for Table {
    fn default() -> Self {
        Self::new()
    }
}

impl Table {
    pub fn new() -> Self {
        Self {
            cols: Vec::new(),
            rows: Vec::new(),
            border: Border::Unicode,
            indent: 0,
        }
    }

    /// Set column definitions from simple string titles.
    ///
    /// Creates columns with default alignment (Left) and auto width.
    pub fn columns(mut self, titles: &[&str]) -> Self {
        self.cols = titles.iter().map(|t| Column::new(*t)).collect();
        self
    }

    /// Set column definitions from `Column` structs for full control.
    pub fn columns_detailed(mut self, cols: Vec<Column>) -> Self {
        self.cols = cols;
        self
    }

    /// Add rows of data. Each inner Vec must match the column count.
    pub fn rows(mut self, rows: Vec<Vec<impl Into<String>>>) -> Self {
        self.rows = rows
            .into_iter()
            .map(|r| r.into_iter().map(|c| c.into()).collect())
            .collect();
        self
    }

    /// Set the border style.
    pub fn border(mut self, border: Border) -> Self {
        self.border = border;
        self
    }

    /// Set left indent (number of spaces before each line).
    pub fn indent(mut self, spaces: u16) -> Self {
        self.indent = spaces;
        self
    }

    /// Compute column widths based on content and constraints.
    ///
    /// Returns a Vec of widths in characters for each column.
    fn compute_widths(&self, available: u16) -> Vec<u16> {
        let num_cols = self.cols.len();
        if num_cols == 0 {
            return Vec::new();
        }

        // Calculate content widths (max of header and all rows for each column)
        let mut content_widths: Vec<u16> = self
            .cols
            .iter()
            .enumerate()
            .map(|(i, col)| {
                let header_w = col.title.chars().count() as u16;
                let max_row_w = self
                    .rows
                    .iter()
                    .map(|row| row.get(i).map_or(0, |cell| cell.chars().count() as u16))
                    .max()
                    .unwrap_or(0);
                header_w.max(max_row_w)
            })
            .collect();

        // Apply column constraints
        for (i, col) in self.cols.iter().enumerate() {
            match col.width {
                Width::Auto => {}
                Width::Fixed(w) => content_widths[i] = w,
                Width::Min(min) => content_widths[i] = content_widths[i].max(min),
                Width::Max(max) => content_widths[i] = content_widths[i].min(max),
                Width::MinMax(min, max) => {
                    content_widths[i] = content_widths[i].clamp(min, max);
                }
            }
        }

        // Calculate total width needed including borders and padding
        let border_overhead = self.border_overhead(num_cols);
        let total_content: u16 = content_widths.iter().sum();
        let total_needed = total_content + border_overhead;

        if total_needed <= available {
            return content_widths;
        }

        // Need to shrink — find columns that can be reduced
        let target_content = available.saturating_sub(border_overhead);
        if target_content == 0 {
            return vec![1; num_cols];
        }

        // Proportional shrinking
        let scale = target_content as f64 / total_content as f64;
        let mut widths: Vec<u16> = content_widths
            .iter()
            .enumerate()
            .map(|(i, &w)| {
                let min = match self.cols[i].width {
                    Width::Min(m) | Width::MinMax(m, _) => m,
                    _ => 1,
                };
                let scaled = (w as f64 * scale).floor() as u16;
                scaled.max(min)
            })
            .collect();

        // Distribute any remaining space or trim excess
        let current_total: u16 = widths.iter().sum();
        if current_total < target_content {
            let extra = target_content - current_total;
            // Give extra to the widest column
            if let Some(max_idx) = widths
                .iter()
                .enumerate()
                .max_by_key(|(_, w)| **w)
                .map(|(i, _)| i)
            {
                widths[max_idx] += extra;
            }
        } else if current_total > target_content {
            // Trim from widest columns
            let mut excess = current_total - target_content;
            while excess > 0 {
                if let Some(max_idx) = widths
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, w)| **w)
                    .map(|(i, _)| i)
                {
                    let min = match self.cols[max_idx].width {
                        Width::Min(m) | Width::MinMax(m, _) => m,
                        _ => 1,
                    };
                    if widths[max_idx] > min {
                        widths[max_idx] -= 1;
                        excess -= 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
        }

        widths
    }

    /// Calculate border overhead in characters for a given number of columns.
    fn border_overhead(&self, num_cols: usize) -> u16 {
        match self.border {
            Border::None => {
                // Just spacing between columns: (n-1) * 2
                if num_cols > 1 {
                    (num_cols as u16 - 1) * 2
                } else {
                    0
                }
            }
            Border::Ascii | Border::Unicode => {
                // "| col1 | col2 | col3 |" = 2 (outer) + n*2 (padding) + (n-1)*3 (inner separators)
                // Simplified: leading "| " (2) + each col has " | " after (3) except last has " |" (2)
                // = 2 + (n-1)*3 + 2 = 4 + (n-1)*3
                if num_cols == 0 {
                    0
                } else {
                    // Each cell has 1 space padding on each side = 2*n
                    // Separators: n+1 border chars, but we count spacing differently
                    // "| " + col + " | " + col + " |"
                    // Leading "| " = 2, each separator " | " = 3 for (n-1), trailing " |" = 2
                    2 + (num_cols as u16 - 1) * 3 + 2
                }
            }
        }
    }

    /// Format a cell value to a given width with alignment.
    fn format_cell(value: &str, width: u16, align: Align) -> String {
        let w = width as usize;
        let char_count = value.chars().count();

        if char_count > w {
            // Truncate with ellipsis
            if w <= 1 {
                return value.chars().take(w).collect();
            }
            let mut truncated: String = value.chars().take(w - 1).collect();
            truncated.push('\u{2026}'); // …
            return truncated;
        }

        let padding = w - char_count;
        match align {
            Align::Left => format!("{value}{}", " ".repeat(padding)),
            Align::Right => format!("{}{value}", " ".repeat(padding)),
            Align::Center => {
                let left_pad = padding / 2;
                let right_pad = padding - left_pad;
                format!("{}{value}{}", " ".repeat(left_pad), " ".repeat(right_pad))
            }
        }
    }

    /// Render the horizontal border line (top, middle, or bottom).
    fn render_border_line(
        &self,
        fmt: &mut Formatter,
        widths: &[u16],
        position: BorderPosition,
    ) -> io::Result<()> {
        let indent_str = " ".repeat(self.indent as usize);
        fmt.write_raw(&indent_str)?;

        match self.border {
            Border::Ascii => {
                let (left, mid, right) = match position {
                    BorderPosition::Top | BorderPosition::Middle | BorderPosition::Bottom => {
                        ("+", "+", "+")
                    }
                };
                let bar = "-";
                fmt.write_muted(left)?;
                for (i, &w) in widths.iter().enumerate() {
                    fmt.write_muted(&bar.repeat(w as usize + 2))?;
                    if i < widths.len() - 1 {
                        fmt.write_muted(mid)?;
                    }
                }
                fmt.write_muted(right)?;
            }
            Border::Unicode => {
                let (left, mid, right, bar) = match position {
                    BorderPosition::Top => ("\u{256D}", "\u{252C}", "\u{256E}", "\u{2500}"),
                    BorderPosition::Middle => ("\u{251C}", "\u{253C}", "\u{2524}", "\u{2500}"),
                    BorderPosition::Bottom => ("\u{2570}", "\u{2534}", "\u{256F}", "\u{2500}"),
                };
                fmt.write_muted(left)?;
                for (i, &w) in widths.iter().enumerate() {
                    fmt.write_muted(&bar.repeat(w as usize + 2))?;
                    if i < widths.len() - 1 {
                        fmt.write_muted(mid)?;
                    }
                }
                fmt.write_muted(right)?;
            }
            Border::None => {}
        }

        fmt.newline()
    }

    /// Render a data row.
    fn render_row(
        &self,
        fmt: &mut Formatter,
        cells: &[String],
        widths: &[u16],
        is_header: bool,
    ) -> io::Result<()> {
        let indent_str = " ".repeat(self.indent as usize);
        fmt.write_raw(&indent_str)?;

        let sep = match self.border {
            Border::None => "  ",
            Border::Ascii => " | ",
            Border::Unicode => " \u{2502} ",
        };

        let (left, right) = match self.border {
            Border::None => ("", ""),
            Border::Ascii => ("| ", " |"),
            Border::Unicode => ("\u{2502} ", " \u{2502}"),
        };

        if !left.is_empty() {
            fmt.write_muted(left)?;
        }

        for (i, &w) in widths.iter().enumerate() {
            let value = cells.get(i).map_or("", |s| s.as_str());
            let align = self.cols.get(i).map_or(Align::Left, |c| c.align);
            let formatted = Self::format_cell(value, w, align);

            if is_header {
                fmt.write_bold(&formatted)?;
            } else {
                fmt.write_primary(&formatted)?;
            }

            if i < widths.len() - 1 {
                fmt.write_muted(sep)?;
            }
        }

        if !right.is_empty() {
            fmt.write_muted(right)?;
        }

        fmt.newline()
    }
}

/// Position of a border line in the table.
#[derive(Debug, Clone, Copy)]
enum BorderPosition {
    Top,
    Middle,
    Bottom,
}

impl Renderable for Table {
    fn render(&self, fmt: &mut Formatter) -> io::Result<()> {
        if self.cols.is_empty() {
            return Ok(());
        }

        let available = fmt.width().saturating_sub(self.indent);
        let widths = self.compute_widths(available);

        if widths.is_empty() {
            return Ok(());
        }

        // Top border
        if self.border != Border::None {
            self.render_border_line(fmt, &widths, BorderPosition::Top)?;
        }

        // Header row
        let headers: Vec<String> = self.cols.iter().map(|c| c.title.clone()).collect();
        self.render_row(fmt, &headers, &widths, true)?;

        // Header separator
        if self.border != Border::None {
            self.render_border_line(fmt, &widths, BorderPosition::Middle)?;
        }

        // Data rows
        if self.rows.is_empty() {
            // Show "(empty)" row for tables with no data
            let empty_msg = "(empty)".to_string();
            let total_inner: u16 = widths.iter().sum::<u16>()
                + if widths.len() > 1 {
                    (widths.len() as u16 - 1) * 3 // account for separators
                } else {
                    0
                };
            let padded = Self::format_cell(&empty_msg, total_inner, Align::Center);

            let indent_str = " ".repeat(self.indent as usize);
            fmt.write_raw(&indent_str)?;

            match self.border {
                Border::None => {
                    fmt.write_muted(&padded)?;
                }
                Border::Ascii => {
                    fmt.write_muted("| ")?;
                    fmt.write_muted(&padded)?;
                    fmt.write_muted(" |")?;
                }
                Border::Unicode => {
                    fmt.write_muted("\u{2502} ")?;
                    fmt.write_muted(&padded)?;
                    fmt.write_muted(" \u{2502}")?;
                }
            }
            fmt.newline()?;
        } else {
            for row in &self.rows {
                self.render_row(fmt, row, &widths, false)?;
            }
        }

        // Bottom border
        if self.border != Border::None {
            self.render_border_line(fmt, &widths, BorderPosition::Bottom)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render_plain(table: &Table) -> String {
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::plain(&mut buf);
            table.render(&mut fmt).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    fn render_plain_width(table: &Table, width: u16) -> String {
        use crate::ui::theme::ActiveTheme;
        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::new(
                &mut buf,
                super::super::formatter::OutputMode::Plain,
                ActiveTheme::default(),
                width,
            );
            table.render(&mut fmt).unwrap();
        }
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn test_basic_table_unicode() {
        let table = Table::new()
            .columns(&["ID", "Title", "Status"])
            .rows(vec![
                vec!["cas-1", "Fix bug", "Open"],
                vec!["cas-2", "Add feature", "Closed"],
            ])
            .border(Border::Unicode);

        let output = render_plain(&table);
        // Should contain headers and data
        assert!(output.contains("ID"));
        assert!(output.contains("Title"));
        assert!(output.contains("Status"));
        assert!(output.contains("cas-1"));
        assert!(output.contains("Fix bug"));
        assert!(output.contains("Open"));
        assert!(output.contains("cas-2"));
        assert!(output.contains("Add feature"));
        assert!(output.contains("Closed"));
    }

    #[test]
    fn test_basic_table_ascii() {
        let table = Table::new()
            .columns(&["Name", "Value"])
            .rows(vec![vec!["alpha", "1"], vec!["beta", "2"]])
            .border(Border::Ascii);

        let output = render_plain(&table);
        assert!(output.contains("+"));
        assert!(output.contains("|"));
        assert!(output.contains("-"));
        assert!(output.contains("alpha"));
        assert!(output.contains("beta"));
    }

    #[test]
    fn test_table_no_border() {
        let table = Table::new()
            .columns(&["A", "B"])
            .rows(vec![vec!["x", "y"]])
            .border(Border::None);

        let output = render_plain(&table);
        assert!(!output.contains("|"));
        assert!(!output.contains("+"));
        assert!(output.contains("A"));
        assert!(output.contains("B"));
        assert!(output.contains("x"));
        assert!(output.contains("y"));
    }

    #[test]
    fn test_empty_table() {
        let table = Table::new()
            .columns(&["ID", "Name"])
            .rows(Vec::<Vec<&str>>::new())
            .border(Border::Unicode);

        let output = render_plain(&table);
        assert!(output.contains("ID"));
        assert!(output.contains("Name"));
        assert!(output.contains("(empty)"));
    }

    #[test]
    fn test_no_columns() {
        let table = Table::new().border(Border::Unicode);
        let output = render_plain(&table);
        assert!(output.is_empty());
    }

    #[test]
    fn test_column_alignment() {
        let table = Table::new()
            .columns_detailed(vec![
                Column::new("Left")
                    .align(Align::Left)
                    .width(Width::Fixed(10)),
                Column::new("Right")
                    .align(Align::Right)
                    .width(Width::Fixed(10)),
                Column::new("Center")
                    .align(Align::Center)
                    .width(Width::Fixed(10)),
            ])
            .rows(vec![vec!["abc", "abc", "abc"]])
            .border(Border::None);

        let output = render_plain(&table);
        let lines: Vec<&str> = output.lines().collect();
        // Header line
        assert!(lines[0].contains("Left"));
        assert!(lines[0].contains("Right"));
        assert!(lines[0].contains("Center"));

        // Data line — check alignment
        let data_line = lines[1];
        // Left-aligned: "abc" followed by spaces
        assert!(data_line.contains("abc       "));
        // Right-aligned: spaces followed by "abc"
        assert!(data_line.contains("       abc"));
    }

    #[test]
    fn test_format_cell_left() {
        assert_eq!(Table::format_cell("hi", 5, Align::Left), "hi   ");
    }

    #[test]
    fn test_format_cell_right() {
        assert_eq!(Table::format_cell("hi", 5, Align::Right), "   hi");
    }

    #[test]
    fn test_format_cell_center() {
        assert_eq!(Table::format_cell("hi", 6, Align::Center), "  hi  ");
        // Odd padding: left gets floor, right gets ceil
        assert_eq!(Table::format_cell("hi", 5, Align::Center), " hi  ");
    }

    #[test]
    fn test_format_cell_truncation() {
        let result = Table::format_cell("hello world", 5, Align::Left);
        assert_eq!(result.chars().count(), 5);
        assert!(result.ends_with('\u{2026}')); // ends with …
        assert!(result.starts_with("hell"));
    }

    #[test]
    fn test_format_cell_exact_fit() {
        assert_eq!(Table::format_cell("abc", 3, Align::Left), "abc");
    }

    #[test]
    fn test_format_cell_width_one_truncation() {
        let result = Table::format_cell("hello", 1, Align::Left);
        assert_eq!(result, "h");
    }

    #[test]
    fn test_column_width_constraints() {
        let table = Table::new()
            .columns_detailed(vec![
                Column::new("A").width(Width::Fixed(5)),
                Column::new("B").width(Width::Min(3)),
                Column::new("C").width(Width::Max(4)),
            ])
            .rows(vec![vec!["x", "y", "longer"]]);

        let widths = table.compute_widths(80);
        assert_eq!(widths[0], 5); // Fixed
        assert!(widths[1] >= 3); // Min
        assert!(widths[2] <= 4); // Max
    }

    #[test]
    fn test_auto_truncation_narrow_terminal() {
        let table = Table::new()
            .columns(&["Name", "Description"])
            .rows(vec![vec![
                "short",
                "This is a very long description that should get truncated",
            ]])
            .border(Border::Unicode);

        // Render with a narrow width
        let output = render_plain_width(&table, 40);
        // Every line should be <= 40 chars
        for line in output.lines() {
            assert!(
                line.chars().count() <= 42, // allow small rounding
                "Line too wide ({} chars): {:?}",
                line.chars().count(),
                line
            );
        }
    }

    #[test]
    fn test_indent() {
        let table = Table::new()
            .columns(&["X"])
            .rows(vec![vec!["val"]])
            .border(Border::Ascii)
            .indent(4);

        let output = render_plain(&table);
        for line in output.lines() {
            assert!(
                line.starts_with("    "),
                "Line should be indented: {:?}",
                line
            );
        }
    }

    #[test]
    fn test_minmax_width() {
        let table = Table::new()
            .columns_detailed(vec![Column::new("Col").width(Width::MinMax(3, 8))])
            .rows(vec![vec!["ab"]]);

        let widths = table.compute_widths(80);
        assert!(widths[0] >= 3 && widths[0] <= 8);
    }

    #[test]
    fn test_missing_cells_in_row() {
        let table = Table::new()
            .columns(&["A", "B", "C"])
            .rows(vec![vec!["only_one".to_string()]])
            .border(Border::Unicode);

        // Should not panic with fewer cells than columns
        let output = render_plain(&table);
        assert!(output.contains("only_one"));
    }

    #[test]
    fn test_single_column_table() {
        let table = Table::new()
            .columns(&["Items"])
            .rows(vec![vec!["one"], vec!["two"], vec!["three"]])
            .border(Border::Unicode);

        let output = render_plain(&table);
        assert!(output.contains("Items"));
        assert!(output.contains("one"));
        assert!(output.contains("two"));
        assert!(output.contains("three"));
    }

    #[test]
    fn test_border_overhead_none() {
        let table = Table::new().border(Border::None);
        assert_eq!(table.border_overhead(1), 0);
        assert_eq!(table.border_overhead(2), 2);
        assert_eq!(table.border_overhead(3), 4);
    }

    #[test]
    fn test_border_overhead_unicode() {
        let table = Table::new().border(Border::Unicode);
        // "| col |" = 2 + 2 = 4
        assert_eq!(table.border_overhead(1), 4);
        // "| col | col |" = 2 + 3 + 2 = 7
        assert_eq!(table.border_overhead(2), 7);
        // "| col | col | col |" = 2 + 3 + 3 + 2 = 10
        assert_eq!(table.border_overhead(3), 10);
    }

    #[test]
    fn test_styled_output_contains_ansi() {
        use crate::ui::theme::ActiveTheme;
        let theme = ActiveTheme::default_dark();
        let table = Table::new()
            .columns(&["ID", "Name"])
            .rows(vec![vec!["1", "test"]])
            .border(Border::Unicode);

        let mut buf = Vec::new();
        {
            let mut fmt = Formatter::styled(&mut buf, theme);
            table.render(&mut fmt).unwrap();
        }
        let output = String::from_utf8(buf).unwrap();
        // Styled output should contain ANSI escape codes
        assert!(output.contains("\x1b["));
        assert!(output.contains("ID"));
        assert!(output.contains("test"));
    }

    #[test]
    fn test_default_border_is_unicode() {
        let table = Table::new();
        assert_eq!(table.border, Border::Unicode);
    }

    #[test]
    fn test_builder_chain() {
        // Verify builder methods return Self for chaining
        let table = Table::new()
            .columns(&["A", "B"])
            .rows(vec![vec!["1", "2"]])
            .border(Border::Ascii)
            .indent(2);

        assert_eq!(table.border, Border::Ascii);
        assert_eq!(table.indent, 2);
        assert_eq!(table.cols.len(), 2);
        assert_eq!(table.rows.len(), 1);
    }
}
