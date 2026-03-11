//! Cell representation for terminal screen buffer
//!
//! Each cell in the terminal grid contains a grapheme, color, and style attributes.

use serde::{Deserialize, Serialize};

/// Color representation for terminal cells
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Color {
    /// Default terminal color
    #[default]
    Default,
    /// 256-color palette index (0-255)
    Indexed(u8),
    /// True color RGB
    Rgb(u8, u8, u8),
}

impl Color {
    /// Create a color from the 16 standard ANSI colors (0-15)
    pub fn ansi(index: u8) -> Self {
        Color::Indexed(index)
    }

    /// Create an RGB color
    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color::Rgb(r, g, b)
    }
}

/// Text style attributes for a cell
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CellAttrs {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub hidden: bool,
    pub strikethrough: bool,
    pub dim: bool,
}

impl CellAttrs {
    /// Create attributes with no styling
    pub fn none() -> Self {
        Self::default()
    }

    /// Check if any attribute is set
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Set bold
    pub fn with_bold(mut self) -> Self {
        self.bold = true;
        self
    }

    /// Set italic
    pub fn with_italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// Set underline
    pub fn with_underline(mut self) -> Self {
        self.underline = true;
        self
    }

    /// Set reverse video
    pub fn with_reverse(mut self) -> Self {
        self.reverse = true;
        self
    }
}

/// A single cell in the terminal grid
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cell {
    /// The grapheme displayed in this cell (may be empty for continuation of wide chars)
    pub grapheme: String,
    /// Cell width (1 for normal, 2 for wide characters, 0 for continuation)
    pub width: u8,
    /// Foreground color
    pub fg: Color,
    /// Background color
    pub bg: Color,
    /// Text style attributes
    pub attrs: CellAttrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            grapheme: " ".to_string(),
            width: 1,
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::default(),
        }
    }
}

impl Cell {
    /// Create an empty cell (space with default colors)
    pub fn empty() -> Self {
        Self::default()
    }

    /// Create a cell with a single character
    pub fn with_char(c: char) -> Self {
        Self {
            grapheme: c.to_string(),
            width: if c.is_ascii() { 1 } else { unicode_width(c) },
            ..Default::default()
        }
    }

    /// Create a cell with a grapheme cluster
    pub fn with_grapheme(grapheme: impl Into<String>) -> Self {
        let grapheme = grapheme.into();
        let width = grapheme.chars().next().map(unicode_width).unwrap_or(1);
        Self {
            grapheme,
            width,
            ..Default::default()
        }
    }

    /// Set the foreground color
    pub fn fg(mut self, color: Color) -> Self {
        self.fg = color;
        self
    }

    /// Set the background color
    pub fn bg(mut self, color: Color) -> Self {
        self.bg = color;
        self
    }

    /// Set the attributes
    pub fn attrs(mut self, attrs: CellAttrs) -> Self {
        self.attrs = attrs;
        self
    }

    /// Check if this is an empty/space cell
    pub fn is_empty(&self) -> bool {
        self.grapheme == " " || self.grapheme.is_empty()
    }

    /// Check if this cell is a continuation of a wide character
    pub fn is_continuation(&self) -> bool {
        self.width == 0
    }
}

/// Get the display width of a Unicode character
fn unicode_width(c: char) -> u8 {
    // Simplified width calculation
    // Wide characters (CJK, emoji, etc.) are width 2
    if c.is_ascii() {
        1
    } else {
        // Check for common wide character ranges
        let cp = c as u32;
        if (0x1100..=0x115F).contains(&cp)      // Hangul Jamo
            || (0x2E80..=0x9FFF).contains(&cp)  // CJK
            || (0xAC00..=0xD7A3).contains(&cp)  // Hangul Syllables
            || (0xF900..=0xFAFF).contains(&cp)  // CJK Compatibility
            || (0xFE10..=0xFE1F).contains(&cp)  // Vertical forms
            || (0xFF00..=0xFF60).contains(&cp)  // Fullwidth forms
            || (0x1F300..=0x1F9FF).contains(&cp) // Emoji
            || (0x20000..=0x2FFFF).contains(&cp)
        // CJK Extension
        {
            2
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::screen::cell::*;

    #[test]
    fn test_cell_default() {
        let cell = Cell::default();
        assert_eq!(cell.grapheme, " ");
        assert_eq!(cell.width, 1);
        assert_eq!(cell.fg, Color::Default);
        assert!(cell.is_empty());
    }

    #[test]
    fn test_cell_with_char() {
        let cell = Cell::with_char('A');
        assert_eq!(cell.grapheme, "A");
        assert_eq!(cell.width, 1);
        assert!(!cell.is_empty());
    }

    #[test]
    fn test_cell_colors() {
        let cell = Cell::with_char('X')
            .fg(Color::Indexed(1))
            .bg(Color::Rgb(255, 0, 0));
        assert_eq!(cell.fg, Color::Indexed(1));
        assert_eq!(cell.bg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn test_cell_attrs() {
        let attrs = CellAttrs::none().with_bold().with_underline();
        assert!(attrs.bold);
        assert!(attrs.underline);
        assert!(!attrs.italic);
    }

    #[test]
    fn test_unicode_width() {
        assert_eq!(unicode_width('A'), 1);
        assert_eq!(unicode_width('中'), 2);
    }
}
