//! Text selection support for the factory TUI
//!
//! Provides selection tracking and highlighting for terminal panes.

use ratatui::style::Modifier;
use ratatui::text::{Line, Span};

/// Represents a text selection in a pane.
///
/// Coordinates are relative to the pane's inner area (after borders).
/// Row 0 is the top of the visible viewport at the time the selection was made.
/// The `scroll_offset` field records the pane's viewport offset when the
/// selection was created, so rendering and text extraction can adjust for
/// subsequent scrolling.
#[derive(Debug, Clone, Default)]
pub struct Selection {
    /// The pane this selection belongs to
    pub pane_name: String,
    /// Starting position (row, col) - where mouse was pressed
    pub start: (u16, u16),
    /// Ending position (row, col) - current mouse position
    pub end: (u16, u16),
    /// Whether the selection is currently active (being dragged)
    pub is_active: bool,
    /// The pane's viewport scroll offset when the selection was created.
    /// Used to adjust selection coordinates when the pane scrolls.
    pub scroll_offset: u32,
}

impl Selection {
    /// Create a new selection starting at the given position
    pub fn new(pane_name: String, row: u16, col: u16) -> Self {
        Self {
            pane_name,
            start: (row, col),
            end: (row, col),
            is_active: true,
            scroll_offset: 0,
        }
    }

    /// Update the end position of the selection
    pub fn update_end(&mut self, row: u16, col: u16) {
        self.end = (row, col);
    }

    /// Finalize the selection (mouse released)
    pub fn finalize(&mut self) {
        self.is_active = false;
    }

    /// Get normalized selection bounds (start <= end)
    ///
    /// Returns (start_row, start_col, end_row, end_col) where start is always
    /// before or equal to end in reading order.
    pub fn normalized(&self) -> (u16, u16, u16, u16) {
        let (sr, sc) = self.start;
        let (er, ec) = self.end;

        if sr < er || (sr == er && sc <= ec) {
            (sr, sc, er, ec)
        } else {
            (er, ec, sr, sc)
        }
    }

    /// Check if a cell at (row, col) is within the selection
    pub fn contains(&self, row: u16, col: u16) -> bool {
        let (sr, sc, er, ec) = self.normalized();

        if row < sr || row > er {
            return false;
        }

        if sr == er {
            // Single line selection
            col >= sc && col <= ec
        } else if row == sr {
            // First line: from start_col to end of line
            col >= sc
        } else if row == er {
            // Last line: from start of line to end_col
            col <= ec
        } else {
            // Middle lines: entire line is selected
            true
        }
    }

    /// Check if the selection is empty (start == end)
    pub fn is_empty(&self) -> bool {
        self.start == self.end
    }

    /// Clear the selection
    pub fn clear(&mut self) {
        self.start = (0, 0);
        self.end = (0, 0);
        self.is_active = false;
        self.pane_name.clear();
        self.scroll_offset = 0;
    }
}

/// Apply selection highlighting to a line.
///
/// Takes a line and selection info, returns a new line with selected
/// characters highlighted using reversed colors.
///
/// `scroll_delta` is `current_scroll_offset - selection.scroll_offset` (as i32).
/// It shifts the selection rows so highlighting follows the text when the
/// pane is scrolled after the selection was made.
pub fn apply_selection_to_line(
    line: Line<'static>,
    row: u16,
    selection: &Selection,
    scroll_delta: i32,
) -> Line<'static> {
    if selection.is_empty() {
        return line;
    }

    let (sr, sc, er, ec) = selection.normalized();

    // Shift selection rows by the scroll delta so the highlight tracks the text.
    let adjusted_sr = sr as i32 + scroll_delta;
    let adjusted_er = er as i32 + scroll_delta;
    let row_i = row as i32;

    // Check if this row intersects the adjusted selection
    if row_i < adjusted_sr || row_i > adjusted_er {
        return line;
    }

    // Calculate selection range for this row
    let (sel_start, sel_end) = if adjusted_sr == adjusted_er {
        // Single line selection
        (sc as usize, ec as usize)
    } else if row_i == adjusted_sr {
        // First line: from start_col to end of line
        (sc as usize, usize::MAX)
    } else if row_i == adjusted_er {
        // Last line: from start of line to end_col
        (0, ec as usize)
    } else {
        // Middle lines: entire line
        (0, usize::MAX)
    };

    // Apply highlighting to spans
    let mut new_spans = Vec::new();
    let mut char_offset = 0;

    for span in line.spans {
        let span_len = span.content.chars().count();
        let span_start = char_offset;
        let span_end = char_offset + span_len;

        if span_end <= sel_start || span_start > sel_end {
            // Span is entirely outside selection
            new_spans.push(span);
        } else if span_start >= sel_start && span_end <= sel_end {
            // Span is entirely inside selection - highlight all
            new_spans.push(Span::styled(
                span.content,
                span.style.add_modifier(Modifier::REVERSED),
            ));
        } else {
            // Span is partially selected - split it
            let chars: Vec<char> = span.content.chars().collect();
            let mut i = 0;

            while i < chars.len() {
                let abs_pos = span_start + i;
                let in_selection = abs_pos >= sel_start && abs_pos <= sel_end;

                // Find run of same selection state
                let mut j = i + 1;
                while j < chars.len() {
                    let next_abs = span_start + j;
                    let next_in = next_abs >= sel_start && next_abs <= sel_end;
                    if next_in != in_selection {
                        break;
                    }
                    j += 1;
                }

                // Create span for this run
                let text: String = chars[i..j].iter().collect();
                let style = if in_selection {
                    span.style.add_modifier(Modifier::REVERSED)
                } else {
                    span.style
                };
                new_spans.push(Span::styled(text, style));

                i = j;
            }
        }

        char_offset = span_end;
    }

    Line::from(new_spans)
}

#[cfg(test)]
mod tests {
    use crate::ui::factory::selection::*;

    #[test]
    fn test_selection_contains_single_line() {
        let sel = Selection {
            pane_name: "test".to_string(),
            start: (5, 10),
            end: (5, 20),
            is_active: false,
            scroll_offset: 0,
        };

        assert!(sel.contains(5, 10)); // Start
        assert!(sel.contains(5, 15)); // Middle
        assert!(sel.contains(5, 20)); // End
        assert!(!sel.contains(5, 9)); // Before
        assert!(!sel.contains(5, 21)); // After
        assert!(!sel.contains(4, 15)); // Wrong row
    }

    #[test]
    fn test_selection_contains_multi_line() {
        let sel = Selection {
            pane_name: "test".to_string(),
            start: (5, 10),
            end: (7, 5),
            is_active: false,
            scroll_offset: 0,
        };

        // First line: col >= 10
        assert!(!sel.contains(5, 9));
        assert!(sel.contains(5, 10));
        assert!(sel.contains(5, 100));

        // Middle line: all columns
        assert!(sel.contains(6, 0));
        assert!(sel.contains(6, 50));

        // Last line: col <= 5
        assert!(sel.contains(7, 0));
        assert!(sel.contains(7, 5));
        assert!(!sel.contains(7, 6));
    }

    #[test]
    fn test_selection_normalized_reverse() {
        // Selection dragged backwards
        let sel = Selection {
            pane_name: "test".to_string(),
            start: (10, 20),
            end: (5, 10),
            is_active: false,
            scroll_offset: 0,
        };

        let (sr, sc, er, ec) = sel.normalized();
        assert_eq!((sr, sc, er, ec), (5, 10, 10, 20));
    }

    #[test]
    fn test_apply_selection_to_line() {
        let line = Line::from(vec![Span::raw("Hello World")]);
        let sel = Selection {
            pane_name: "test".to_string(),
            start: (0, 0),
            end: (0, 4),
            is_active: false,
            scroll_offset: 0,
        };

        let highlighted = apply_selection_to_line(line, 0, &sel, 0);
        assert_eq!(highlighted.spans.len(), 2); // "Hello" highlighted, " World" not
    }

    #[test]
    fn test_apply_selection_with_scroll_delta() {
        let line = Line::from(vec![Span::raw("Hello World")]);
        let sel = Selection {
            pane_name: "test".to_string(),
            start: (2, 0),
            end: (2, 4),
            is_active: false,
            scroll_offset: 0,
        };

        // Selection at row 2 with scroll_delta=3 means it now appears at viewport row 5
        let highlighted = apply_selection_to_line(line.clone(), 5, &sel, 3);
        assert_eq!(highlighted.spans.len(), 2); // Should highlight at adjusted row

        // Row 2 should no longer be highlighted (selection moved to row 5)
        let not_highlighted = apply_selection_to_line(line, 2, &sel, 3);
        assert_eq!(not_highlighted.spans.len(), 1); // No highlight
    }
}
