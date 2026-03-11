//! Table rendering with Unicode box drawing

use pulldown_cmark::Alignment;
use ratatui::text::{Line, Span};

use crate::ui::theme::ActiveTheme;

/// Builder for markdown tables
pub struct TableBuilder {
    alignments: Vec<Alignment>,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    current_row: Vec<String>,
    current_cell: String,
}

impl TableBuilder {
    pub fn new(alignments: Vec<Alignment>) -> Self {
        Self {
            alignments,
            headers: Vec::new(),
            rows: Vec::new(),
            current_row: Vec::new(),
            current_cell: String::new(),
        }
    }

    pub fn add_cell_text(&mut self, text: &str) {
        self.current_cell.push_str(text);
    }

    pub fn end_cell(&mut self) {
        self.current_row
            .push(std::mem::take(&mut self.current_cell));
    }

    pub fn start_row(&mut self) {
        self.current_row = Vec::new();
    }

    pub fn end_row(&mut self, is_header: bool) {
        let row = std::mem::take(&mut self.current_row);
        if is_header {
            self.headers = row;
        } else {
            self.rows.push(row);
        }
    }

    pub fn render(self, theme: &ActiveTheme) -> Vec<Line<'static>> {
        let mut lines = Vec::new();

        if self.headers.is_empty() && self.rows.is_empty() {
            return lines;
        }

        // Calculate column widths
        let col_count = self
            .headers
            .len()
            .max(self.rows.iter().map(|r| r.len()).max().unwrap_or(0));

        let mut col_widths: Vec<usize> = vec![0; col_count];

        // Measure headers
        for (i, header) in self.headers.iter().enumerate() {
            col_widths[i] = col_widths[i].max(header.len());
        }

        // Measure rows
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate() {
                if i < col_widths.len() {
                    col_widths[i] = col_widths[i].max(cell.len());
                }
            }
        }

        // Ensure minimum column width
        for width in &mut col_widths {
            *width = (*width).max(3);
        }

        let border_style = theme.styles.border_muted;
        let header_style = theme.styles.text_bold;
        let cell_style = theme.styles.text_primary;

        // Top border: ┌───┬───┐
        lines.push(self.render_border_line(&col_widths, '┌', '┬', '┐', border_style));

        // Header row
        if !self.headers.is_empty() {
            lines.push(self.render_data_row(
                &self.headers,
                &col_widths,
                &self.alignments,
                header_style,
                border_style,
            ));

            // Header separator: ├───┼───┤
            lines.push(self.render_border_line(&col_widths, '├', '┼', '┤', border_style));
        }

        // Data rows
        for row in &self.rows {
            lines.push(self.render_data_row(
                row,
                &col_widths,
                &self.alignments,
                cell_style,
                border_style,
            ));
        }

        // Bottom border: └───┴───┘
        lines.push(self.render_border_line(&col_widths, '└', '┴', '┘', border_style));

        lines
    }

    fn render_border_line(
        &self,
        col_widths: &[usize],
        left: char,
        mid: char,
        right: char,
        style: ratatui::style::Style,
    ) -> Line<'static> {
        let mut s = String::new();
        s.push(left);

        for (i, &width) in col_widths.iter().enumerate() {
            s.push_str(&"─".repeat(width + 2)); // +2 for padding
            if i < col_widths.len() - 1 {
                s.push(mid);
            }
        }

        s.push(right);
        Line::from(Span::styled(s, style))
    }

    fn render_data_row(
        &self,
        cells: &[String],
        col_widths: &[usize],
        alignments: &[Alignment],
        cell_style: ratatui::style::Style,
        border_style: ratatui::style::Style,
    ) -> Line<'static> {
        let mut spans = Vec::new();

        spans.push(Span::styled("│", border_style));

        for (i, width) in col_widths.iter().enumerate() {
            let cell_text = cells.get(i).map(|s| s.as_str()).unwrap_or("");
            let alignment = alignments.get(i).copied().unwrap_or(Alignment::None);

            let padded = pad_cell(cell_text, *width, alignment);
            spans.push(Span::styled(format!(" {padded} "), cell_style));
            spans.push(Span::styled("│", border_style));
        }

        Line::from(spans)
    }
}

fn pad_cell(text: &str, width: usize, alignment: Alignment) -> String {
    let text_len = text.len();
    if text_len >= width {
        return text.to_string();
    }

    let padding = width - text_len;

    match alignment {
        Alignment::Left | Alignment::None => format!("{}{}", text, " ".repeat(padding)),
        Alignment::Right => format!("{}{}", " ".repeat(padding), text),
        Alignment::Center => {
            let left_pad = padding / 2;
            let right_pad = padding - left_pad;
            format!("{}{}{}", " ".repeat(left_pad), text, " ".repeat(right_pad))
        }
    }
}
