//! Buffer-based ratatui backend for PTY forwarding
//!
//! This backend captures terminal output to a buffer instead of writing
//! directly to a terminal. The buffer contents can then be sent to clients.

use crossterm::{
    Command,
    cursor::{Hide, MoveTo, Show},
    style::{
        Attribute, Color as CrosstermColor, Print, SetAttribute, SetBackgroundColor,
        SetForegroundColor,
    },
    terminal::{Clear, ClearType},
};
use ratatui::backend::ClearType as RatatuiClearType;
use ratatui::backend::{Backend, WindowSize};
use ratatui::buffer::Cell;
use ratatui::layout::{Position, Size};
use ratatui::style::{Color, Modifier};
use std::io;

/// A ratatui backend that writes to a buffer
pub struct BufferBackend {
    /// Output buffer
    buffer: Vec<u8>,
    /// Terminal size
    width: u16,
    height: u16,
}

impl BufferBackend {
    /// Create a new buffer backend with the given size
    pub fn new(width: u16, height: u16) -> Self {
        Self {
            buffer: Vec::with_capacity(16384),
            width,
            height,
        }
    }

    /// Take the buffered output, leaving an empty buffer
    pub fn take_buffer(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Resize the backend
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
    }

    /// Write a crossterm command to the buffer
    fn write_command(&mut self, cmd: impl Command) -> io::Result<()> {
        let mut tmp = Vec::new();
        crossterm::execute!(tmp, cmd)?;
        self.buffer.extend_from_slice(&tmp);
        Ok(())
    }

    /// Convert ratatui color to crossterm color
    fn to_crossterm_color(color: Color) -> CrosstermColor {
        match color {
            Color::Reset => CrosstermColor::Reset,
            Color::Black => CrosstermColor::Black,
            Color::Red => CrosstermColor::DarkRed,
            Color::Green => CrosstermColor::DarkGreen,
            Color::Yellow => CrosstermColor::DarkYellow,
            Color::Blue => CrosstermColor::DarkBlue,
            Color::Magenta => CrosstermColor::DarkMagenta,
            Color::Cyan => CrosstermColor::DarkCyan,
            Color::Gray => CrosstermColor::Grey,
            Color::DarkGray => CrosstermColor::DarkGrey,
            Color::LightRed => CrosstermColor::Red,
            Color::LightGreen => CrosstermColor::Green,
            Color::LightYellow => CrosstermColor::Yellow,
            Color::LightBlue => CrosstermColor::Blue,
            Color::LightMagenta => CrosstermColor::Magenta,
            Color::LightCyan => CrosstermColor::Cyan,
            Color::White => CrosstermColor::White,
            Color::Rgb(r, g, b) => CrosstermColor::Rgb { r, g, b },
            Color::Indexed(i) => CrosstermColor::AnsiValue(i),
        }
    }
}

impl Backend for BufferBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a Cell)>,
    {
        let mut last_pos: Option<(u16, u16)> = None;
        let mut last_fg = Color::Reset;
        let mut last_bg = Color::Reset;
        let mut last_modifier = Modifier::empty();

        for (x, y, cell) in content {
            // Move cursor if not contiguous
            if last_pos != Some((x.saturating_sub(1), y)) {
                self.write_command(MoveTo(x, y))?;
            }
            last_pos = Some((x, y));

            // Update foreground color if changed
            if cell.fg != last_fg {
                self.write_command(SetForegroundColor(Self::to_crossterm_color(cell.fg)))?;
                last_fg = cell.fg;
            }

            // Update background color if changed
            if cell.bg != last_bg {
                self.write_command(SetBackgroundColor(Self::to_crossterm_color(cell.bg)))?;
                last_bg = cell.bg;
            }

            // Update modifiers if changed
            if cell.modifier != last_modifier {
                // Reset all attributes first
                self.write_command(SetAttribute(Attribute::Reset))?;

                // Reapply colors after reset
                if last_fg != Color::Reset {
                    self.write_command(SetForegroundColor(Self::to_crossterm_color(last_fg)))?;
                }
                if last_bg != Color::Reset {
                    self.write_command(SetBackgroundColor(Self::to_crossterm_color(last_bg)))?;
                }

                // Apply new modifiers
                if cell.modifier.contains(Modifier::BOLD) {
                    self.write_command(SetAttribute(Attribute::Bold))?;
                }
                if cell.modifier.contains(Modifier::DIM) {
                    self.write_command(SetAttribute(Attribute::Dim))?;
                }
                if cell.modifier.contains(Modifier::ITALIC) {
                    self.write_command(SetAttribute(Attribute::Italic))?;
                }
                if cell.modifier.contains(Modifier::UNDERLINED) {
                    self.write_command(SetAttribute(Attribute::Underlined))?;
                }
                if cell.modifier.contains(Modifier::REVERSED) {
                    self.write_command(SetAttribute(Attribute::Reverse))?;
                }
                if cell.modifier.contains(Modifier::CROSSED_OUT) {
                    self.write_command(SetAttribute(Attribute::CrossedOut))?;
                }
                last_modifier = cell.modifier;
            }

            // Write the cell content
            self.write_command(Print(cell.symbol()))?;
        }

        // Reset attributes at the end
        self.write_command(SetAttribute(Attribute::Reset))?;

        Ok(())
    }

    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.write_command(Hide)
    }

    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.write_command(Show)
    }

    fn get_cursor_position(&mut self) -> Result<Position, Self::Error> {
        Ok(Position::new(0, 0))
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> Result<(), Self::Error> {
        let pos = position.into();
        self.write_command(MoveTo(pos.x, pos.y))
    }

    fn clear(&mut self) -> Result<(), Self::Error> {
        self.write_command(Clear(ClearType::All))
    }

    fn clear_region(&mut self, clear_type: RatatuiClearType) -> Result<(), Self::Error> {
        let crossterm_clear = match clear_type {
            RatatuiClearType::All => ClearType::All,
            RatatuiClearType::AfterCursor => ClearType::FromCursorDown,
            RatatuiClearType::BeforeCursor => ClearType::FromCursorUp,
            RatatuiClearType::CurrentLine => ClearType::CurrentLine,
            RatatuiClearType::UntilNewLine => ClearType::UntilNewLine,
        };
        self.write_command(Clear(crossterm_clear))
    }

    fn size(&self) -> Result<Size, Self::Error> {
        Ok(Size::new(self.width, self.height))
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn window_size(&mut self) -> Result<WindowSize, Self::Error> {
        // Return window size with cell/pixel dimensions (not really used, but required by trait)
        Ok(WindowSize {
            columns_rows: Size::new(self.width, self.height),
            pixels: Size::new(self.width * 8, self.height * 16),
        })
    }
}
