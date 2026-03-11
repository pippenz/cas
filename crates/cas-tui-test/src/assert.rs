//! Assertion DSL - Test assertions for terminal output
//!
//! Provides assertions for:
//! - Text content (contains, exact, at position)
//! - Regex patterns
//! - Region-based assertions
//! - Cursor position (when available from ANSI parsing)

use crate::screen::{ScreenBuffer, VtParser};
use regex::Regex;
use thiserror::Error;

/// Errors from assertion failures
#[derive(Debug, Error)]
pub enum AssertError {
    #[error("Text not found: expected '{expected}' in output")]
    TextNotFound { expected: String },

    #[error("Text mismatch at row {row}: expected '{expected}', found '{found}'")]
    TextMismatch {
        row: usize,
        expected: String,
        found: String,
    },

    #[error("Pattern not matched: '{pattern}' not found in output")]
    PatternNotMatched { pattern: String },

    #[error("Region mismatch at ({row}, {col}): expected '{expected}', found '{found}'")]
    RegionMismatch {
        row: usize,
        col: usize,
        expected: String,
        found: String,
    },

    #[error("Row {row} out of bounds (max {max})")]
    RowOutOfBounds { row: usize, max: usize },

    #[error("Column {col} out of bounds at row {row} (max {max})")]
    ColOutOfBounds { col: usize, row: usize, max: usize },

    #[error("Invalid regex: {0}")]
    InvalidRegex(#[from] regex::Error),
}

/// Result type for assertions
pub type AssertResult = Result<(), AssertError>;

/// Screen representation for assertions
/// Splits output into lines for positional assertions
#[derive(Debug, Clone)]
pub struct Screen {
    lines: Vec<String>,
    raw: String,
    buffer: ScreenBuffer,
}

impl Screen {
    /// Create a screen from raw output
    pub fn from_output(output: &str) -> Self {
        Self::from_output_with_size(output, 80, 24)
    }

    /// Create a screen from raw output with explicit terminal size
    pub fn from_output_with_size(output: &str, cols: u16, rows: u16) -> Self {
        let mut parser = VtParser::new(cols, rows);
        let normalized = normalize_newlines(output);
        parser.process(normalized.as_bytes());
        let buffer = parser.into_buffer();
        let lines = buffer.text_lines();
        Self {
            lines,
            raw: output.to_string(),
            buffer,
        }
    }

    /// Get the raw output with ANSI codes
    pub fn raw(&self) -> &str {
        &self.raw
    }

    /// Get cleaned lines
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Get number of lines
    pub fn height(&self) -> usize {
        self.lines.len()
    }

    /// Get a specific line (0-indexed)
    pub fn line(&self, row: usize) -> Option<&str> {
        self.lines.get(row).map(|s| s.as_str())
    }

    /// Get text at a specific position
    pub fn text_at(&self, row: usize, col: usize, len: usize) -> Option<String> {
        self.region_text(row, col, len)
    }

    /// Assert that output contains text
    pub fn assert_contains(&self, text: &str) -> AssertResult {
        if self.text().contains(text) {
            Ok(())
        } else {
            Err(AssertError::TextNotFound {
                expected: text.to_string(),
            })
        }
    }

    /// Assert that output does not contain text
    pub fn assert_not_contains(&self, text: &str) -> AssertResult {
        if !self.text().contains(text) {
            Ok(())
        } else {
            Err(AssertError::TextNotFound {
                expected: format!("NOT: {text}"),
            })
        }
    }

    /// Assert that a specific row contains text
    pub fn assert_row_contains(&self, row: usize, text: &str) -> AssertResult {
        if row >= self.lines.len() {
            return Err(AssertError::RowOutOfBounds {
                row,
                max: self.lines.len().saturating_sub(1),
            });
        }

        if self.lines[row].contains(text) {
            Ok(())
        } else {
            Err(AssertError::TextMismatch {
                row,
                expected: text.to_string(),
                found: self.lines[row].clone(),
            })
        }
    }

    /// Assert exact text at position
    pub fn assert_text_at(&self, row: usize, col: usize, expected: &str) -> AssertResult {
        if row >= self.lines.len() {
            return Err(AssertError::RowOutOfBounds {
                row,
                max: self.lines.len().saturating_sub(1),
            });
        }

        let width = display_width(expected);
        let found = self.region_text(row, col, width);
        if found.is_none() {
            return Err(AssertError::ColOutOfBounds {
                col,
                row,
                max: self.buffer.size().cols.saturating_sub(1) as usize,
            });
        }

        let found = found.unwrap_or_default();
        if found == expected {
            Ok(())
        } else {
            Err(AssertError::RegionMismatch {
                row,
                col,
                expected: expected.to_string(),
                found,
            })
        }
    }

    /// Assert a regex pattern matches somewhere in output
    pub fn assert_matches(&self, pattern: &str) -> AssertResult {
        let regex = Regex::new(pattern)?;
        let text = self.text();
        if regex.is_match(&text) {
            Ok(())
        } else {
            Err(AssertError::PatternNotMatched {
                pattern: pattern.to_string(),
            })
        }
    }

    /// Assert a regex pattern matches on a specific row
    pub fn assert_row_matches(&self, row: usize, pattern: &str) -> AssertResult {
        if row >= self.lines.len() {
            return Err(AssertError::RowOutOfBounds {
                row,
                max: self.lines.len().saturating_sub(1),
            });
        }

        let regex = Regex::new(pattern)?;

        if regex.is_match(&self.lines[row]) {
            Ok(())
        } else {
            Err(AssertError::PatternNotMatched {
                pattern: pattern.to_string(),
            })
        }
    }

    /// Assert a region of the screen matches expected content
    pub fn assert_region(
        &self,
        start_row: usize,
        start_col: usize,
        width: usize,
        height: usize,
        expected: &[&str],
    ) -> AssertResult {
        if expected.len() != height {
            return Err(AssertError::RegionMismatch {
                row: start_row,
                col: start_col,
                expected: format!("{height} lines"),
                found: format!("{} lines provided", expected.len()),
            });
        }

        for (i, expected_line) in expected.iter().enumerate() {
            let row = start_row + i;
            if row >= self.lines.len() {
                return Err(AssertError::RowOutOfBounds {
                    row,
                    max: self.lines.len().saturating_sub(1),
                });
            }

            let found = self.region_text(row, start_col, width).unwrap_or_default();

            // Pad found to width for comparison
            let found_padded = format!("{found:width$}");
            let expected_padded = format!("{expected_line:width$}");

            if found_padded != expected_padded {
                return Err(AssertError::RegionMismatch {
                    row,
                    col: start_col,
                    expected: expected_line.to_string(),
                    found: found.to_string(),
                });
            }
        }

        Ok(())
    }

    /// Get the full text content as a single string
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn region_text(&self, row: usize, col: usize, width: usize) -> Option<String> {
        let size = self.buffer.size();
        if row >= size.rows as usize || col >= size.cols as usize {
            return None;
        }

        let end = (col + width).min(size.cols as usize);
        let mut out = String::new();
        for c in col..end {
            if let Some(cell) = self.buffer.get_cell(row as u16, c as u16) {
                if cell.grapheme.is_empty() {
                    out.push(' ');
                } else {
                    out.push_str(&cell.grapheme);
                }
            }
        }
        Some(out)
    }
}

/// Strip ANSI escape codes from a string
pub fn strip_ansi_codes(s: &str) -> String {
    // Match ANSI escape sequences: ESC [ ... (letter or @-_)
    let ansi_regex = Regex::new(r"\x1b\[[0-9;]*[A-Za-z@-_]|\x1b\][^\x07]*\x07|\x1b[()][AB012]")
        .expect("invalid regex");
    ansi_regex.replace_all(s, "").to_string()
}

/// Convenience function to create a Screen from output string
pub fn screen(output: &str) -> Screen {
    Screen::from_output(output)
}

/// Convenience function to create a Screen with explicit terminal size
pub fn screen_with_size(output: &str, cols: u16, rows: u16) -> Screen {
    Screen::from_output_with_size(output, cols, rows)
}

fn normalize_newlines(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\n' {
            // Convert lone LF to CRLF for display-like rendering
            if !out.ends_with('\r') {
                out.push('\r');
            }
        }
        out.push(c);
    }
    out
}

fn display_width(s: &str) -> usize {
    s.chars().map(char_display_width).sum()
}

fn char_display_width(c: char) -> usize {
    if c.is_ascii() {
        1
    } else {
        let cp = c as u32;
        if (0x1100..=0x115F).contains(&cp)
            || (0x2E80..=0x9FFF).contains(&cp)
            || (0xAC00..=0xD7A3).contains(&cp)
            || (0xF900..=0xFAFF).contains(&cp)
            || (0xFE10..=0xFE1F).contains(&cp)
            || (0xFF00..=0xFF60).contains(&cp)
            || (0x1F300..=0x1F9FF).contains(&cp)
            || (0x20000..=0x2FFFF).contains(&cp)
        {
            2
        } else {
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::assert::*;

    #[test]
    fn test_strip_ansi_codes() {
        let input = "\x1b[31mred\x1b[0m text";
        assert_eq!(strip_ansi_codes(input), "red text");

        let input2 = "\x1b[1;32mbold green\x1b[0m";
        assert_eq!(strip_ansi_codes(input2), "bold green");
    }

    #[test]
    fn test_screen_from_output() {
        let output = "line1\nline2\nline3";
        let screen = screen_with_size(output, 80, 3);

        assert_eq!(screen.height(), 3);
        assert_eq!(screen.line(0), Some("line1"));
        assert_eq!(screen.line(1), Some("line2"));
        assert_eq!(screen.line(2), Some("line3"));
        assert_eq!(screen.line(3), None);
    }

    #[test]
    fn test_assert_contains() {
        let screen = screen("hello world\nfoo bar");

        assert!(screen.assert_contains("hello").is_ok());
        assert!(screen.assert_contains("world").is_ok());
        assert!(screen.assert_contains("foo").is_ok());
        assert!(screen.assert_contains("missing").is_err());
    }

    #[test]
    fn test_assert_not_contains() {
        let screen = screen("hello world");

        assert!(screen.assert_not_contains("missing").is_ok());
        assert!(screen.assert_not_contains("hello").is_err());
    }

    #[test]
    fn test_assert_row_contains() {
        let screen = screen("line one\nline two\nline three");

        assert!(screen.assert_row_contains(0, "one").is_ok());
        assert!(screen.assert_row_contains(1, "two").is_ok());
        assert!(screen.assert_row_contains(0, "two").is_err());
        assert!(screen.assert_row_contains(10, "any").is_err());
    }

    #[test]
    fn test_assert_text_at() {
        let screen = screen("0123456789\nabcdefghij");

        assert!(screen.assert_text_at(0, 0, "0123").is_ok());
        assert!(screen.assert_text_at(0, 5, "5678").is_ok());
        assert!(screen.assert_text_at(1, 0, "abcd").is_ok());
        assert!(screen.assert_text_at(0, 0, "xxxx").is_err());
    }

    #[test]
    fn test_assert_matches() {
        let screen = screen("value: 42\ncount: 100");

        assert!(screen.assert_matches(r"value:\s+\d+").is_ok());
        assert!(screen.assert_matches(r"count:\s+\d{3}").is_ok());
        assert!(screen.assert_matches(r"missing:\s+\d+").is_err());
    }

    #[test]
    fn test_assert_row_matches() {
        let screen = screen("item: 123\nitem: 456");

        assert!(screen.assert_row_matches(0, r"item:\s+123").is_ok());
        assert!(screen.assert_row_matches(1, r"item:\s+\d+").is_ok());
        assert!(screen.assert_row_matches(0, r"item:\s+456").is_err());
    }

    #[test]
    fn test_assert_region() {
        let screen = screen("ABCDE\nFGHIJ\nKLMNO");

        assert!(screen.assert_region(0, 0, 3, 2, &["ABC", "FGH"]).is_ok());
        assert!(screen.assert_region(1, 1, 3, 2, &["GHI", "LMN"]).is_ok());
        assert!(screen.assert_region(0, 0, 3, 2, &["XXX", "XXX"]).is_err());
    }

    #[test]
    fn test_text_at() {
        let screen = screen_with_size("hello world", 80, 1);
        assert_eq!(screen.text_at(0, 0, 5), Some("hello".to_string()));
        assert_eq!(screen.text_at(0, 6, 5), Some("world".to_string()));
        assert_eq!(screen.text_at(1, 0, 5), None);
    }

    #[test]
    fn test_vt_cursor_overwrite() {
        let output = "12345\x1b[1GAB";
        let screen = screen(output);
        assert!(screen.assert_text_at(0, 0, "AB").is_ok());
        assert!(screen.assert_contains("AB345").is_ok());
    }
}
