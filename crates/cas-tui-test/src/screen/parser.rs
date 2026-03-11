//! VT sequence parser using the `vte` crate
//!
//! Parses terminal escape sequences and updates the screen buffer accordingly.

use crate::screen::buffer::{Attr, ScreenBuffer};
use crate::screen::cell::Color;
use vte::{Params, Parser, Perform};

/// Parser that processes VT sequences and updates a screen buffer
pub struct VtParser {
    parser: Parser,
    buffer: ScreenBuffer,
}

impl VtParser {
    /// Create a new parser with the given terminal size
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            parser: Parser::new(),
            buffer: ScreenBuffer::new(cols, rows),
        }
    }

    /// Create a parser from an existing buffer
    pub fn with_buffer(buffer: ScreenBuffer) -> Self {
        Self {
            parser: Parser::new(),
            buffer,
        }
    }

    /// Process bytes through the parser
    pub fn process(&mut self, bytes: &[u8]) {
        let mut performer = BufferPerformer {
            buffer: &mut self.buffer,
        };
        self.parser.advance(&mut performer, bytes);
        self.buffer.update_metadata(bytes.len());
    }

    /// Get a reference to the screen buffer
    pub fn buffer(&self) -> &ScreenBuffer {
        &self.buffer
    }

    /// Get a mutable reference to the screen buffer
    pub fn buffer_mut(&mut self) -> &mut ScreenBuffer {
        &mut self.buffer
    }

    /// Take ownership of the buffer
    pub fn into_buffer(self) -> ScreenBuffer {
        self.buffer
    }

    /// Reset the parser and clear the buffer
    pub fn reset(&mut self) {
        self.parser = Parser::new();
        self.buffer.clear();
        self.buffer.move_cursor_to(0, 0);
        self.buffer.reset_pen();
    }
}

/// Performer that applies VT sequences to the buffer
struct BufferPerformer<'a> {
    buffer: &'a mut ScreenBuffer,
}

impl<'a> Perform for BufferPerformer<'a> {
    fn print(&mut self, c: char) {
        self.buffer.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            // Bell
            0x07 => {}
            // Backspace
            0x08 => {
                self.buffer.move_cursor_by(0, -1);
            }
            // Horizontal tab
            0x09 => {
                self.buffer.tab();
            }
            // Line feed / New line / Vertical tab
            0x0A..=0x0C => {
                self.buffer.linefeed();
            }
            // Carriage return
            0x0D => {
                self.buffer.carriage_return();
            }
            _ => {}
        }
    }

    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        // DCS sequences - not commonly needed for basic TUI testing
    }

    fn put(&mut self, _byte: u8) {
        // DCS data - not commonly needed
    }

    fn unhook(&mut self) {
        // End DCS
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // OSC sequences (window title, etc.) - ignore for now
    }

    fn csi_dispatch(
        &mut self,
        params: &Params,
        _intermediates: &[u8],
        _ignore: bool,
        action: char,
    ) {
        let params: Vec<u16> = params.iter().map(|p| p[0]).collect();

        match action {
            // Cursor movement
            'A' => {
                // CUU - Cursor Up
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(-(n as i16), 0);
            }
            'B' => {
                // CUD - Cursor Down
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(n as i16, 0);
            }
            'C' => {
                // CUF - Cursor Forward
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(0, n as i16);
            }
            'D' => {
                // CUB - Cursor Back
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(0, -(n as i16));
            }
            'E' => {
                // CNL - Cursor Next Line
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(n as i16, 0);
                self.buffer.carriage_return();
            }
            'F' => {
                // CPL - Cursor Previous Line
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.move_cursor_by(-(n as i16), 0);
                self.buffer.carriage_return();
            }
            'G' => {
                // CHA - Cursor Horizontal Absolute
                let col = params.first().copied().unwrap_or(1).saturating_sub(1);
                let row = self.buffer.cursor().row;
                self.buffer.move_cursor_to(row, col);
            }
            'H' | 'f' => {
                // CUP/HVP - Cursor Position
                let row = params.first().copied().unwrap_or(1).saturating_sub(1);
                let col = params.get(1).copied().unwrap_or(1).saturating_sub(1);
                self.buffer.move_cursor_to(row, col);
            }
            'J' => {
                // ED - Erase in Display
                match params.first().copied().unwrap_or(0) {
                    0 => self.buffer.clear_to_end(),
                    1 => self.buffer.clear_to_start(),
                    2 | 3 => self.buffer.clear(),
                    _ => {}
                }
            }
            'K' => {
                // EL - Erase in Line
                match params.first().copied().unwrap_or(0) {
                    0 => self.buffer.clear_line_from_cursor(),
                    1 => self.buffer.clear_line_to_cursor(),
                    2 => self.buffer.clear_line(),
                    _ => {}
                }
            }
            'S' => {
                // SU - Scroll Up
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.scroll_up(n);
            }
            'T' => {
                // SD - Scroll Down
                let n = params.first().copied().unwrap_or(1).max(1);
                self.buffer.scroll_down(n);
            }
            'd' => {
                // VPA - Vertical Position Absolute
                let row = params.first().copied().unwrap_or(1).saturating_sub(1);
                let col = self.buffer.cursor().col;
                self.buffer.move_cursor_to(row, col);
            }
            'm' => {
                // SGR - Select Graphic Rendition
                self.handle_sgr(&params);
            }
            'r' => {
                // DECSTBM - Set Top and Bottom Margins
                if params.len() >= 2 {
                    let top = params[0].saturating_sub(1);
                    let bottom = params[1].saturating_sub(1);
                    self.buffer.set_scroll_region(top, bottom);
                } else {
                    self.buffer.clear_scroll_region();
                }
            }
            's' => {
                // SCOSC - Save Cursor Position
                self.buffer.save_cursor();
            }
            'u' => {
                // SCORC - Restore Cursor Position
                self.buffer.restore_cursor();
            }
            'h' => {
                // SM - Set Mode
                // Check for cursor visibility (DECTCEM)
                if params.first() == Some(&25) {
                    self.buffer.set_cursor_visible(true);
                }
            }
            'l' => {
                // RM - Reset Mode
                // Check for cursor visibility (DECTCEM)
                if params.first() == Some(&25) {
                    self.buffer.set_cursor_visible(false);
                }
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, _intermediates: &[u8], _ignore: bool, byte: u8) {
        match byte {
            // RIS - Reset to Initial State
            b'c' => {
                self.buffer.clear();
                self.buffer.move_cursor_to(0, 0);
                self.buffer.reset_pen();
            }
            // Save cursor (DECSC)
            b'7' => {
                self.buffer.save_cursor();
            }
            // Restore cursor (DECRC)
            b'8' => {
                self.buffer.restore_cursor();
            }
            // Index (move down, scroll if at bottom)
            b'D' => {
                self.buffer.linefeed();
            }
            // Next Line
            b'E' => {
                self.buffer.carriage_return();
                self.buffer.linefeed();
            }
            // Reverse Index (move up, scroll if at top)
            b'M' => {
                if self.buffer.cursor().row == 0 {
                    self.buffer.scroll_down(1);
                } else {
                    self.buffer.move_cursor_by(-1, 0);
                }
            }
            _ => {}
        }
    }
}

impl<'a> BufferPerformer<'a> {
    /// Handle SGR (Select Graphic Rendition) sequences
    fn handle_sgr(&mut self, params: &[u16]) {
        if params.is_empty() {
            self.buffer.reset_pen();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            match params[i] {
                0 => self.buffer.reset_pen(),
                1 => self.buffer.set_attr(Attr::Bold),
                2 => self.buffer.set_attr(Attr::Dim),
                3 => self.buffer.set_attr(Attr::Italic),
                4 => self.buffer.set_attr(Attr::Underline),
                5 => self.buffer.set_attr(Attr::Blink),
                7 => self.buffer.set_attr(Attr::Reverse),
                8 => self.buffer.set_attr(Attr::Hidden),
                9 => self.buffer.set_attr(Attr::Strikethrough),
                21 => self.buffer.set_attr(Attr::NoBold),
                22 => {
                    self.buffer.set_attr(Attr::NoBold);
                    self.buffer.set_attr(Attr::NoDim);
                }
                23 => self.buffer.set_attr(Attr::NoItalic),
                24 => self.buffer.set_attr(Attr::NoUnderline),
                25 => self.buffer.set_attr(Attr::NoBlink),
                27 => self.buffer.set_attr(Attr::NoReverse),
                28 => self.buffer.set_attr(Attr::NoHidden),
                29 => self.buffer.set_attr(Attr::NoStrikethrough),
                // Foreground colors (30-37)
                30..=37 => {
                    self.buffer.set_fg(Color::Indexed((params[i] - 30) as u8));
                }
                38 => {
                    // Extended foreground color
                    if i + 1 < params.len() {
                        match params[i + 1] {
                            5 if i + 2 < params.len() => {
                                // 256 color
                                self.buffer.set_fg(Color::Indexed(params[i + 2] as u8));
                                i += 2;
                            }
                            2 if i + 4 < params.len() => {
                                // RGB color
                                self.buffer.set_fg(Color::Rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ));
                                i += 4;
                            }
                            _ => {}
                        }
                    }
                }
                39 => self.buffer.set_fg(Color::Default),
                // Background colors (40-47)
                40..=47 => {
                    self.buffer.set_bg(Color::Indexed((params[i] - 40) as u8));
                }
                48 => {
                    // Extended background color
                    if i + 1 < params.len() {
                        match params[i + 1] {
                            5 if i + 2 < params.len() => {
                                // 256 color
                                self.buffer.set_bg(Color::Indexed(params[i + 2] as u8));
                                i += 2;
                            }
                            2 if i + 4 < params.len() => {
                                // RGB color
                                self.buffer.set_bg(Color::Rgb(
                                    params[i + 2] as u8,
                                    params[i + 3] as u8,
                                    params[i + 4] as u8,
                                ));
                                i += 4;
                            }
                            _ => {}
                        }
                    }
                }
                49 => self.buffer.set_bg(Color::Default),
                // Bright foreground colors (90-97)
                90..=97 => {
                    self.buffer
                        .set_fg(Color::Indexed((params[i] - 90 + 8) as u8));
                }
                // Bright background colors (100-107)
                100..=107 => {
                    self.buffer
                        .set_bg(Color::Indexed((params[i] - 100 + 8) as u8));
                }
                _ => {}
            }
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::screen::parser::*;

    #[test]
    fn test_simple_text() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"Hello, World!");
        assert_eq!(parser.buffer().row_text(0), "Hello, World!");
    }

    #[test]
    fn test_newline() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"Line 1\r\nLine 2");
        assert_eq!(parser.buffer().row_text(0), "Line 1");
        assert_eq!(parser.buffer().row_text(1), "Line 2");
    }

    #[test]
    fn test_cursor_movement() {
        let mut parser = VtParser::new(80, 24);
        // Move to row 5, col 10 (1-indexed in escape sequence)
        parser.process(b"\x1b[6;11H");
        assert_eq!(parser.buffer().cursor().row, 5);
        assert_eq!(parser.buffer().cursor().col, 10);
    }

    #[test]
    fn test_cursor_up_down() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[10;10H"); // Move to row 10
        parser.process(b"\x1b[3A"); // Up 3
        assert_eq!(parser.buffer().cursor().row, 6);
        parser.process(b"\x1b[2B"); // Down 2
        assert_eq!(parser.buffer().cursor().row, 8);
    }

    #[test]
    fn test_clear_screen() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"Some text here");
        parser.process(b"\x1b[2J"); // Clear screen
        assert_eq!(parser.buffer().row_text(0), "");
    }

    #[test]
    fn test_clear_line() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"Hello World");
        parser.process(b"\x1b[6G"); // Move to column 6
        parser.process(b"\x1b[K"); // Clear to end of line
        assert_eq!(parser.buffer().row_text(0), "Hello");
    }

    #[test]
    fn test_sgr_colors() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[31mRed\x1b[0m"); // Red text, then reset
        let cell = parser.buffer().get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(1));
        assert_eq!(cell.grapheme, "R");
    }

    #[test]
    fn test_sgr_bold() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[1mBold\x1b[0m");
        let cell = parser.buffer().get_cell(0, 0).unwrap();
        assert!(cell.attrs.bold);
    }

    #[test]
    fn test_256_color() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[38;5;196mRed");
        let cell = parser.buffer().get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Indexed(196));
    }

    #[test]
    fn test_rgb_color() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[38;2;255;128;64mOrange");
        let cell = parser.buffer().get_cell(0, 0).unwrap();
        assert_eq!(cell.fg, Color::Rgb(255, 128, 64));
    }

    #[test]
    fn test_save_restore_cursor() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"\x1b[5;10H"); // Move to row 5, col 10
        parser.process(b"\x1b[s"); // Save
        parser.process(b"\x1b[1;1H"); // Move to origin
        parser.process(b"\x1b[u"); // Restore
        assert_eq!(parser.buffer().cursor().row, 4);
        assert_eq!(parser.buffer().cursor().col, 9);
    }

    #[test]
    fn test_scroll_up() {
        let mut parser = VtParser::new(80, 3);
        parser.process(b"Line1\r\nLine2\r\nLine3");
        parser.process(b"\x1b[S"); // Scroll up 1
        assert_eq!(parser.buffer().row_text(0), "Line2");
        assert_eq!(parser.buffer().row_text(1), "Line3");
        assert_eq!(parser.buffer().row_text(2), "");
    }

    #[test]
    fn test_contains_text() {
        let mut parser = VtParser::new(80, 24);
        parser.process(b"Welcome to the app!\r\nPress any key...");
        assert!(parser.buffer().contains_text("Welcome"));
        assert!(parser.buffer().contains_text("Press"));
        assert!(!parser.buffer().contains_text("Goodbye"));
    }
}
