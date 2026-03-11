//! Screen buffer implementation
//!
//! The ScreenBuffer represents the current state of the terminal display,
//! including the grid of cells, cursor position, and terminal modes.

use crate::screen::cell::{Cell, CellAttrs, Color};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Terminal size in columns and rows
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl TermSize {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { cols, rows }
    }
}

impl Default for TermSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24 }
    }
}

/// Cursor position (0-indexed)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CursorPos {
    pub row: u16,
    pub col: u16,
}

impl CursorPos {
    pub fn new(row: u16, col: u16) -> Self {
        Self { row, col }
    }
}

/// Metadata about a screen frame
#[derive(Clone, Debug)]
pub struct FrameMetadata {
    /// Monotonic frame number
    pub frame_id: u64,
    /// Capture timestamp
    pub timestamp: Instant,
    /// Total bytes processed to reach this state
    pub bytes_processed: usize,
}

impl Default for FrameMetadata {
    fn default() -> Self {
        Self {
            frame_id: 0,
            timestamp: Instant::now(),
            bytes_processed: 0,
        }
    }
}

/// Current pen state for writing to the buffer
#[derive(Clone, Debug, Default)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

impl Pen {
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Terminal screen buffer with full cell state
#[derive(Clone, Debug)]
pub struct ScreenBuffer {
    /// Grid of cells (row-major order)
    cells: Vec<Vec<Cell>>,
    /// Terminal dimensions
    size: TermSize,
    /// Current cursor position
    cursor: CursorPos,
    /// Cursor visibility
    cursor_visible: bool,
    /// Current pen (colors/attributes for new characters)
    pen: Pen,
    /// Frame metadata
    metadata: FrameMetadata,
    /// Scroll region (top, bottom) - 0-indexed, inclusive
    scroll_region: Option<(u16, u16)>,
    /// Saved cursor position (for save/restore)
    saved_cursor: Option<CursorPos>,
    /// Tab stops
    tab_stops: Vec<u16>,
}

impl ScreenBuffer {
    /// Create a new screen buffer with the given size
    pub fn new(cols: u16, rows: u16) -> Self {
        let size = TermSize::new(cols, rows);
        let cells = (0..rows)
            .map(|_| (0..cols).map(|_| Cell::default()).collect())
            .collect();

        // Default tab stops every 8 columns
        let tab_stops = (0..cols).filter(|c| c % 8 == 0).collect();

        Self {
            cells,
            size,
            cursor: CursorPos::default(),
            cursor_visible: true,
            pen: Pen::default(),
            metadata: FrameMetadata::default(),
            scroll_region: None,
            saved_cursor: None,
            tab_stops,
        }
    }

    /// Get the terminal size
    pub fn size(&self) -> TermSize {
        self.size
    }

    /// Get the current cursor position
    pub fn cursor(&self) -> CursorPos {
        self.cursor
    }

    /// Check if cursor is visible
    pub fn cursor_visible(&self) -> bool {
        self.cursor_visible
    }

    /// Set cursor visibility
    pub fn set_cursor_visible(&mut self, visible: bool) {
        self.cursor_visible = visible;
    }

    /// Get a reference to a cell
    pub fn get_cell(&self, row: u16, col: u16) -> Option<&Cell> {
        self.cells
            .get(row as usize)
            .and_then(|r| r.get(col as usize))
    }

    /// Get a mutable reference to a cell
    pub fn get_cell_mut(&mut self, row: u16, col: u16) -> Option<&mut Cell> {
        self.cells
            .get_mut(row as usize)
            .and_then(|r| r.get_mut(col as usize))
    }

    /// Get a row of cells
    pub fn get_row(&self, row: u16) -> Option<&[Cell]> {
        self.cells.get(row as usize).map(|r| r.as_slice())
    }

    /// Get the text content of a row (trimming trailing spaces)
    pub fn row_text(&self, row: u16) -> String {
        self.get_row(row)
            .map(|cells| {
                let text: String = cells.iter().map(|c| c.grapheme.as_str()).collect();
                text.trim_end().to_string()
            })
            .unwrap_or_default()
    }

    /// Get all text content as lines
    pub fn text_lines(&self) -> Vec<String> {
        (0..self.size.rows).map(|r| self.row_text(r)).collect()
    }

    /// Get text content as a single string
    pub fn text(&self) -> String {
        self.text_lines().join("\n")
    }

    /// Check if the buffer contains text anywhere
    pub fn contains_text(&self, needle: &str) -> bool {
        self.text().contains(needle)
    }

    /// Check if a specific row contains text
    pub fn row_contains(&self, row: u16, needle: &str) -> bool {
        self.row_text(row).contains(needle)
    }

    /// Move cursor to absolute position (clamped to bounds)
    pub fn move_cursor_to(&mut self, row: u16, col: u16) {
        self.cursor.row = row.min(self.size.rows.saturating_sub(1));
        self.cursor.col = col.min(self.size.cols.saturating_sub(1));
    }

    /// Move cursor relative to current position
    pub fn move_cursor_by(&mut self, row_delta: i16, col_delta: i16) {
        let new_row = (self.cursor.row as i16 + row_delta)
            .max(0)
            .min(self.size.rows as i16 - 1) as u16;
        let new_col = (self.cursor.col as i16 + col_delta)
            .max(0)
            .min(self.size.cols as i16 - 1) as u16;
        self.cursor.row = new_row;
        self.cursor.col = new_col;
    }

    /// Move cursor to beginning of line
    pub fn carriage_return(&mut self) {
        self.cursor.col = 0;
    }

    /// Move cursor down, scrolling if at bottom
    pub fn linefeed(&mut self) {
        let bottom = self
            .scroll_region
            .map(|(_, b)| b)
            .unwrap_or(self.size.rows - 1);

        if self.cursor.row >= bottom {
            self.scroll_up(1);
        } else {
            self.cursor.row += 1;
        }
    }

    /// Move to next tab stop
    pub fn tab(&mut self) {
        let next_tab = self
            .tab_stops
            .iter()
            .find(|&&t| t > self.cursor.col)
            .copied()
            .unwrap_or(self.size.cols - 1);
        self.cursor.col = next_tab.min(self.size.cols - 1);
    }

    /// Write a character at cursor position and advance
    pub fn put_char(&mut self, c: char) {
        let cell = Cell::with_char(c)
            .fg(self.pen.fg)
            .bg(self.pen.bg)
            .attrs(self.pen.attrs);
        let width = cell.width.max(1) as u16;

        // If wide char would overflow, wrap first
        if width > 1 && self.cursor.col >= self.size.cols.saturating_sub(1) {
            self.carriage_return();
            self.linefeed();
        }

        if self.cursor.col >= self.size.cols {
            // Wrap to next line
            self.carriage_return();
            self.linefeed();
        }

        if let Some(target) = self.get_cell_mut(self.cursor.row, self.cursor.col) {
            *target = cell;
        }

        if width > 1 && self.cursor.col + 1 < self.size.cols {
            let continuation = Cell {
                grapheme: String::new(),
                width: 0,
                fg: self.pen.fg,
                bg: self.pen.bg,
                attrs: self.pen.attrs,
            };
            if let Some(next) = self.get_cell_mut(self.cursor.row, self.cursor.col + 1) {
                *next = continuation;
            }
        }

        self.cursor.col = self.cursor.col.saturating_add(width);
    }

    /// Write a string at cursor position
    pub fn put_str(&mut self, s: &str) {
        for c in s.chars() {
            self.put_char(c);
        }
    }

    /// Clear the entire screen
    pub fn clear(&mut self) {
        for row in &mut self.cells {
            for cell in row {
                *cell = Cell::default();
            }
        }
    }

    /// Clear from cursor to end of screen
    pub fn clear_to_end(&mut self) {
        // Clear rest of current line
        self.clear_line_from_cursor();
        // Clear all lines below
        for row in (self.cursor.row + 1)..self.size.rows {
            if let Some(line) = self.cells.get_mut(row as usize) {
                for cell in line {
                    *cell = Cell::default();
                }
            }
        }
    }

    /// Clear from start of screen to cursor
    pub fn clear_to_start(&mut self) {
        // Clear all lines above
        for row in 0..self.cursor.row {
            if let Some(line) = self.cells.get_mut(row as usize) {
                for cell in line {
                    *cell = Cell::default();
                }
            }
        }
        // Clear current line up to and including cursor
        self.clear_line_to_cursor();
    }

    /// Clear entire current line
    pub fn clear_line(&mut self) {
        if let Some(line) = self.cells.get_mut(self.cursor.row as usize) {
            for cell in line {
                *cell = Cell::default();
            }
        }
    }

    /// Clear from cursor to end of line
    pub fn clear_line_from_cursor(&mut self) {
        if let Some(line) = self.cells.get_mut(self.cursor.row as usize) {
            for cell in line.iter_mut().skip(self.cursor.col as usize) {
                *cell = Cell::default();
            }
        }
    }

    /// Clear from start of line to cursor
    pub fn clear_line_to_cursor(&mut self) {
        if let Some(line) = self.cells.get_mut(self.cursor.row as usize) {
            for cell in line.iter_mut().take(self.cursor.col as usize + 1) {
                *cell = Cell::default();
            }
        }
    }

    /// Scroll the screen up by n lines (content moves up, new blank lines at bottom)
    pub fn scroll_up(&mut self, n: u16) {
        let (top, bottom) = self
            .scroll_region
            .unwrap_or((0, self.size.rows.saturating_sub(1)));
        let top = top as usize;
        let bottom = bottom as usize;
        if top >= self.cells.len() || bottom >= self.cells.len() || top >= bottom {
            return;
        }

        let region_len = bottom - top + 1;
        let n = (n as usize).min(region_len);
        if n == 0 {
            return;
        }

        self.cells[top..=bottom].rotate_left(n);
        let blank_row: Vec<Cell> = (0..self.size.cols).map(|_| Cell::default()).collect();
        for i in 0..n {
            self.cells[bottom - i] = blank_row.clone();
        }
    }

    /// Scroll the screen down by n lines (content moves down, new blank lines at top)
    pub fn scroll_down(&mut self, n: u16) {
        let (top, bottom) = self
            .scroll_region
            .unwrap_or((0, self.size.rows.saturating_sub(1)));
        let top = top as usize;
        let bottom = bottom as usize;
        if top >= self.cells.len() || bottom >= self.cells.len() || top >= bottom {
            return;
        }

        let region_len = bottom - top + 1;
        let n = (n as usize).min(region_len);
        if n == 0 {
            return;
        }

        self.cells[top..=bottom].rotate_right(n);
        let blank_row: Vec<Cell> = (0..self.size.cols).map(|_| Cell::default()).collect();
        for i in 0..n {
            self.cells[top + i] = blank_row.clone();
        }
    }

    /// Set scroll region (top and bottom are 0-indexed, inclusive)
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        if top < bottom && bottom < self.size.rows {
            self.scroll_region = Some((top, bottom));
        }
    }

    /// Clear scroll region (use full screen)
    pub fn clear_scroll_region(&mut self) {
        self.scroll_region = None;
    }

    /// Save cursor position
    pub fn save_cursor(&mut self) {
        self.saved_cursor = Some(self.cursor);
    }

    /// Restore cursor position
    pub fn restore_cursor(&mut self) {
        if let Some(pos) = self.saved_cursor {
            self.cursor = pos;
        }
    }

    /// Get the current pen (colors/attributes)
    pub fn pen(&self) -> &Pen {
        &self.pen
    }

    /// Get mutable reference to pen
    pub fn pen_mut(&mut self) -> &mut Pen {
        &mut self.pen
    }

    /// Reset pen to default
    pub fn reset_pen(&mut self) {
        self.pen.reset();
    }

    /// Set foreground color
    pub fn set_fg(&mut self, color: Color) {
        self.pen.fg = color;
    }

    /// Set background color
    pub fn set_bg(&mut self, color: Color) {
        self.pen.bg = color;
    }

    /// Set attribute
    pub fn set_attr(&mut self, attr: Attr) {
        match attr {
            Attr::Bold => self.pen.attrs.bold = true,
            Attr::Dim => self.pen.attrs.dim = true,
            Attr::Italic => self.pen.attrs.italic = true,
            Attr::Underline => self.pen.attrs.underline = true,
            Attr::Blink => self.pen.attrs.blink = true,
            Attr::Reverse => self.pen.attrs.reverse = true,
            Attr::Hidden => self.pen.attrs.hidden = true,
            Attr::Strikethrough => self.pen.attrs.strikethrough = true,
            Attr::NoBold => self.pen.attrs.bold = false,
            Attr::NoDim => self.pen.attrs.dim = false,
            Attr::NoItalic => self.pen.attrs.italic = false,
            Attr::NoUnderline => self.pen.attrs.underline = false,
            Attr::NoBlink => self.pen.attrs.blink = false,
            Attr::NoReverse => self.pen.attrs.reverse = false,
            Attr::NoHidden => self.pen.attrs.hidden = false,
            Attr::NoStrikethrough => self.pen.attrs.strikethrough = false,
        }
    }

    /// Update frame metadata
    pub fn update_metadata(&mut self, bytes_processed: usize) {
        self.metadata.frame_id += 1;
        self.metadata.timestamp = Instant::now();
        self.metadata.bytes_processed += bytes_processed;
    }

    /// Get frame metadata
    pub fn metadata(&self) -> &FrameMetadata {
        &self.metadata
    }

    /// Resize the buffer
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let new_size = TermSize::new(cols, rows);

        // Resize rows
        while self.cells.len() < rows as usize {
            self.cells
                .push((0..cols).map(|_| Cell::default()).collect());
        }
        self.cells.truncate(rows as usize);

        // Resize columns in each row
        for row in &mut self.cells {
            while row.len() < cols as usize {
                row.push(Cell::default());
            }
            row.truncate(cols as usize);
        }

        self.size = new_size;

        // Clamp cursor to new bounds
        self.cursor.row = self.cursor.row.min(rows.saturating_sub(1));
        self.cursor.col = self.cursor.col.min(cols.saturating_sub(1));

        // Update tab stops
        self.tab_stops = (0..cols).filter(|c| c % 8 == 0).collect();
    }
}

/// Text attributes that can be set/unset
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attr {
    Bold,
    Dim,
    Italic,
    Underline,
    Blink,
    Reverse,
    Hidden,
    Strikethrough,
    NoBold,
    NoDim,
    NoItalic,
    NoUnderline,
    NoBlink,
    NoReverse,
    NoHidden,
    NoStrikethrough,
}

#[cfg(test)]
mod tests {
    use crate::screen::buffer::*;

    #[test]
    fn test_new_buffer() {
        let buf = ScreenBuffer::new(80, 24);
        assert_eq!(buf.size().cols, 80);
        assert_eq!(buf.size().rows, 24);
        assert_eq!(buf.cursor(), CursorPos::new(0, 0));
        assert!(buf.cursor_visible());
    }

    #[test]
    fn test_put_char() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_char('H');
        buf.put_char('i');

        assert_eq!(buf.cursor().col, 2);
        assert_eq!(buf.row_text(0), "Hi");
    }

    #[test]
    fn test_put_str() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_str("Hello");

        assert_eq!(buf.row_text(0), "Hello");
        assert_eq!(buf.cursor().col, 5);
    }

    #[test]
    fn test_put_wide_char() {
        let mut buf = ScreenBuffer::new(10, 2);
        buf.put_char('界');

        let cell = buf.get_cell(0, 0).unwrap();
        let cont = buf.get_cell(0, 1).unwrap();

        assert_eq!(cell.grapheme, "界");
        assert_eq!(cell.width, 2);
        assert_eq!(cont.width, 0);
    }

    #[test]
    fn test_cursor_movement() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.move_cursor_to(5, 10);
        assert_eq!(buf.cursor(), CursorPos::new(5, 10));

        buf.move_cursor_by(-2, 3);
        assert_eq!(buf.cursor(), CursorPos::new(3, 13));

        // Clamp to bounds
        buf.move_cursor_to(100, 100);
        assert_eq!(buf.cursor(), CursorPos::new(23, 79));
    }

    #[test]
    fn test_clear_line() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_str("Hello World");
        buf.move_cursor_to(0, 5);
        buf.clear_line_from_cursor();

        assert_eq!(buf.row_text(0), "Hello");
    }

    #[test]
    fn test_linefeed_scroll() {
        let mut buf = ScreenBuffer::new(80, 3);
        buf.move_cursor_to(0, 0);
        buf.put_str("Line 1");
        buf.move_cursor_to(1, 0);
        buf.put_str("Line 2");
        buf.move_cursor_to(2, 0);
        buf.put_str("Line 3");

        // At bottom, linefeed should scroll
        buf.linefeed();
        assert_eq!(buf.row_text(0), "Line 2");
        assert_eq!(buf.row_text(1), "Line 3");
        assert_eq!(buf.row_text(2), "");
    }

    #[test]
    fn test_contains_text() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.put_str("Hello World");
        assert!(buf.contains_text("World"));
        assert!(!buf.contains_text("Foo"));
    }

    #[test]
    fn test_resize() {
        let mut buf = ScreenBuffer::new(80, 24);
        buf.move_cursor_to(20, 70);
        buf.put_str("Test");

        buf.resize(40, 10);
        assert_eq!(buf.size(), TermSize::new(40, 10));
        // Cursor should be clamped
        assert_eq!(buf.cursor(), CursorPos::new(9, 39));
    }
}
