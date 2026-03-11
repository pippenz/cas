//! Diff iteration with split/unified layout support.
//!
//! Ports the iteration logic from `@pierre/diffs` `iterateOverDiff.ts` to Rust.
//! Walks through hunks in a [`FileDiffMetadata`], emitting [`DiffLineEvent`]s
//! for each line. Supports unified, split, and dual-mode layout, windowed
//! rendering for virtual scrolling, collapsed context between hunks, and
//! hunk expansion regions.

use std::collections::HashMap;

use crate::{ChangeContent, FileDiffMetadata, Hunk, HunkContent};

/// Default number of collapsed context lines that auto-expand.
pub const DEFAULT_COLLAPSED_CONTEXT_THRESHOLD: usize = 1;

/// Layout mode for diff rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffStyle {
    /// Side-by-side: deletions on the left, additions on the right.
    Split,
    /// Interleaved: deletions then additions in sequence.
    Unified,
    /// Track both unified and split counters simultaneously.
    Both,
}

/// Metadata for a single line in the diff output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiffLineMetadata {
    /// 1-based line number in the file.
    pub line_number: usize,
    /// Zero-based index into the addition_lines or deletion_lines array.
    pub line_index: usize,
    /// Line index in unified rendering.
    pub unified_line_index: usize,
    /// Line index in split rendering.
    pub split_line_index: usize,
    /// True if this line has no trailing newline at EOF.
    pub no_eof_cr: bool,
}

/// An event emitted during diff iteration, representing one rendered line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineEvent {
    /// An unchanged context line present in both old and new files.
    Context {
        hunk_index: usize,
        collapsed_before: usize,
        collapsed_after: usize,
        deletion_line: DiffLineMetadata,
        addition_line: DiffLineMetadata,
    },
    /// An expanded context line loaded from full file content.
    ContextExpanded {
        hunk_index: usize,
        collapsed_before: usize,
        collapsed_after: usize,
        deletion_line: DiffLineMetadata,
        addition_line: DiffLineMetadata,
    },
    /// A changed line — deletion, addition, or both paired in split mode.
    Change {
        hunk_index: usize,
        collapsed_before: usize,
        collapsed_after: usize,
        deletion_line: Option<DiffLineMetadata>,
        addition_line: Option<DiffLineMetadata>,
    },
}

/// A region of expanded context around a hunk separator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkExpansionRegion {
    /// Number of lines expanded from the start of the collapsed region.
    pub from_start: usize,
    /// Number of lines expanded from the end of the collapsed region.
    pub from_end: usize,
}

/// Configuration for diff iteration.
pub struct IterateOverDiffProps<'a> {
    /// The file diff to iterate over.
    pub diff: &'a FileDiffMetadata,
    /// Layout mode.
    pub diff_style: DiffStyle,
    /// First visible line index (for windowed rendering).
    pub starting_line: usize,
    /// Number of visible lines (for windowed rendering). Use `usize::MAX` for
    /// no limit.
    pub total_lines: usize,
    /// Expanded hunk regions keyed by hunk index. `None` means no expansion.
    pub expanded_hunks: Option<&'a HashMap<usize, HunkExpansionRegion>>,
    /// If true, expand all collapsed regions.
    pub expand_all: bool,
    /// Number of collapsed lines that auto-expand without a separator.
    pub collapsed_context_threshold: usize,
}

impl<'a> IterateOverDiffProps<'a> {
    /// Create props with default windowing (show everything).
    pub fn new(diff: &'a FileDiffMetadata, diff_style: DiffStyle) -> Self {
        Self {
            diff,
            diff_style,
            starting_line: 0,
            total_lines: usize::MAX,
            expanded_hunks: None,
            expand_all: false,
            collapsed_context_threshold: DEFAULT_COLLAPSED_CONTEXT_THRESHOLD,
        }
    }
}

// Internal state for the iteration.
struct IterationState {
    diff_style: DiffStyle,
    viewport_start: usize,
    viewport_end: usize, // saturating: start + total
    is_windowed: bool,
    split_count: usize,
    unified_count: usize,
}

impl IterationState {
    fn new(props: &IterateOverDiffProps<'_>) -> Self {
        let viewport_end = props.starting_line.saturating_add(props.total_lines);
        Self {
            diff_style: props.diff_style,
            viewport_start: props.starting_line,
            viewport_end,
            is_windowed: props.starting_line > 0 || props.total_lines < usize::MAX,
            split_count: 0,
            unified_count: 0,
        }
    }

    fn should_break(&self) -> bool {
        if !self.is_windowed {
            return false;
        }
        let break_unified = self.unified_count >= self.viewport_end;
        let break_split = self.split_count >= self.viewport_end;
        match self.diff_style {
            DiffStyle::Unified => break_unified,
            DiffStyle::Split => break_split,
            DiffStyle::Both => break_unified && break_split,
        }
    }

    fn should_skip(&self, unified_height: usize, split_height: usize) -> bool {
        if !self.is_windowed {
            return false;
        }
        let skip_unified = self.unified_count + unified_height < self.viewport_start;
        let skip_split = self.split_count + split_height < self.viewport_start;
        match self.diff_style {
            DiffStyle::Unified => skip_unified,
            DiffStyle::Split => skip_split,
            DiffStyle::Both => skip_unified && skip_split,
        }
    }

    fn increment_counts(&mut self, unified_value: usize, split_value: usize) {
        match self.diff_style {
            DiffStyle::Unified => self.unified_count += unified_value,
            DiffStyle::Split => self.split_count += split_value,
            DiffStyle::Both => {
                self.unified_count += unified_value;
                self.split_count += split_value;
            }
        }
    }

    fn is_in_window(&self, unified_height: usize, split_height: usize) -> bool {
        if !self.is_windowed {
            return true;
        }
        let unified_in = self.is_in_unified_window(unified_height);
        let split_in = self.is_in_split_window(split_height);
        match self.diff_style {
            DiffStyle::Unified => unified_in,
            DiffStyle::Split => split_in,
            DiffStyle::Both => unified_in || split_in,
        }
    }

    fn is_in_unified_window(&self, unified_height: usize) -> bool {
        !self.is_windowed
            || (self.unified_count + unified_height >= self.viewport_start
                && self.unified_count < self.viewport_end)
    }

    fn is_in_split_window(&self, split_height: usize) -> bool {
        !self.is_windowed
            || (self.split_count + split_height >= self.viewport_start
                && self.split_count < self.viewport_end)
    }
}

/// Iterate over a file diff, calling `callback` for each line event.
///
/// Returns `true` if the callback requested an early stop (by returning `true`),
/// `false` if iteration completed normally.
///
/// The callback receives a [`DiffLineEvent`] and returns `true` to stop
/// iteration or `false` to continue.
pub fn iterate_over_diff<F>(props: &IterateOverDiffProps<'_>, mut callback: F) -> bool
where
    F: FnMut(DiffLineEvent) -> bool,
{
    let diff = props.diff;
    let mut state = IterationState::new(props);
    let final_hunk_idx = diff.hunks.len().checked_sub(1);

    'hunk_iter: for (hunk_index, hunk) in diff.hunks.iter().enumerate() {
        if state.should_break() {
            break;
        }

        let is_final_hunk = Some(hunk_index) == final_hunk_idx;

        // Leading expanded region
        let mut leading = get_expanded_region(
            diff.is_partial,
            hunk.collapsed_before,
            props.expanded_hunks,
            props.expand_all,
            hunk_index,
            props.collapsed_context_threshold,
        );

        // Trailing expanded region (only on last hunk)
        let trailing = if is_final_hunk && has_final_collapsed_hunk(diff) {
            let addition_remaining = diff
                .addition_lines
                .len()
                .saturating_sub(hunk.addition_line_index + hunk.addition_count);
            let deletion_remaining = diff
                .deletion_lines
                .len()
                .saturating_sub(hunk.deletion_line_index + hunk.deletion_count);
            let trailing_size = addition_remaining.min(deletion_remaining);
            Some(get_expanded_region(
                diff.is_partial,
                trailing_size,
                props.expanded_hunks,
                props.expand_all,
                diff.hunks.len(),
                props.collapsed_context_threshold,
            ))
        } else {
            None
        };

        let expanded_line_count = leading.from_start + leading.from_end;

        // Helper closures for collapsed tracking
        let get_trailing_collapsed_after =
            |trailing: &Option<ExpandedRegionResult>,
             unified_line_index: usize,
             split_line_index: usize| {
                let tr = match trailing {
                    Some(tr) if tr.collapsed_lines > 0 && tr.from_start + tr.from_end == 0 => tr,
                    _ => return 0,
                };
                match props.diff_style {
                    DiffStyle::Unified => {
                        if unified_line_index
                            == hunk.unified_line_start + hunk.unified_line_count - 1
                        {
                            tr.collapsed_lines
                        } else {
                            0
                        }
                    }
                    _ => {
                        if split_line_index == hunk.split_line_start + hunk.split_line_count - 1 {
                            tr.collapsed_lines
                        } else {
                            0
                        }
                    }
                }
            };

        // Emit leading expanded lines (from_start)
        if !state.should_skip(expanded_line_count, expanded_line_count) {
            let base_unified = hunk.unified_line_start.wrapping_sub(leading.range_size);
            let base_split = hunk.split_line_start.wrapping_sub(leading.range_size);
            let base_del_idx = hunk.deletion_line_index.wrapping_sub(leading.range_size);
            let base_add_idx = hunk.addition_line_index.wrapping_sub(leading.range_size);
            let base_del_num = hunk.deletion_start.wrapping_sub(leading.range_size);
            let base_add_num = hunk.addition_start.wrapping_sub(leading.range_size);

            for index in 0..leading.from_start {
                if state.is_in_window(0, 0) {
                    let event = DiffLineEvent::ContextExpanded {
                        hunk_index,
                        collapsed_before: 0,
                        collapsed_after: 0,
                        deletion_line: DiffLineMetadata {
                            line_number: base_del_num + index,
                            line_index: base_del_idx + index,
                            no_eof_cr: false,
                            unified_line_index: base_unified + index,
                            split_line_index: base_split + index,
                        },
                        addition_line: DiffLineMetadata {
                            line_number: base_add_num + index,
                            line_index: base_add_idx + index,
                            no_eof_cr: false,
                            unified_line_index: base_unified + index,
                            split_line_index: base_split + index,
                        },
                    };
                    if callback(event) {
                        return true;
                    }
                }
                state.increment_counts(1, 1);
            }

            // Emit leading expanded lines (from_end)
            let fe_unified = hunk.unified_line_start.wrapping_sub(leading.from_end);
            let fe_split = hunk.split_line_start.wrapping_sub(leading.from_end);
            let fe_del_idx = hunk.deletion_line_index.wrapping_sub(leading.from_end);
            let fe_add_idx = hunk.addition_line_index.wrapping_sub(leading.from_end);
            let fe_del_num = hunk.deletion_start.wrapping_sub(leading.from_end);
            let fe_add_num = hunk.addition_start.wrapping_sub(leading.from_end);

            for index in 0..leading.from_end {
                if state.is_in_window(0, 0) {
                    let collapsed_before = if leading.collapsed_lines > 0 {
                        let v = leading.collapsed_lines;
                        leading.collapsed_lines = 0;
                        v
                    } else {
                        0
                    };
                    let event = DiffLineEvent::ContextExpanded {
                        hunk_index,
                        collapsed_before,
                        collapsed_after: 0,
                        deletion_line: DiffLineMetadata {
                            line_number: fe_del_num + index,
                            line_index: fe_del_idx + index,
                            no_eof_cr: false,
                            unified_line_index: fe_unified + index,
                            split_line_index: fe_split + index,
                        },
                        addition_line: DiffLineMetadata {
                            line_number: fe_add_num + index,
                            line_index: fe_add_idx + index,
                            no_eof_cr: false,
                            unified_line_index: fe_unified + index,
                            split_line_index: fe_split + index,
                        },
                    };
                    if callback(event) {
                        return true;
                    }
                }
                state.increment_counts(1, 1);
            }
        } else {
            state.increment_counts(expanded_line_count, expanded_line_count);
            // Consume pending collapsed
            leading.collapsed_lines = 0;
        }

        // Now iterate the hunk's own content
        let mut unified_line_index = hunk.unified_line_start;
        let mut split_line_index = hunk.split_line_start;
        let mut deletion_line_index = hunk.deletion_line_index;
        let mut addition_line_index = hunk.addition_line_index;
        let mut deletion_line_number = hunk.deletion_start;
        let mut addition_line_number = hunk.addition_start;
        let last_content_idx = hunk.hunk_content.len().checked_sub(1);

        for (content_idx, content) in hunk.hunk_content.iter().enumerate() {
            if state.should_break() {
                break 'hunk_iter;
            }

            let is_last_content = Some(content_idx) == last_content_idx;

            match content {
                HunkContent::Context(ctx) => {
                    if !state.should_skip(ctx.lines, ctx.lines) {
                        for index in 0..ctx.lines {
                            if state.is_in_window(0, 0) {
                                let is_last_line = is_last_content && index == ctx.lines - 1;
                                let u_idx = unified_line_index + index;
                                let s_idx = split_line_index + index;
                                let collapsed_before = if leading.collapsed_lines > 0 {
                                    let v = leading.collapsed_lines;
                                    leading.collapsed_lines = 0;
                                    v
                                } else {
                                    0
                                };
                                let event = DiffLineEvent::Context {
                                    hunk_index,
                                    collapsed_before,
                                    collapsed_after: get_trailing_collapsed_after(
                                        &trailing, u_idx, s_idx,
                                    ),
                                    deletion_line: DiffLineMetadata {
                                        line_number: deletion_line_number + index,
                                        line_index: deletion_line_index + index,
                                        no_eof_cr: is_last_line && hunk.no_eof_cr_deletions,
                                        unified_line_index: u_idx,
                                        split_line_index: s_idx,
                                    },
                                    addition_line: DiffLineMetadata {
                                        line_number: addition_line_number + index,
                                        line_index: addition_line_index + index,
                                        no_eof_cr: is_last_line && hunk.no_eof_cr_additions,
                                        unified_line_index: u_idx,
                                        split_line_index: s_idx,
                                    },
                                };
                                if callback(event) {
                                    return true;
                                }
                            }
                            state.increment_counts(1, 1);
                        }
                    } else {
                        state.increment_counts(ctx.lines, ctx.lines);
                        leading.collapsed_lines = 0;
                    }
                    unified_line_index += ctx.lines;
                    split_line_index += ctx.lines;
                    deletion_line_index += ctx.lines;
                    addition_line_index += ctx.lines;
                    deletion_line_number += ctx.lines;
                    addition_line_number += ctx.lines;
                }
                HunkContent::Change(change) => {
                    let split_count = change.deletions.max(change.additions);
                    let unified_count = change.deletions + change.additions;
                    let should_skip_change = state.should_skip(unified_count, split_count);

                    if !should_skip_change {
                        let iteration_ranges =
                            get_change_iteration_ranges(&state, change, props.diff_style);

                        for (range_start, range_end) in &iteration_ranges {
                            for index in *range_start..*range_end {
                                let event = get_change_line_data(
                                    hunk,
                                    hunk_index,
                                    &mut leading,
                                    &trailing,
                                    props.diff_style,
                                    index,
                                    unified_line_index,
                                    split_line_index,
                                    addition_line_index,
                                    deletion_line_index,
                                    addition_line_number,
                                    deletion_line_number,
                                    change,
                                    is_last_content,
                                    unified_count,
                                    split_count,
                                    &get_trailing_collapsed_after,
                                );
                                // Change events use silent emit in TS (don't increment
                                // inside emit) — counts are incremented after the loop
                                if callback(event) {
                                    return true;
                                }
                            }
                        }
                    }

                    leading.collapsed_lines = 0;
                    state.increment_counts(unified_count, split_count);
                    unified_line_index += unified_count;
                    split_line_index += split_count;
                    deletion_line_index += change.deletions;
                    addition_line_index += change.additions;
                    deletion_line_number += change.deletions;
                    addition_line_number += change.additions;
                }
            }
        }

        // Trailing expanded region (after last hunk only)
        if let Some(tr) = &trailing {
            let len = tr.from_start + tr.from_end;
            for index in 0..len {
                if state.should_break() {
                    break 'hunk_iter;
                }
                if state.is_in_window(0, 0) {
                    let is_last_line = index == len - 1;
                    let event = DiffLineEvent::ContextExpanded {
                        hunk_index: diff.hunks.len(),
                        collapsed_before: 0,
                        collapsed_after: if is_last_line { tr.collapsed_lines } else { 0 },
                        deletion_line: DiffLineMetadata {
                            line_number: deletion_line_number + index,
                            line_index: deletion_line_index + index,
                            no_eof_cr: false,
                            unified_line_index: unified_line_index + index,
                            split_line_index: split_line_index + index,
                        },
                        addition_line: DiffLineMetadata {
                            line_number: addition_line_number + index,
                            line_index: addition_line_index + index,
                            no_eof_cr: false,
                            unified_line_index: unified_line_index + index,
                            split_line_index: split_line_index + index,
                        },
                    };
                    if callback(event) {
                        return true;
                    }
                }
                state.increment_counts(1, 1);
            }
        }
    }

    false
}

// --- Internal helpers ---

struct ExpandedRegionResult {
    from_start: usize,
    from_end: usize,
    range_size: usize,
    collapsed_lines: usize,
}

fn get_expanded_region(
    is_partial: bool,
    range_size_raw: usize,
    expanded_hunks: Option<&HashMap<usize, HunkExpansionRegion>>,
    expand_all: bool,
    hunk_index: usize,
    collapsed_context_threshold: usize,
) -> ExpandedRegionResult {
    let range_size = range_size_raw;
    if range_size == 0 || is_partial {
        return ExpandedRegionResult {
            from_start: 0,
            from_end: 0,
            range_size,
            collapsed_lines: range_size,
        };
    }
    if expand_all || range_size <= collapsed_context_threshold {
        return ExpandedRegionResult {
            from_start: range_size,
            from_end: 0,
            range_size,
            collapsed_lines: 0,
        };
    }
    let region = expanded_hunks.and_then(|m| m.get(&hunk_index));
    let from_start = region.map_or(0, |r| r.from_start).min(range_size);
    let from_end = region.map_or(0, |r| r.from_end).min(range_size);
    let expanded_count = from_start + from_end;
    if expanded_count >= range_size {
        ExpandedRegionResult {
            from_start: range_size,
            from_end: 0,
            range_size,
            collapsed_lines: 0,
        }
    } else {
        ExpandedRegionResult {
            from_start,
            from_end,
            range_size,
            collapsed_lines: range_size.saturating_sub(expanded_count),
        }
    }
}

fn has_final_collapsed_hunk(diff: &FileDiffMetadata) -> bool {
    let last_hunk = match diff.hunks.last() {
        Some(h) => h,
        None => return false,
    };
    if diff.is_partial || diff.addition_lines.is_empty() || diff.deletion_lines.is_empty() {
        return false;
    }
    last_hunk.addition_line_index + last_hunk.addition_count < diff.addition_lines.len()
        || last_hunk.deletion_line_index + last_hunk.deletion_count < diff.deletion_lines.len()
}

/// Compute visible iteration ranges for a change block.
fn get_change_iteration_ranges(
    state: &IterationState,
    content: &ChangeContent,
    diff_style: DiffStyle,
) -> Vec<(usize, usize)> {
    if !state.is_windowed {
        let end = match diff_style {
            DiffStyle::Unified => content.deletions + content.additions,
            _ => content.deletions.max(content.additions),
        };
        return vec![(0, end)];
    }

    let use_unified = diff_style != DiffStyle::Split;
    let use_split = diff_style != DiffStyle::Unified;
    let iteration_space_is_unified = diff_style == DiffStyle::Unified;

    let mut ranges: Vec<(usize, usize)> = Vec::new();

    let get_visible_range = |start: usize, count: usize| -> Option<(usize, usize)> {
        let end = start + count;
        if end <= state.viewport_start || start >= state.viewport_end {
            return None;
        }
        let visible_start = state.viewport_start.saturating_sub(start);
        let visible_end = count.min(state.viewport_end.saturating_sub(start));
        if visible_end > visible_start {
            Some((visible_start, visible_end))
        } else {
            None
        }
    };

    let map_range = |range: (usize, usize), is_additions: bool| -> (usize, usize) {
        if !iteration_space_is_unified {
            return range;
        }
        if is_additions {
            (range.0 + content.deletions, range.1 + content.deletions)
        } else {
            range
        }
    };

    if use_unified {
        if let Some(r) = get_visible_range(state.unified_count, content.deletions) {
            let mapped = map_range(r, false);
            if mapped.1 > mapped.0 {
                ranges.push(mapped);
            }
        }
        if let Some(r) =
            get_visible_range(state.unified_count + content.deletions, content.additions)
        {
            let mapped = map_range(r, true);
            if mapped.1 > mapped.0 {
                ranges.push(mapped);
            }
        }
    }

    if use_split {
        if let Some(r) = get_visible_range(state.split_count, content.deletions) {
            let mapped = map_range(r, false);
            if mapped.1 > mapped.0 {
                ranges.push(mapped);
            }
        }
        if let Some(r) = get_visible_range(state.split_count, content.additions) {
            let mapped = map_range(r, true);
            if mapped.1 > mapped.0 {
                ranges.push(mapped);
            }
        }
    }

    if ranges.is_empty() {
        return ranges;
    }

    // Sort and merge overlapping ranges
    ranges.sort_unstable_by_key(|r| r.0);
    let mut merged: Vec<(usize, usize)> = vec![ranges[0]];
    for &(start, end) in &ranges[1..] {
        let last = merged.last_mut().unwrap();
        if start <= last.1 {
            last.1 = last.1.max(end);
        } else {
            merged.push((start, end));
        }
    }
    merged
}

#[allow(clippy::too_many_arguments)]
fn get_change_line_data<F>(
    hunk: &Hunk,
    hunk_index: usize,
    leading: &mut ExpandedRegionResult,
    trailing: &Option<ExpandedRegionResult>,
    diff_style: DiffStyle,
    index: usize,
    unified_line_index: usize,
    split_line_index: usize,
    addition_line_index: usize,
    deletion_line_index: usize,
    addition_line_number: usize,
    deletion_line_number: usize,
    content: &ChangeContent,
    is_last_content: bool,
    unified_count: usize,
    split_count: usize,
    get_trailing_collapsed_after: &F,
) -> DiffLineEvent
where
    F: Fn(&Option<ExpandedRegionResult>, usize, usize) -> usize,
{
    let is_unified = diff_style == DiffStyle::Unified;

    // Compute unified line indices for deletion and addition
    let unified_del_idx = if index < content.deletions {
        Some(unified_line_index + index)
    } else {
        None
    };
    let unified_add_idx = if is_unified {
        if index >= content.deletions {
            Some(unified_line_index + index)
        } else {
            None
        }
    } else if index < content.additions {
        Some(unified_line_index + content.deletions + index)
    } else {
        None
    };

    let resolved_split_idx = if is_unified {
        split_line_index
            + if index < content.deletions {
                index
            } else {
                index - content.deletions
            }
    } else {
        split_line_index + index
    };

    // Deletion line info
    let del_line_idx = if index < content.deletions {
        Some(deletion_line_index + index)
    } else {
        None
    };
    let del_line_num = if index < content.deletions {
        Some(deletion_line_number + index)
    } else {
        None
    };

    // Addition line info
    let add_line_idx = if is_unified {
        if index >= content.deletions {
            Some(addition_line_index + (index - content.deletions))
        } else {
            None
        }
    } else if index < content.additions {
        Some(addition_line_index + index)
    } else {
        None
    };
    let add_line_num = if is_unified {
        if index >= content.deletions {
            Some(addition_line_number + (index - content.deletions))
        } else {
            None
        }
    } else if index < content.additions {
        Some(addition_line_number + index)
    } else {
        None
    };

    // EOF newline flags
    let no_eof_cr_deletion = if is_unified {
        is_last_content && index == content.deletions.wrapping_sub(1) && hunk.no_eof_cr_deletions
    } else {
        is_last_content && index == split_count.wrapping_sub(1) && hunk.no_eof_cr_deletions
    };
    let no_eof_cr_addition = if is_unified {
        is_last_content && index == unified_count.wrapping_sub(1) && hunk.no_eof_cr_additions
    } else {
        is_last_content && index == split_count.wrapping_sub(1) && hunk.no_eof_cr_additions
    };

    let collapsed_before = if leading.collapsed_lines > 0 {
        let v = leading.collapsed_lines;
        leading.collapsed_lines = 0;
        v
    } else {
        0
    };

    let u_idx = unified_line_index + index;
    let collapsed_after = get_trailing_collapsed_after(trailing, u_idx, resolved_split_idx);

    // Build the deletion/addition metadata
    let deletion_line = match (del_line_idx, del_line_num, unified_del_idx) {
        (Some(li), Some(ln), Some(ui)) => Some(DiffLineMetadata {
            line_number: ln,
            line_index: li,
            no_eof_cr: no_eof_cr_deletion,
            unified_line_index: ui,
            split_line_index: resolved_split_idx,
        }),
        _ => None,
    };

    let addition_line = match (add_line_idx, add_line_num, unified_add_idx) {
        (Some(li), Some(ln), Some(ui)) => Some(DiffLineMetadata {
            line_number: ln,
            line_index: li,
            no_eof_cr: no_eof_cr_addition,
            unified_line_index: ui,
            split_line_index: resolved_split_idx,
        }),
        _ => None,
    };

    DiffLineEvent::Change {
        hunk_index,
        collapsed_before,
        collapsed_after,
        deletion_line,
        addition_line,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::*;

    /// Build a simple diff: 3 context, 2 deletions + 2 additions, 3 context.
    fn simple_diff() -> FileDiffMetadata {
        FileDiffMetadata {
            name: "test.rs".into(),
            prev_name: None,
            change_type: ChangeType::Change,
            hunks: vec![Hunk {
                collapsed_before: 0,
                addition_start: 1,
                addition_count: 8,
                addition_lines: 2,
                addition_line_index: 0,
                deletion_start: 1,
                deletion_count: 8,
                deletion_lines: 2,
                deletion_line_index: 0,
                hunk_content: vec![
                    HunkContent::Context(ContextContent {
                        lines: 3,
                        addition_line_index: 0,
                        deletion_line_index: 0,
                    }),
                    HunkContent::Change(ChangeContent {
                        deletions: 2,
                        deletion_line_index: 3,
                        additions: 2,
                        addition_line_index: 3,
                    }),
                    HunkContent::Context(ContextContent {
                        lines: 3,
                        addition_line_index: 5,
                        deletion_line_index: 5,
                    }),
                ],
                hunk_context: None,
                hunk_specs: Some("@@ -1,8 +1,8 @@".into()),
                split_line_start: 0,
                split_line_count: 8,
                unified_line_start: 0,
                unified_line_count: 10,
                no_eof_cr_deletions: false,
                no_eof_cr_additions: false,
            }],
            split_line_count: 8,
            unified_line_count: 10,
            is_partial: true,
            deletion_lines: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "old1".into(),
                "old2".into(),
                "d".into(),
                "e".into(),
                "f".into(),
            ],
            addition_lines: vec![
                "a".into(),
                "b".into(),
                "c".into(),
                "new1".into(),
                "new2".into(),
                "d".into(),
                "e".into(),
                "f".into(),
            ],
        }
    }

    #[test]
    fn unified_iteration_produces_correct_sequence() {
        let diff = simple_diff();
        let props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        let mut events = Vec::new();
        iterate_over_diff(&props, |e| {
            events.push(e);
            false
        });

        // 3 context + 2 deletions + 2 additions + 3 context = 10 events
        assert_eq!(events.len(), 10, "unified should emit 10 events");

        // First 3 are context
        for i in 0..3 {
            assert!(
                matches!(&events[i], DiffLineEvent::Context { .. }),
                "event {i} should be Context"
            );
        }

        // Next 4 are changes (2 del + 2 add in unified)
        for i in 3..7 {
            assert!(
                matches!(&events[i], DiffLineEvent::Change { .. }),
                "event {i} should be Change"
            );
        }

        // In unified mode, first 2 changes have deletion_line only
        if let DiffLineEvent::Change {
            deletion_line,
            addition_line,
            ..
        } = &events[3]
        {
            assert!(deletion_line.is_some(), "first change should have deletion");
            assert!(
                addition_line.is_none(),
                "first change should not have addition in unified"
            );
        }

        // Last 2 changes have addition_line only
        if let DiffLineEvent::Change {
            deletion_line,
            addition_line,
            ..
        } = &events[5]
        {
            assert!(
                deletion_line.is_none(),
                "third change should not have deletion in unified"
            );
            assert!(addition_line.is_some(), "third change should have addition");
        }

        // Last 3 are context
        for i in 7..10 {
            assert!(
                matches!(&events[i], DiffLineEvent::Context { .. }),
                "event {i} should be Context"
            );
        }
    }

    #[test]
    fn split_iteration_pairs_deletions_and_additions() {
        let diff = simple_diff();
        let props = IterateOverDiffProps::new(&diff, DiffStyle::Split);
        let mut events = Vec::new();
        iterate_over_diff(&props, |e| {
            events.push(e);
            false
        });

        // 3 context + 2 paired changes + 3 context = 8 events
        assert_eq!(events.len(), 8, "split should emit 8 events");

        // Change events should have both deletion and addition
        if let DiffLineEvent::Change {
            deletion_line,
            addition_line,
            ..
        } = &events[3]
        {
            assert!(deletion_line.is_some(), "split change should pair deletion");
            assert!(addition_line.is_some(), "split change should pair addition");
        }
    }

    #[test]
    fn windowed_iteration_skips_and_breaks() {
        let diff = simple_diff();
        let mut props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        props.starting_line = 2;
        props.total_lines = 4;

        let mut events = Vec::new();
        iterate_over_diff(&props, |e| {
            events.push(e);
            false
        });

        // Window starts at line 2 (3rd line), shows 4 lines
        // Should get: context[2], change-del[0], change-del[1], change-add[0]
        assert_eq!(events.len(), 4, "windowed should emit exactly 4 events");

        // First event should be context (the 3rd context line)
        assert!(matches!(&events[0], DiffLineEvent::Context { .. }));
    }

    #[test]
    fn collapsed_context_between_hunks() {
        let diff = FileDiffMetadata {
            name: "test.rs".into(),
            prev_name: None,
            change_type: ChangeType::Change,
            hunks: vec![
                Hunk {
                    collapsed_before: 0,
                    addition_start: 1,
                    addition_count: 2,
                    addition_lines: 1,
                    addition_line_index: 0,
                    deletion_start: 1,
                    deletion_count: 2,
                    deletion_lines: 1,
                    deletion_line_index: 0,
                    hunk_content: vec![
                        HunkContent::Context(ContextContent {
                            lines: 1,
                            addition_line_index: 0,
                            deletion_line_index: 0,
                        }),
                        HunkContent::Change(ChangeContent {
                            deletions: 1,
                            deletion_line_index: 1,
                            additions: 1,
                            addition_line_index: 1,
                        }),
                    ],
                    hunk_context: None,
                    hunk_specs: None,
                    split_line_start: 0,
                    split_line_count: 2,
                    unified_line_start: 0,
                    unified_line_count: 3,
                    no_eof_cr_deletions: false,
                    no_eof_cr_additions: false,
                },
                Hunk {
                    collapsed_before: 10,
                    addition_start: 15,
                    addition_count: 2,
                    addition_lines: 1,
                    addition_line_index: 5,
                    deletion_start: 15,
                    deletion_count: 2,
                    deletion_lines: 1,
                    deletion_line_index: 5,
                    hunk_content: vec![
                        HunkContent::Context(ContextContent {
                            lines: 1,
                            addition_line_index: 5,
                            deletion_line_index: 5,
                        }),
                        HunkContent::Change(ChangeContent {
                            deletions: 1,
                            deletion_line_index: 6,
                            additions: 1,
                            addition_line_index: 6,
                        }),
                    ],
                    hunk_context: None,
                    hunk_specs: None,
                    split_line_start: 2,
                    split_line_count: 2,
                    unified_line_start: 3,
                    unified_line_count: 3,
                    no_eof_cr_deletions: false,
                    no_eof_cr_additions: false,
                },
            ],
            split_line_count: 4,
            unified_line_count: 6,
            is_partial: true,
            deletion_lines: (0..8).map(|i| format!("del{i}")).collect(),
            addition_lines: (0..8).map(|i| format!("add{i}")).collect(),
        };

        let props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        let mut collapsed_values = Vec::new();
        iterate_over_diff(&props, |e| {
            let cb = match &e {
                DiffLineEvent::Context {
                    collapsed_before, ..
                } => *collapsed_before,
                DiffLineEvent::Change {
                    collapsed_before, ..
                } => *collapsed_before,
                DiffLineEvent::ContextExpanded {
                    collapsed_before, ..
                } => *collapsed_before,
            };
            if cb > 0 {
                collapsed_values.push(cb);
            }
            false
        });

        // Second hunk has collapsed_before=10, is_partial=true so no expansion
        // → collapsed_before should appear once with value 10
        assert_eq!(collapsed_values, vec![10]);
    }

    #[test]
    fn line_numbers_correct_in_unified() {
        let diff = simple_diff();
        let props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        let mut line_numbers = Vec::new();
        iterate_over_diff(&props, |e| {
            match &e {
                DiffLineEvent::Context {
                    deletion_line,
                    addition_line,
                    ..
                } => {
                    line_numbers.push(('C', deletion_line.line_number, addition_line.line_number));
                }
                DiffLineEvent::Change {
                    deletion_line,
                    addition_line,
                    ..
                } => {
                    let del = deletion_line.map(|d| d.line_number).unwrap_or(0);
                    let add = addition_line.map(|a| a.line_number).unwrap_or(0);
                    line_numbers.push(('X', del, add));
                }
                _ => {}
            }
            false
        });

        // Context lines 1,2,3 (both sides same)
        assert_eq!(line_numbers[0], ('C', 1, 1));
        assert_eq!(line_numbers[1], ('C', 2, 2));
        assert_eq!(line_numbers[2], ('C', 3, 3));

        // Change: deletions first (lines 4, 5), then additions (lines 4, 5)
        assert_eq!(line_numbers[3], ('X', 4, 0)); // deletion line 4
        assert_eq!(line_numbers[4], ('X', 5, 0)); // deletion line 5
        assert_eq!(line_numbers[5], ('X', 0, 4)); // addition line 4
        assert_eq!(line_numbers[6], ('X', 0, 5)); // addition line 5

        // Context lines 6,7,8
        assert_eq!(line_numbers[7], ('C', 6, 6));
        assert_eq!(line_numbers[8], ('C', 7, 7));
        assert_eq!(line_numbers[9], ('C', 8, 8));
    }

    #[test]
    fn split_unmatched_change_lines() {
        // 3 deletions, 1 addition → split has 3 rows, last 2 deletion-only
        let diff = FileDiffMetadata {
            name: "test.rs".into(),
            prev_name: None,
            change_type: ChangeType::Change,
            hunks: vec![Hunk {
                collapsed_before: 0,
                addition_start: 1,
                addition_count: 1,
                addition_lines: 1,
                addition_line_index: 0,
                deletion_start: 1,
                deletion_count: 3,
                deletion_lines: 3,
                deletion_line_index: 0,
                hunk_content: vec![HunkContent::Change(ChangeContent {
                    deletions: 3,
                    deletion_line_index: 0,
                    additions: 1,
                    addition_line_index: 0,
                })],
                hunk_context: None,
                hunk_specs: None,
                split_line_start: 0,
                split_line_count: 3,
                unified_line_start: 0,
                unified_line_count: 4,
                no_eof_cr_deletions: false,
                no_eof_cr_additions: false,
            }],
            split_line_count: 3,
            unified_line_count: 4,
            is_partial: true,
            deletion_lines: vec!["a".into(), "b".into(), "c".into()],
            addition_lines: vec!["x".into()],
        };

        let props = IterateOverDiffProps::new(&diff, DiffStyle::Split);
        let mut events = Vec::new();
        iterate_over_diff(&props, |e| {
            events.push(e);
            false
        });

        assert_eq!(events.len(), 3, "split: max(3 del, 1 add) = 3 rows");

        // First row: paired (del + add)
        if let DiffLineEvent::Change {
            deletion_line,
            addition_line,
            ..
        } = &events[0]
        {
            assert!(deletion_line.is_some());
            assert!(addition_line.is_some());
        }

        // Rows 2 and 3: deletion only
        for i in 1..3 {
            if let DiffLineEvent::Change {
                deletion_line,
                addition_line,
                ..
            } = &events[i]
            {
                assert!(deletion_line.is_some(), "row {i} should have deletion");
                assert!(addition_line.is_none(), "row {i} should not have addition");
            }
        }
    }

    #[test]
    fn callback_early_stop() {
        let diff = simple_diff();
        let props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        let mut count = 0;
        let stopped = iterate_over_diff(&props, |_| {
            count += 1;
            count >= 3 // stop after 3 events
        });
        assert!(stopped, "should report early stop");
        assert_eq!(count, 3, "should have emitted exactly 3 events");
    }

    #[test]
    fn empty_diff_produces_no_events() {
        let diff = FileDiffMetadata {
            name: "empty.rs".into(),
            prev_name: None,
            change_type: ChangeType::Change,
            hunks: vec![],
            split_line_count: 0,
            unified_line_count: 0,
            is_partial: true,
            deletion_lines: vec![],
            addition_lines: vec![],
        };
        let props = IterateOverDiffProps::new(&diff, DiffStyle::Unified);
        let mut count = 0;
        iterate_over_diff(&props, |_| {
            count += 1;
            false
        });
        assert_eq!(count, 0);
    }

    #[test]
    fn expanded_region_auto_expands_small_threshold() {
        let result = get_expanded_region(false, 1, None, false, 0, 1);
        assert_eq!(result.from_start, 1);
        assert_eq!(result.collapsed_lines, 0);
    }

    #[test]
    fn expanded_region_partial_stays_collapsed() {
        let result = get_expanded_region(true, 10, None, false, 0, 1);
        assert_eq!(result.from_start, 0);
        assert_eq!(result.from_end, 0);
        assert_eq!(result.collapsed_lines, 10);
    }

    #[test]
    fn expanded_region_with_expansion_map() {
        let mut map = HashMap::new();
        map.insert(
            0,
            HunkExpansionRegion {
                from_start: 3,
                from_end: 2,
            },
        );
        let result = get_expanded_region(false, 20, Some(&map), false, 0, 1);
        assert_eq!(result.from_start, 3);
        assert_eq!(result.from_end, 2);
        assert_eq!(result.collapsed_lines, 15);
    }
}
