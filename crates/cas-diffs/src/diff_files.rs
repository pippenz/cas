//! Two-file diff generation.
//!
//! Produces a [`FileDiffMetadata`] from two file contents by diffing at
//! line granularity with `similar::TextDiff`.

use similar::{DiffOp, TextDiff};

use crate::{ChangeContent, ChangeType, ContextContent, FileDiffMetadata, Hunk, HunkContent};

/// Default number of context lines around changes in generated diffs.
const DEFAULT_CONTEXT: usize = 3;

/// Generate a [`FileDiffMetadata`] by diffing two file contents.
///
/// Uses `similar::TextDiff` at line-level granularity and groups changes
/// into hunks with a 3-line context window (matching `git diff` defaults).
///
/// The returned metadata has `is_partial = false` — `deletion_lines` and
/// `addition_lines` contain the complete old and new file contents.
pub fn diff_files(
    old_name: &str,
    old_content: &str,
    new_name: &str,
    new_content: &str,
) -> FileDiffMetadata {
    let diff = TextDiff::from_lines(old_content, new_content);

    let deletion_lines = split_lines_keep_newline(old_content);
    let addition_lines = split_lines_keep_newline(new_content);

    let change_type = determine_change_type(old_name, old_content, new_name, new_content);
    let prev_name = if old_name != new_name {
        Some(old_name.to_string())
    } else {
        None
    };

    let groups = diff.grouped_ops(DEFAULT_CONTEXT);
    let mut hunks = Vec::new();
    let mut split_total: usize = 0;
    let mut unified_total: usize = 0;
    let mut prev_hunk_old_end: usize = 0;

    let old_no_eof = !old_content.is_empty() && !old_content.ends_with('\n');
    let new_no_eof = !new_content.is_empty() && !new_content.ends_with('\n');
    let group_count = groups.len();

    for (gi, group) in groups.iter().enumerate() {
        if group.is_empty() {
            continue;
        }

        let first_op = &group[0];
        let last_op = &group[group.len() - 1];

        let hunk_old_start = op_old_start(first_op);
        let hunk_old_end = op_old_end(last_op);
        let hunk_new_start = op_new_start(first_op);
        let hunk_new_end = op_new_end(last_op);

        let collapsed_before = hunk_old_start - prev_hunk_old_end;
        prev_hunk_old_end = hunk_old_end;

        let mut hunk_content = Vec::new();
        let mut del_line_count: usize = 0;
        let mut add_line_count: usize = 0;

        for op in group {
            match *op {
                DiffOp::Equal {
                    old_index,
                    new_index,
                    len,
                } => {
                    hunk_content.push(HunkContent::Context(ContextContent {
                        lines: len,
                        deletion_line_index: old_index,
                        addition_line_index: new_index,
                    }));
                }
                DiffOp::Delete {
                    old_index,
                    old_len,
                    new_index,
                } => {
                    hunk_content.push(HunkContent::Change(ChangeContent {
                        deletions: old_len,
                        deletion_line_index: old_index,
                        additions: 0,
                        addition_line_index: new_index,
                    }));
                    del_line_count += old_len;
                }
                DiffOp::Insert {
                    old_index,
                    new_index,
                    new_len,
                } => {
                    hunk_content.push(HunkContent::Change(ChangeContent {
                        deletions: 0,
                        deletion_line_index: old_index,
                        additions: new_len,
                        addition_line_index: new_index,
                    }));
                    add_line_count += new_len;
                }
                DiffOp::Replace {
                    old_index,
                    old_len,
                    new_index,
                    new_len,
                } => {
                    hunk_content.push(HunkContent::Change(ChangeContent {
                        deletions: old_len,
                        deletion_line_index: old_index,
                        additions: new_len,
                        addition_line_index: new_index,
                    }));
                    del_line_count += old_len;
                    add_line_count += new_len;
                }
            }
        }

        let deletion_count = hunk_old_end - hunk_old_start;
        let addition_count = hunk_new_end - hunk_new_start;

        // 1-based line numbers; 0 when the side has no lines (git convention).
        let deletion_start = if deletion_count == 0 {
            0
        } else {
            hunk_old_start + 1
        };
        let addition_start = if addition_count == 0 {
            0
        } else {
            hunk_new_start + 1
        };

        let (split_count, unified_count) = compute_hunk_line_counts(&hunk_content);

        let hunk_specs = format!(
            "@@ -{},{} +{},{} @@",
            deletion_start, deletion_count, addition_start, addition_count
        );

        let is_last_group = gi == group_count - 1;

        hunks.push(Hunk {
            collapsed_before,
            addition_start,
            addition_count,
            addition_lines: add_line_count,
            addition_line_index: hunk_new_start,
            deletion_start,
            deletion_count,
            deletion_lines: del_line_count,
            deletion_line_index: hunk_old_start,
            hunk_content,
            hunk_context: None,
            hunk_specs: Some(hunk_specs),
            split_line_start: split_total,
            split_line_count: split_count,
            unified_line_start: unified_total,
            unified_line_count: unified_count,
            no_eof_cr_deletions: if is_last_group { old_no_eof } else { false },
            no_eof_cr_additions: if is_last_group { new_no_eof } else { false },
        });

        split_total += split_count;
        unified_total += unified_count;
    }

    FileDiffMetadata {
        name: new_name.to_string(),
        prev_name,
        change_type,
        hunks,
        split_line_count: split_total,
        unified_line_count: unified_total,
        is_partial: false,
        deletion_lines,
        addition_lines,
    }
}

fn determine_change_type(
    old_name: &str,
    old_content: &str,
    new_name: &str,
    new_content: &str,
) -> ChangeType {
    if old_content.is_empty() && !new_content.is_empty() {
        return ChangeType::New;
    }
    if !old_content.is_empty() && new_content.is_empty() {
        return ChangeType::Deleted;
    }
    let renamed = old_name != new_name;
    let changed = old_content != new_content;
    match (renamed, changed) {
        (true, true) => ChangeType::RenameChanged,
        (true, false) => ChangeType::RenamePure,
        (false, _) => ChangeType::Change,
    }
}

/// Split content into lines, keeping the trailing `\n` on each line.
/// Matches the tokenisation that `similar::TextDiff::from_lines` uses.
fn split_lines_keep_newline(content: &str) -> Vec<String> {
    if content.is_empty() {
        return vec![];
    }
    content.split_inclusive('\n').map(String::from).collect()
}

fn op_old_start(op: &DiffOp) -> usize {
    match *op {
        DiffOp::Equal { old_index, .. }
        | DiffOp::Delete { old_index, .. }
        | DiffOp::Insert { old_index, .. }
        | DiffOp::Replace { old_index, .. } => old_index,
    }
}

fn op_old_end(op: &DiffOp) -> usize {
    match *op {
        DiffOp::Equal { old_index, len, .. } => old_index + len,
        DiffOp::Delete {
            old_index, old_len, ..
        } => old_index + old_len,
        DiffOp::Insert { old_index, .. } => old_index,
        DiffOp::Replace {
            old_index, old_len, ..
        } => old_index + old_len,
    }
}

fn op_new_start(op: &DiffOp) -> usize {
    match *op {
        DiffOp::Equal { new_index, .. }
        | DiffOp::Delete { new_index, .. }
        | DiffOp::Insert { new_index, .. }
        | DiffOp::Replace { new_index, .. } => new_index,
    }
}

fn op_new_end(op: &DiffOp) -> usize {
    match *op {
        DiffOp::Equal { new_index, len, .. } => new_index + len,
        DiffOp::Delete { new_index, .. } => new_index,
        DiffOp::Insert {
            new_index, new_len, ..
        } => new_index + new_len,
        DiffOp::Replace {
            new_index, new_len, ..
        } => new_index + new_len,
    }
}

/// Compute split and unified line counts for a hunk's content blocks.
fn compute_hunk_line_counts(content: &[HunkContent]) -> (usize, usize) {
    let mut split = 0;
    let mut unified = 0;
    for block in content {
        match block {
            HunkContent::Context(ctx) => {
                split += ctx.lines;
                unified += ctx.lines;
            }
            HunkContent::Change(change) => {
                split += change.deletions.max(change.additions);
                unified += change.deletions + change.additions;
            }
        }
    }
    (split, unified)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_simple_change() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nmodified\nline3\n";
        let meta = diff_files("a.txt", old, "a.txt", new);

        assert_eq!(meta.name, "a.txt");
        assert_eq!(meta.change_type, ChangeType::Change);
        assert!(!meta.is_partial);
        assert!(meta.prev_name.is_none());

        assert_eq!(meta.deletion_lines.len(), 3);
        assert_eq!(meta.addition_lines.len(), 3);

        assert_eq!(meta.hunks.len(), 1);
        let hunk = &meta.hunks[0];
        assert_eq!(hunk.deletion_lines, 1); // "line2" removed
        assert_eq!(hunk.addition_lines, 1); // "modified" added
    }

    #[test]
    fn diff_new_file() {
        let meta = diff_files("a.txt", "", "a.txt", "hello\nworld\n");

        assert_eq!(meta.change_type, ChangeType::New);
        assert!(meta.deletion_lines.is_empty());
        assert_eq!(meta.addition_lines.len(), 2);
        assert_eq!(meta.hunks.len(), 1);
        assert_eq!(meta.hunks[0].addition_lines, 2);
        assert_eq!(meta.hunks[0].deletion_lines, 0);
    }

    #[test]
    fn diff_deleted_file() {
        let meta = diff_files("a.txt", "hello\nworld\n", "a.txt", "");

        assert_eq!(meta.change_type, ChangeType::Deleted);
        assert_eq!(meta.deletion_lines.len(), 2);
        assert!(meta.addition_lines.is_empty());
        assert_eq!(meta.hunks.len(), 1);
        assert_eq!(meta.hunks[0].deletion_lines, 2);
        assert_eq!(meta.hunks[0].addition_lines, 0);
    }

    #[test]
    fn diff_identical_files() {
        let content = "line1\nline2\nline3\n";
        let meta = diff_files("a.txt", content, "a.txt", content);

        assert_eq!(meta.change_type, ChangeType::Change);
        assert!(meta.hunks.is_empty());
        assert_eq!(meta.split_line_count, 0);
        assert_eq!(meta.unified_line_count, 0);
    }

    #[test]
    fn diff_rename_pure() {
        let content = "same\n";
        let meta = diff_files("old.txt", content, "new.txt", content);

        assert_eq!(meta.change_type, ChangeType::RenamePure);
        assert_eq!(meta.prev_name.as_deref(), Some("old.txt"));
        assert_eq!(meta.name, "new.txt");
        assert!(meta.hunks.is_empty());
    }

    #[test]
    fn diff_rename_changed() {
        let meta = diff_files("old.txt", "a\n", "new.txt", "b\n");

        assert_eq!(meta.change_type, ChangeType::RenameChanged);
        assert_eq!(meta.prev_name.as_deref(), Some("old.txt"));
    }

    #[test]
    fn hunk_specs_format() {
        let old = "a\nb\nc\n";
        let new = "a\nx\nc\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.hunks.len(), 1);
        let specs = meta.hunks[0].hunk_specs.as_deref().unwrap();
        assert!(
            specs.starts_with("@@ -") && specs.ends_with(" @@"),
            "unexpected hunk specs: {specs}"
        );
    }

    #[test]
    fn multiple_hunks_with_gap() {
        // Create a file where changes are far apart so they form separate hunks.
        let mut old_lines: Vec<String> = (1..=20).map(|i| format!("line{i}\n")).collect();
        let mut new_lines = old_lines.clone();
        // Change line 2 and line 19 (far apart, > 2*context apart)
        old_lines[1] = "old_line2\n".to_string();
        new_lines[1] = "new_line2\n".to_string();
        old_lines[18] = "old_line19\n".to_string();
        new_lines[18] = "new_line19\n".to_string();

        let old: String = old_lines.into_iter().collect();
        let new: String = new_lines.into_iter().collect();
        let meta = diff_files("f.rs", &old, "f.rs", &new);

        assert!(
            meta.hunks.len() >= 2,
            "expected at least 2 hunks, got {}",
            meta.hunks.len()
        );

        // Second hunk should have collapsed_before > 0
        assert!(meta.hunks[1].collapsed_before > 0);

        // Split/unified totals should be sum of hunk counts
        let split_sum: usize = meta.hunks.iter().map(|h| h.split_line_count).sum();
        let unified_sum: usize = meta.hunks.iter().map(|h| h.unified_line_count).sum();
        assert_eq!(meta.split_line_count, split_sum);
        assert_eq!(meta.unified_line_count, unified_sum);
    }

    #[test]
    fn hunk_content_blocks() {
        let old = "ctx1\nold\nctx2\n";
        let new = "ctx1\nnew\nctx2\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.hunks.len(), 1);
        let blocks = &meta.hunks[0].hunk_content;
        // Should have: context, change, context
        assert_eq!(blocks.len(), 3);
        assert!(matches!(blocks[0], HunkContent::Context(_)));
        assert!(matches!(blocks[1], HunkContent::Change(_)));
        assert!(matches!(blocks[2], HunkContent::Context(_)));
    }

    #[test]
    fn split_vs_unified_counts() {
        // 2 deletions replaced by 3 additions
        let old = "a\nb\nc\n";
        let new = "a\nx\ny\nz\nc\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.hunks.len(), 1);
        let hunk = &meta.hunks[0];
        // Change block: 1 deletion ("b"), 3 additions ("x","y","z")
        // Context: "a" (1) + "c" (1) = 2
        // Split: 1 ctx + max(1,3)=3 + 1 ctx = 5
        // Unified: 1 ctx + 1+3=4 + 1 ctx = 6
        assert_eq!(hunk.split_line_count, 5);
        assert_eq!(hunk.unified_line_count, 6);
    }

    #[test]
    fn no_eof_newline_flags() {
        let old = "hello\nworld"; // no trailing newline
        let new = "hello\nearth\n"; // trailing newline
        let meta = diff_files("f.rs", old, "f.rs", new);

        let last_hunk = meta.hunks.last().unwrap();
        assert!(last_hunk.no_eof_cr_deletions);
        assert!(!last_hunk.no_eof_cr_additions);
    }

    #[test]
    fn deletion_lines_and_addition_lines_match_content() {
        let old = "line1\nline2\n";
        let new = "line1\nchanged\nline3\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.deletion_lines, vec!["line1\n", "line2\n"]);
        assert_eq!(meta.addition_lines, vec!["line1\n", "changed\n", "line3\n"]);
    }

    #[test]
    fn split_lines_keep_newline_basic() {
        assert_eq!(
            split_lines_keep_newline("a\nb\nc\n"),
            vec!["a\n", "b\n", "c\n"]
        );
    }

    #[test]
    fn split_lines_keep_newline_no_trailing() {
        assert_eq!(split_lines_keep_newline("a\nb"), vec!["a\n", "b"]);
    }

    #[test]
    fn split_lines_keep_newline_empty() {
        assert!(split_lines_keep_newline("").is_empty());
    }

    #[test]
    fn pure_insertion_hunk() {
        let old = "a\nc\n";
        let new = "a\nb\nc\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.hunks.len(), 1);
        let hunk = &meta.hunks[0];
        assert_eq!(hunk.addition_lines, 1);
        assert_eq!(hunk.deletion_lines, 0);
    }

    #[test]
    fn pure_deletion_hunk() {
        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        let meta = diff_files("f.rs", old, "f.rs", new);

        assert_eq!(meta.hunks.len(), 1);
        let hunk = &meta.hunks[0];
        assert_eq!(hunk.deletion_lines, 1);
        assert_eq!(hunk.addition_lines, 0);
    }
}
