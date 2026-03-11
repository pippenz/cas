use crate::error::{Error, Result};
use crate::pane::Pane;
use crate::pane::style::{cell_style_to_flags, convert_style_runs_to_proto, debug_log_enabled};
use cas_factory_protocol::{CacheRow, CursorPosition, RowData, TerminalCell, TerminalSnapshot};

impl Pane {
    /// Get scrollback information from the terminal
    pub fn scrollback_info(&self) -> ghostty_vt::ScrollbackInfo {
        self.terminal.scrollback_info()
    }

    /// Get the current scroll offset (lines from bottom, 0 = at bottom)
    pub fn scroll_offset(&self) -> u32 {
        self.terminal.scrollback_info().viewport_offset
    }

    /// Get the total number of scrollback lines
    pub fn scrollback_lines(&self) -> u32 {
        self.terminal.scrollback_info().total_scrollback
    }

    /// Get a full terminal snapshot for the current viewport
    pub fn get_full_snapshot(&self) -> Result<TerminalSnapshot> {
        let mut cells = Vec::with_capacity((self.rows as usize) * (self.cols as usize));

        for row in 0..self.rows {
            let text = self.dump_row(row).unwrap_or_default();
            let styles = self.row_styles(row).unwrap_or_default();
            let chars: Vec<char> = text.chars().collect();

            for col in 0..self.cols as usize {
                let ch = chars.get(col).copied().unwrap_or(' ');
                let style = styles.get(col).cloned().unwrap_or_default();

                cells.push(TerminalCell {
                    codepoint: ch as u32,
                    fg: (style.fg.r, style.fg.g, style.fg.b),
                    bg: (style.bg.r, style.bg.g, style.bg.b),
                    flags: cell_style_to_flags(&style),
                    width: 1,
                });
            }
        }

        let (cursor_col, cursor_row) = self.cursor_position();
        Ok(TerminalSnapshot {
            cells,
            cursor: CursorPosition {
                x: cursor_col.saturating_sub(1),
                y: cursor_row.saturating_sub(1),
            },
            cols: self.cols,
            rows: self.rows,
        })
    }

    #[allow(clippy::type_complexity)]
    pub fn get_incremental_update(
        &mut self,
    ) -> Result<Option<(Vec<RowData>, Option<CursorPosition>, u64)>> {
        let mut force_all = self.take_force_all_dirty();
        let scroll_info = self.terminal.scrollback_info();
        if scroll_info.total_scrollback != self.last_total_scrollback {
            if scroll_info.total_scrollback > self.last_total_scrollback {
                force_all = true;
            }
            self.last_total_scrollback = scroll_info.total_scrollback;
        }

        let dirty_rows = if force_all {
            if debug_log_enabled() {
                tracing::debug!(
                    "Pane {}: force_all_dirty, returning all {} rows",
                    self.id,
                    self.rows
                );
            }
            (0..self.rows).collect::<Vec<_>>()
        } else {
            let rows = self.terminal.take_dirty_rows(self.rows);
            if debug_log_enabled() && !rows.is_empty() {
                tracing::debug!("Pane {}: {} dirty rows from terminal", self.id, rows.len());
            }
            rows
        };

        if dirty_rows.is_empty() {
            return Ok(None);
        }

        let mut rows = Vec::with_capacity(dirty_rows.len());
        for row_idx in dirty_rows {
            let text = match self.dump_row(row_idx) {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(
                        "Pane {}: dump_row({}) failed (pane.rows={}): {}",
                        self.id,
                        row_idx,
                        self.rows,
                        e
                    );
                    return Err(e);
                }
            };
            let style_runs = self
                .terminal
                .row_style_runs(row_idx)
                .map_err(|e| Error::terminal(e.to_string()))?;

            let runs = convert_style_runs_to_proto(&text, &style_runs, self.cols as usize);
            rows.push(RowData { row: row_idx, runs });
        }

        let (cursor_col, cursor_row) = self.cursor_position();
        let cursor = Some(CursorPosition {
            x: cursor_col.saturating_sub(1),
            y: cursor_row.saturating_sub(1),
        });

        let seq = self.seq_counter;
        self.seq_counter = self.seq_counter.wrapping_add(1);

        Ok(Some((rows, cursor, seq)))
    }

    pub fn create_snapshot_with_cache(
        &self,
        cache_window: u32,
    ) -> Result<(TerminalSnapshot, Vec<CacheRow>, Option<u32>)> {
        let snapshot = self.get_full_snapshot()?;

        if cache_window == 0 {
            return Ok((snapshot, Vec::new(), None));
        }

        let info = self.terminal.scrollback_info();
        let viewport_rows = info.viewport_rows as u32;
        let total = info.total_scrollback;

        let viewport_bottom = total.saturating_sub(info.viewport_offset);
        let viewport_top = viewport_bottom.saturating_sub(viewport_rows);

        let buffer = cache_window.min(viewport_rows * 2);
        let cache_start = viewport_top.saturating_sub(buffer);
        let cache_end = (viewport_bottom + buffer).min(total);

        let mut cache_rows = Vec::new();
        for screen_row in cache_start..cache_end {
            if screen_row >= viewport_top && screen_row < viewport_bottom {
                continue;
            }

            let text = self
                .terminal
                .dump_screen_row(screen_row)
                .unwrap_or_default();
            let style_runs = self
                .terminal
                .screen_row_style_runs(screen_row)
                .unwrap_or_default();

            let proto_runs = convert_style_runs_to_proto(&text, &style_runs, self.cols as usize);
            cache_rows.push(CacheRow {
                screen_row,
                text,
                style_runs: proto_runs,
            });
        }

        Ok((snapshot, cache_rows, Some(cache_start)))
    }

    pub fn create_snapshot_rows_with_cache(
        &self,
        cache_window: u32,
    ) -> Result<(Vec<RowData>, Vec<CacheRow>, Option<u32>)> {
        let info = self.terminal.scrollback_info();
        let viewport_rows = info.viewport_rows as u32;
        let total = info.total_scrollback;

        let rows = self.get_viewport_rows_data()?;

        if cache_window == 0 {
            return Ok((rows, Vec::new(), None));
        }

        let viewport_bottom = total.saturating_sub(info.viewport_offset);
        let viewport_top = viewport_bottom.saturating_sub(viewport_rows);

        let buffer = cache_window.min(viewport_rows * 2);
        let cache_start = viewport_top.saturating_sub(buffer);
        let cache_end = (viewport_bottom + buffer).min(total);

        let mut cache_rows = Vec::new();
        for screen_row in cache_start..cache_end {
            if screen_row >= viewport_top && screen_row < viewport_bottom {
                continue;
            }

            let text = self
                .terminal
                .dump_screen_row(screen_row)
                .unwrap_or_default();
            let style_runs = self
                .terminal
                .screen_row_style_runs(screen_row)
                .unwrap_or_default();

            let proto_runs = convert_style_runs_to_proto(&text, &style_runs, self.cols as usize);
            cache_rows.push(CacheRow {
                screen_row,
                text,
                style_runs: proto_runs,
            });
        }

        Ok((rows, cache_rows, Some(cache_start)))
    }

    pub fn get_viewport_rows_data(&self) -> Result<Vec<RowData>> {
        let mut rows = Vec::with_capacity(self.rows as usize);
        for row_idx in 0..self.rows {
            let text = self.dump_row(row_idx).unwrap_or_default();
            let style_runs = self
                .terminal
                .row_style_runs(row_idx)
                .map_err(|e| Error::terminal(e.to_string()))?;
            let runs = convert_style_runs_to_proto(&text, &style_runs, self.cols as usize);
            rows.push(RowData { row: row_idx, runs });
        }
        Ok(rows)
    }
}
