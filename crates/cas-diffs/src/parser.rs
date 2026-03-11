//! Unified diff/patch parser.
//!
//! Parses git and unified diff output into structured [`ParsedPatch`] data.
//! Ported from `@pierre/diffs` `parsePatchFiles.ts`.

use regex::Regex;
use std::sync::LazyLock;

use crate::{
    ChangeContent, ChangeType, ContextContent, FileDiffMetadata, Hunk, HunkContent, HunkLineType,
    ParsedPatch,
};

// ---------------------------------------------------------------------------
// Regex patterns
// ---------------------------------------------------------------------------

/// Match `From <sha> ...` lines at start of a line (for splitting multi-commit patches).
static COMMIT_METADATA_SPLIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^From [a-f0-9]+ .+$").unwrap());

/// Match `diff --git ` at start of a line (for splitting into per-file sections).
static GIT_DIFF_FILE_BREAK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^diff --git ").unwrap());

/// Match `--- ` followed by non-whitespace at start of a line (unified diff).
static UNIFIED_DIFF_FILE_BREAK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?m)^---\s+\S").unwrap());

/// Match `@@ ` at start of a line (hunk header boundary).
static HUNK_SPLIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^@@ ").unwrap());

/// Parse a hunk header: `@@ -del_start[,del_count] +add_start[,add_count] @@[ context]`.
static HUNK_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@(?: (.*))?").unwrap());

/// Parse `--- `/`+++ ` filename header (unified diff).
static FILENAME_HEADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(---|\+\+\+)\s+([^\t\r\n]+)").unwrap());

/// Parse `--- `/`+++ ` filename header (git diff, strips `a/`/`b/` prefix).
static FILENAME_HEADER_GIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(---|\+\+\+)\s+[ab]/([^\t\r\n]+)").unwrap());

/// Parse `diff --git a/X b/Y` with optional quoted filenames.
static ALTERNATE_FILE_NAMES_GIT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^diff --git (?:"a/(.+?)"|a/(.+?)) (?:"b/(.+?)"|b/(.+?))$"#).unwrap()
});

/// Parse `index <old_sha>..<new_sha>[ mode]` line.
static INDEX_LINE_METADATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^index ([0-9a-f]+)\.\.([0-9a-f]+)(?: (\d+))?$").unwrap());

// ---------------------------------------------------------------------------
// Line type classification
// ---------------------------------------------------------------------------

/// A single parsed diff line with its type and stripped content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLine {
    pub line: String,
    pub line_type: HunkLineType,
}

/// Classify a raw diff line by its first character.
///
/// Returns `None` for invalid lines (no recognized prefix).
pub fn parse_line_type(raw: &str) -> Option<ParsedLine> {
    let first = raw.as_bytes().first()?;
    let (line_type, rest) = match first {
        b' ' => (HunkLineType::Context, &raw[1..]),
        b'+' => (HunkLineType::Addition, &raw[1..]),
        b'-' => (HunkLineType::Deletion, &raw[1..]),
        b'\\' => (HunkLineType::Metadata, &raw[1..]),
        _ => return None,
    };
    let line = if rest.is_empty() {
        "\n".to_string()
    } else {
        rest.to_string()
    };
    Some(ParsedLine { line, line_type })
}

/// Strip a trailing newline (`\n` or `\r\n`) from a string.
fn clean_last_newline(s: &str) -> String {
    if let Some(stripped) = s.strip_suffix("\r\n") {
        stripped.to_string()
    } else if let Some(stripped) = s.strip_suffix('\n') {
        stripped.to_string()
    } else {
        s.to_string()
    }
}

// ---------------------------------------------------------------------------
// Splitting helpers
// ---------------------------------------------------------------------------

/// Split `input` at every position where `re` matches, keeping the match in
/// the start of each resulting segment (lookahead-style split).
fn split_at_matches<'a>(input: &'a str, re: &Regex) -> Vec<&'a str> {
    let mut result = Vec::new();
    let mut last = 0;
    for m in re.find_iter(input) {
        if m.start() > last {
            result.push(&input[last..m.start()]);
        }
        last = m.start();
    }
    if last < input.len() {
        result.push(&input[last..]);
    }
    if result.is_empty() {
        result.push(input);
    }
    result
}

/// Split text while preserving newlines attached to each line (like JS
/// `split(/(?<=\n)/)`) — each segment ends with `\n` except possibly the last.
fn split_with_newlines(input: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    for (i, ch) in input.char_indices() {
        if ch == '\n' {
            result.push(&input[start..=i]);
            start = i + 1;
        }
    }
    if start < input.len() {
        result.push(&input[start..]);
    }
    result
}

// ---------------------------------------------------------------------------
// Top-level entry point
// ---------------------------------------------------------------------------

/// Parse a patch/diff string that may contain multiple commits.
///
/// Handles `git format-patch` output with `From <sha>` separators as well as
/// plain `git diff` output.
pub fn parse_patch_files(data: &str) -> Vec<ParsedPatch> {
    let sections = split_at_matches(data, &COMMIT_METADATA_SPLIT);
    let mut patches = Vec::new();
    for section in sections {
        if let Some(patch) = process_patch(section) {
            patches.push(patch);
        }
    }
    patches
}

/// Parse a single patch section (one commit's worth of diffs).
pub fn process_patch(data: &str) -> Option<ParsedPatch> {
    let is_git_diff = GIT_DIFF_FILE_BREAK.is_match(data);
    let raw_files = if is_git_diff {
        split_at_matches(data, &GIT_DIFF_FILE_BREAK)
    } else {
        split_at_matches(data, &UNIFIED_DIFF_FILE_BREAK)
    };

    let mut patch_metadata: Option<String> = None;
    let mut files = Vec::new();

    for section in &raw_files {
        let section_matches_break = if is_git_diff {
            GIT_DIFF_FILE_BREAK.is_match(section)
        } else {
            UNIFIED_DIFF_FILE_BREAK.is_match(section)
        };

        if !section_matches_break {
            // This is introductory metadata (e.g. commit message) or
            // unrecognized content.
            if patch_metadata.is_none() {
                patch_metadata = Some(section.to_string());
            }
            continue;
        }

        if let Some(file) = process_file(section, is_git_diff) {
            files.push(file);
        }
    }

    Some(ParsedPatch {
        patch_metadata,
        files,
    })
}

// ---------------------------------------------------------------------------
// File-level parsing
// ---------------------------------------------------------------------------

/// Parse a single file's diff section into [`FileDiffMetadata`].
pub fn process_file(file_diff: &str, is_git_diff: bool) -> Option<FileDiffMetadata> {
    let hunk_sections = split_at_matches(file_diff, &HUNK_SPLIT);

    let mut current_file: Option<FileDiffMetadata> = None;
    let mut last_hunk_end: usize = 0;
    let mut deletion_line_index: usize;
    let mut addition_line_index: usize;

    for section in &hunk_sections {
        let lines = split_with_newlines(section);
        if lines.is_empty() {
            continue;
        }

        let first_line = lines[0].trim_end_matches('\n').trim_end_matches('\r');
        let hunk_match = HUNK_HEADER.captures(first_line);

        // -- First section: file header (no hunk match or no current_file) --
        if hunk_match.is_none() || current_file.is_none() {
            if current_file.is_some() {
                // Invalid: hunk header expected but not found
                continue;
            }

            current_file = Some(FileDiffMetadata {
                name: String::new(),
                prev_name: None,
                change_type: ChangeType::Change,
                hunks: Vec::new(),
                split_line_count: 0,
                unified_line_count: 0,
                is_partial: true,
                deletion_lines: Vec::new(),
                addition_lines: Vec::new(),
            });

            let file = current_file.as_mut().unwrap();
            parse_file_header(file, &lines, is_git_diff);
            continue;
        }

        // -- Subsequent sections: hunk parsing --
        let caps = hunk_match.unwrap();
        let file = current_file.as_mut().unwrap();

        let deletion_start: usize = caps.get(1).unwrap().as_str().parse().unwrap_or(0);
        let deletion_count: usize = caps.get(2).map_or(1, |m| m.as_str().parse().unwrap_or(1));
        let addition_start: usize = caps.get(3).unwrap().as_str().parse().unwrap_or(0);
        let addition_count: usize = caps.get(4).map_or(1, |m| m.as_str().parse().unwrap_or(1));
        let hunk_context = caps.get(5).map(|m| m.as_str().to_string());

        // Use array positions (not file-level line numbers) for partial diffs.
        // The deletion_lines/addition_lines arrays only contain lines from the
        // patch, so indices must be array-relative.
        deletion_line_index = file.deletion_lines.len();
        addition_line_index = file.addition_lines.len();

        let mut hunk_data = Hunk {
            collapsed_before: 0,
            addition_start,
            addition_count,
            addition_lines: 0,
            addition_line_index,
            deletion_start,
            deletion_count,
            deletion_lines: 0,
            deletion_line_index,
            hunk_content: Vec::new(),
            hunk_context,
            hunk_specs: Some(first_line.to_string()),
            split_line_start: 0,
            split_line_count: 0,
            unified_line_start: 0,
            unified_line_count: 0,
            no_eof_cr_deletions: false,
            no_eof_cr_additions: false,
        };

        // Strip trailing bare newlines from hunk content lines
        let mut content_lines: Vec<&str> = lines[1..].to_vec();
        while let Some(last) = content_lines.last() {
            let trimmed = last.trim_end_matches('\r');
            if trimmed.is_empty() || trimmed == "\n" {
                content_lines.pop();
            } else {
                break;
            }
        }

        let mut current_content: Option<HunkContent> = None;
        let mut last_line_type: Option<HunkLineType> = None;
        let mut addition_lines_count: usize = 0;
        let mut deletion_lines_count: usize = 0;

        for raw_line in &content_lines {
            let raw = raw_line.trim_end_matches('\n').trim_end_matches('\r');
            let parsed = match parse_line_type(raw) {
                Some(p) => p,
                None => continue,
            };

            match parsed.line_type {
                HunkLineType::Addition => {
                    let needs_new_group =
                        !matches!(current_content.as_ref(), Some(HunkContent::Change(_)));
                    if needs_new_group {
                        if let Some(prev) = current_content.take() {
                            hunk_data.hunk_content.push(prev);
                        }
                        current_content = Some(HunkContent::Change(ChangeContent {
                            deletions: 0,
                            deletion_line_index,
                            additions: 0,
                            addition_line_index,
                        }));
                    }
                    addition_line_index += 1;
                    file.addition_lines.push(parsed.line);
                    if let Some(HunkContent::Change(ref mut c)) = current_content {
                        c.additions += 1;
                    }
                    addition_lines_count += 1;
                    last_line_type = Some(HunkLineType::Addition);
                }
                HunkLineType::Deletion => {
                    let needs_new_group =
                        !matches!(current_content.as_ref(), Some(HunkContent::Change(_)));
                    if needs_new_group {
                        if let Some(prev) = current_content.take() {
                            hunk_data.hunk_content.push(prev);
                        }
                        current_content = Some(HunkContent::Change(ChangeContent {
                            deletions: 0,
                            deletion_line_index,
                            additions: 0,
                            addition_line_index,
                        }));
                    }
                    deletion_line_index += 1;
                    file.deletion_lines.push(parsed.line);
                    if let Some(HunkContent::Change(ref mut c)) = current_content {
                        c.deletions += 1;
                    }
                    deletion_lines_count += 1;
                    last_line_type = Some(HunkLineType::Deletion);
                }
                HunkLineType::Context => {
                    let needs_new_group =
                        !matches!(current_content.as_ref(), Some(HunkContent::Context(_)));
                    if needs_new_group {
                        if let Some(prev) = current_content.take() {
                            hunk_data.hunk_content.push(prev);
                        }
                        current_content = Some(HunkContent::Context(ContextContent {
                            lines: 0,
                            addition_line_index,
                            deletion_line_index,
                        }));
                    }
                    addition_line_index += 1;
                    deletion_line_index += 1;
                    file.deletion_lines.push(parsed.line.clone());
                    file.addition_lines.push(parsed.line);
                    if let Some(HunkContent::Context(ref mut c)) = current_content {
                        c.lines += 1;
                    }
                    last_line_type = Some(HunkLineType::Context);
                }
                HunkLineType::Metadata => {
                    // `\ No newline at end of file`
                    match last_line_type {
                        Some(HunkLineType::Context) => {
                            hunk_data.no_eof_cr_additions = true;
                            hunk_data.no_eof_cr_deletions = true;
                        }
                        Some(HunkLineType::Deletion) => {
                            hunk_data.no_eof_cr_deletions = true;
                        }
                        Some(HunkLineType::Addition) => {
                            hunk_data.no_eof_cr_additions = true;
                        }
                        _ => {}
                    }
                    // Strip trailing newline from the last line in the
                    // affected array(s).
                    if matches!(
                        last_line_type,
                        Some(HunkLineType::Addition | HunkLineType::Context)
                    ) {
                        if let Some(last) = file.addition_lines.last_mut() {
                            *last = clean_last_newline(last);
                        }
                    }
                    if matches!(
                        last_line_type,
                        Some(HunkLineType::Deletion | HunkLineType::Context)
                    ) {
                        if let Some(last) = file.deletion_lines.last_mut() {
                            *last = clean_last_newline(last);
                        }
                    }
                }
                _ => {}
            }
        }

        // Flush the last content group
        if let Some(last) = current_content.take() {
            hunk_data.hunk_content.push(last);
        }

        hunk_data.addition_lines = addition_lines_count;
        hunk_data.deletion_lines = deletion_lines_count;

        // Collapsed lines before this hunk
        hunk_data.collapsed_before = addition_start
            .saturating_sub(1)
            .saturating_sub(last_hunk_end);

        // Compute split and unified line counts for this hunk
        for content in &hunk_data.hunk_content {
            match content {
                HunkContent::Context(ctx) => {
                    hunk_data.split_line_count += ctx.lines;
                    hunk_data.unified_line_count += ctx.lines;
                }
                HunkContent::Change(chg) => {
                    hunk_data.split_line_count += chg.additions.max(chg.deletions);
                    hunk_data.unified_line_count += chg.additions + chg.deletions;
                }
            }
        }

        hunk_data.split_line_start = file.split_line_count + hunk_data.collapsed_before;
        hunk_data.unified_line_start = file.unified_line_count + hunk_data.collapsed_before;

        file.split_line_count += hunk_data.collapsed_before + hunk_data.split_line_count;
        file.unified_line_count += hunk_data.collapsed_before + hunk_data.unified_line_count;

        last_hunk_end = (addition_start + addition_count).saturating_sub(1);
        file.hunks.push(hunk_data);
    }

    let file = current_file.as_mut()?;

    // For non-git diffs, infer rename/new/deleted from metadata
    if !is_git_diff {
        if let Some(ref prev) = file.prev_name {
            if prev != &file.name {
                if file.hunks.is_empty() {
                    file.change_type = ChangeType::RenamePure;
                } else {
                    file.change_type = ChangeType::RenameChanged;
                }
            }
        }
    }

    // Strip prev_name unless it's a rename
    if !matches!(
        file.change_type,
        ChangeType::RenamePure | ChangeType::RenameChanged
    ) {
        file.prev_name = None;
    }

    current_file
}

// ---------------------------------------------------------------------------
// Header parsing
// ---------------------------------------------------------------------------

/// Parse the file header section (everything before the first `@@ ` hunk).
fn parse_file_header(file: &mut FileDiffMetadata, lines: &[&str], is_git_diff: bool) {
    let filename_re = if is_git_diff {
        &*FILENAME_HEADER_GIT
    } else {
        &*FILENAME_HEADER
    };

    for raw_line in lines {
        let line = raw_line.trim_end_matches('\n').trim_end_matches('\r');

        if line.starts_with("diff --git") {
            if let Some(caps) = ALTERNATE_FILE_NAMES_GIT.captures(line.trim()) {
                let prev_name = caps
                    .get(1)
                    .or_else(|| caps.get(2))
                    .map(|m| m.as_str().trim().to_string());
                let name = caps
                    .get(3)
                    .or_else(|| caps.get(4))
                    .map(|m| m.as_str().trim().to_string());
                if let Some(n) = name {
                    file.name = n.clone();
                    if let Some(ref pn) = prev_name {
                        if pn != &n {
                            file.prev_name = Some(pn.clone());
                        }
                    }
                }
            }
        } else if let Some(caps) = filename_re.captures(line) {
            let marker = caps.get(1).unwrap().as_str();
            let filename = caps.get(2).unwrap().as_str().trim();
            if marker == "---" && filename != "/dev/null" {
                file.prev_name = Some(filename.to_string());
                file.name = filename.to_string();
            } else if marker == "+++" && filename != "/dev/null" {
                file.name = filename.to_string();
            }
        } else if is_git_diff {
            parse_git_metadata_line(file, line);
        }
    }
}

/// Parse a single git-specific metadata line.
fn parse_git_metadata_line(file: &mut FileDiffMetadata, line: &str) {
    if line.starts_with("new file mode") {
        file.change_type = ChangeType::New;
    } else if line.starts_with("deleted file mode") {
        file.change_type = ChangeType::Deleted;
    } else if line.starts_with("similarity index 100%") {
        file.change_type = ChangeType::RenamePure;
    } else if line.starts_with("similarity index") {
        file.change_type = ChangeType::RenameChanged;
    } else if let Some(rest) = line.strip_prefix("rename from ") {
        file.prev_name = Some(rest.to_string());
    } else if let Some(rest) = line.strip_prefix("rename to ") {
        file.name = rest.trim().to_string();
    } else if line.starts_with("index ") {
        // Object IDs and mode from `index <old>..<new> [mode]` are not stored
        // in FileDiffMetadata; the regex validates format only.
        let _ = INDEX_LINE_METADATA.captures(line.trim());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_line_type tests --

    #[test]
    fn parse_line_type_context() {
        let result = parse_line_type(" hello world").unwrap();
        assert_eq!(result.line_type, HunkLineType::Context);
        assert_eq!(result.line, "hello world");
    }

    #[test]
    fn parse_line_type_addition() {
        let result = parse_line_type("+new line").unwrap();
        assert_eq!(result.line_type, HunkLineType::Addition);
        assert_eq!(result.line, "new line");
    }

    #[test]
    fn parse_line_type_deletion() {
        let result = parse_line_type("-old line").unwrap();
        assert_eq!(result.line_type, HunkLineType::Deletion);
        assert_eq!(result.line, "old line");
    }

    #[test]
    fn parse_line_type_metadata() {
        let result = parse_line_type("\\ No newline at end of file").unwrap();
        assert_eq!(result.line_type, HunkLineType::Metadata);
        assert_eq!(result.line, " No newline at end of file");
    }

    #[test]
    fn parse_line_type_empty_content() {
        let result = parse_line_type("+").unwrap();
        assert_eq!(result.line_type, HunkLineType::Addition);
        assert_eq!(result.line, "\n");
    }

    #[test]
    fn parse_line_type_invalid() {
        assert!(parse_line_type("no prefix").is_none());
        assert!(parse_line_type("").is_none());
    }

    // -- clean_last_newline tests --

    #[test]
    fn clean_newline_lf() {
        assert_eq!(clean_last_newline("hello\n"), "hello");
    }

    #[test]
    fn clean_newline_crlf() {
        assert_eq!(clean_last_newline("hello\r\n"), "hello");
    }

    #[test]
    fn clean_newline_none() {
        assert_eq!(clean_last_newline("hello"), "hello");
    }

    // -- Git diff parsing --

    const SIMPLE_GIT_DIFF: &str = "\
diff --git a/src/main.rs b/src/main.rs
index abc1234..def5678 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,5 +1,6 @@
 fn main() {
-    println!(\"hello\");
+    println!(\"hello world\");
+    println!(\"goodbye\");
     let x = 1;
     let y = 2;
 }
";

    #[test]
    fn parse_simple_git_diff() {
        let patches = parse_patch_files(SIMPLE_GIT_DIFF);
        assert_eq!(patches.len(), 1);
        let patch = &patches[0];
        assert_eq!(patch.files.len(), 1);

        let file = &patch.files[0];
        assert_eq!(file.name, "src/main.rs");
        assert_eq!(file.change_type, ChangeType::Change);
        assert!(file.prev_name.is_none());
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!(hunk.deletion_start, 1);
        assert_eq!(hunk.deletion_count, 5);
        assert_eq!(hunk.addition_start, 1);
        assert_eq!(hunk.addition_count, 6);
        assert_eq!(hunk.addition_lines, 2); // two `+` lines
        assert_eq!(hunk.deletion_lines, 1); // one `-` line
    }

    #[test]
    fn hunk_content_groups() {
        let patches = parse_patch_files(SIMPLE_GIT_DIFF);
        let hunk = &patches[0].files[0].hunks[0];

        // Should be: context(1 line "fn main()"), change(1 del + 2 add),
        // context(3 lines "let x", "let y", "}")
        assert_eq!(hunk.hunk_content.len(), 3);

        match &hunk.hunk_content[0] {
            HunkContent::Context(ctx) => assert_eq!(ctx.lines, 1),
            _ => panic!("expected context"),
        }
        match &hunk.hunk_content[1] {
            HunkContent::Change(chg) => {
                assert_eq!(chg.deletions, 1);
                assert_eq!(chg.additions, 2);
            }
            _ => panic!("expected change"),
        }
        match &hunk.hunk_content[2] {
            HunkContent::Context(ctx) => assert_eq!(ctx.lines, 3),
            _ => panic!("expected context"),
        }
    }

    #[test]
    fn split_unified_line_counts() {
        let patches = parse_patch_files(SIMPLE_GIT_DIFF);
        let hunk = &patches[0].files[0].hunks[0];

        // split: ctx(1) + max(1del, 2add)=2 + ctx(3) = 6
        assert_eq!(hunk.split_line_count, 6);
        // unified: ctx(1) + (1+2)=3 + ctx(3) = 7
        assert_eq!(hunk.unified_line_count, 7);
    }

    #[test]
    fn addition_and_deletion_lines_partial() {
        let patches = parse_patch_files(SIMPLE_GIT_DIFF);
        let file = &patches[0].files[0];
        assert!(file.is_partial);

        // Deletion lines: context lines + deletion lines
        // "fn main() {\n", "    println!(\"hello\");\n", "    let x = 1;\n",
        // "    let y = 2;\n", "}\n"
        assert_eq!(file.deletion_lines.len(), 5);

        // Addition lines: context lines + addition lines
        // "fn main() {\n", "    println!(\"hello world\");\n",
        // "    println!(\"goodbye\");\n", "    let x = 1;\n",
        // "    let y = 2;\n", "}\n"
        assert_eq!(file.addition_lines.len(), 6);
    }

    // -- New file --

    const NEW_FILE_DIFF: &str = "\
diff --git a/src/new.rs b/src/new.rs
new file mode 100644
index 0000000..abc1234
--- /dev/null
+++ b/src/new.rs
@@ -0,0 +1,3 @@
+fn new_func() {
+    println!(\"new\");
+}
";

    #[test]
    fn parse_new_file() {
        let patches = parse_patch_files(NEW_FILE_DIFF);
        let file = &patches[0].files[0];
        assert_eq!(file.name, "src/new.rs");
        assert_eq!(file.change_type, ChangeType::New);
        assert!(file.prev_name.is_none());
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].addition_lines, 3);
        assert_eq!(file.hunks[0].deletion_lines, 0);
    }

    // -- Deleted file --

    const DELETED_FILE_DIFF: &str = "\
diff --git a/src/old.rs b/src/old.rs
deleted file mode 100644
index abc1234..0000000
--- a/src/old.rs
+++ /dev/null
@@ -1,3 +0,0 @@
-fn old_func() {
-    println!(\"old\");
-}
";

    #[test]
    fn parse_deleted_file() {
        let patches = parse_patch_files(DELETED_FILE_DIFF);
        let file = &patches[0].files[0];
        assert_eq!(file.name, "src/old.rs");
        assert_eq!(file.change_type, ChangeType::Deleted);
        assert_eq!(file.hunks.len(), 1);
        assert_eq!(file.hunks[0].deletion_lines, 3);
        assert_eq!(file.hunks[0].addition_lines, 0);
    }

    // -- Rename --

    const RENAME_DIFF: &str = "\
diff --git a/src/old_name.rs b/src/new_name.rs
similarity index 100%
rename from src/old_name.rs
rename to src/new_name.rs
";

    #[test]
    fn parse_pure_rename() {
        let patches = parse_patch_files(RENAME_DIFF);
        let file = &patches[0].files[0];
        assert_eq!(file.name, "src/new_name.rs");
        assert_eq!(file.prev_name.as_deref(), Some("src/old_name.rs"));
        assert_eq!(file.change_type, ChangeType::RenamePure);
        assert!(file.hunks.is_empty());
    }

    const RENAME_CHANGED_DIFF: &str = "\
diff --git a/src/old.rs b/src/renamed.rs
similarity index 85%
rename from src/old.rs
rename to src/renamed.rs
index abc1234..def5678 100644
--- a/src/old.rs
+++ b/src/renamed.rs
@@ -1,3 +1,4 @@
 fn func() {
     println!(\"hello\");
+    println!(\"extra\");
 }
";

    #[test]
    fn parse_rename_with_changes() {
        let patches = parse_patch_files(RENAME_CHANGED_DIFF);
        let file = &patches[0].files[0];
        assert_eq!(file.name, "src/renamed.rs");
        assert_eq!(file.prev_name.as_deref(), Some("src/old.rs"));
        assert_eq!(file.change_type, ChangeType::RenameChanged);
        assert_eq!(file.hunks.len(), 1);
    }

    // -- Multiple hunks --

    const MULTI_HUNK_DIFF: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index abc1234..def5678 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,4 @@
 fn first() {
+    // added
     println!(\"first\");
 }
@@ -10,3 +11,4 @@
 fn second() {
+    // also added
     println!(\"second\");
 }
";

    #[test]
    fn parse_multiple_hunks() {
        let patches = parse_patch_files(MULTI_HUNK_DIFF);
        let file = &patches[0].files[0];
        assert_eq!(file.hunks.len(), 2);

        let h0 = &file.hunks[0];
        assert_eq!(h0.addition_start, 1);
        assert_eq!(h0.addition_count, 4);
        assert_eq!(h0.collapsed_before, 0);

        let h1 = &file.hunks[1];
        assert_eq!(h1.addition_start, 11);
        assert_eq!(h1.addition_count, 4);
        // collapsed_before = addition_start - 1 - last_hunk_end
        // = 11 - 1 - (1 + 4 - 1) = 10 - 4 = 6
        assert_eq!(h1.collapsed_before, 6);
    }

    #[test]
    fn cumulative_split_unified_starts() {
        let patches = parse_patch_files(MULTI_HUNK_DIFF);
        let file = &patches[0].files[0];

        let h0 = &file.hunks[0];
        assert_eq!(h0.split_line_start, 0);
        assert_eq!(h0.unified_line_start, 0);

        let h1 = &file.hunks[1];
        // h1.split_line_start = file.split_line_count(after h0) + collapsed_before
        // after h0: 0 (collapsed) + h0.split_line_count
        // h0: ctx(1) + change(max(0,1)=1) + ctx(2) = 4
        // h1 collapsed_before = 6
        // h1.split_line_start = 4 + 6 = 10
        assert_eq!(
            h1.split_line_start,
            h0.split_line_count + h1.collapsed_before
        );
    }

    // -- Multi-file diff --

    const MULTI_FILE_DIFF: &str = "\
diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,3 +1,3 @@
 fn a() {
-    old();
+    new();
 }
diff --git a/src/b.rs b/src/b.rs
index 333..444 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -1,3 +1,3 @@
 fn b() {
-    old_b();
+    new_b();
 }
";

    #[test]
    fn parse_multi_file_diff() {
        let patches = parse_patch_files(MULTI_FILE_DIFF);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].files.len(), 2);
        assert_eq!(patches[0].files[0].name, "src/a.rs");
        assert_eq!(patches[0].files[1].name, "src/b.rs");
    }

    // -- No newline at end of file --

    const NO_EOF_NEWLINE_DIFF: &str = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,2 +1,2 @@
 fn main() {
-    old()
\\ No newline at end of file
+    new()
\\ No newline at end of file
";

    #[test]
    fn parse_no_eof_newline() {
        let patches = parse_patch_files(NO_EOF_NEWLINE_DIFF);
        let hunk = &patches[0].files[0].hunks[0];
        assert!(hunk.no_eof_cr_deletions);
        assert!(hunk.no_eof_cr_additions);
    }

    // -- Unified (non-git) diff --

    const UNIFIED_DIFF: &str = "\
--- src/main.rs\t2026-01-01 00:00:00.000000000 +0000
+++ src/main.rs\t2026-01-02 00:00:00.000000000 +0000
@@ -1,3 +1,3 @@
 fn main() {
-    old();
+    new();
 }
";

    #[test]
    fn parse_unified_non_git_diff() {
        let patches = parse_patch_files(UNIFIED_DIFF);
        assert_eq!(patches.len(), 1);
        let file = &patches[0].files[0];
        assert_eq!(file.name, "src/main.rs");
        assert_eq!(file.change_type, ChangeType::Change);
        assert_eq!(file.hunks.len(), 1);
    }

    // -- Real git diff output (multi-file, multi-hunk) --

    #[test]
    fn parse_real_world_git_diff() {
        // A real-world diff from the CAS repo
        let diff = "\
diff --git a/cas-cli/src/ui/factory/renderer/factory/rendering.rs b/cas-cli/src/ui/factory/renderer/factory/rendering.rs
index 148d01ba..6aa037f6 100644
--- a/cas-cli/src/ui/factory/renderer/factory/rendering.rs
+++ b/cas-cli/src/ui/factory/renderer/factory/rendering.rs
@@ -1052,8 +1052,8 @@ impl FactoryRenderer {
                 {
                     use crate::ui::factory::renderer::FactoryViewMode;
                     let hint = match self.state.factory_view_mode {
-                        FactoryViewMode::Panes => \"V\",
-                        FactoryViewMode::MissionControl => \"V\",
+                        FactoryViewMode::Panes => \"^W\",
+                        FactoryViewMode::MissionControl => \"^W\",
                     };
                     right_spans.push(Span::styled(hint, Style::default().fg(palette.status_info)));
                     right_spans.push(Span::raw(\" \"));
";
        let patches = parse_patch_files(diff);
        assert_eq!(patches.len(), 1);
        let file = &patches[0].files[0];
        assert_eq!(
            file.name,
            "cas-cli/src/ui/factory/renderer/factory/rendering.rs"
        );
        assert_eq!(file.hunks.len(), 1);

        let hunk = &file.hunks[0];
        assert_eq!(hunk.deletion_start, 1052);
        assert_eq!(hunk.deletion_count, 8);
        assert_eq!(hunk.addition_start, 1052);
        assert_eq!(hunk.addition_count, 8);
        assert_eq!(hunk.deletion_lines, 2);
        assert_eq!(hunk.addition_lines, 2);

        // Content groups: ctx(3), change(2del+2add), ctx(3)
        assert_eq!(hunk.hunk_content.len(), 3);
        match &hunk.hunk_content[0] {
            HunkContent::Context(ctx) => assert_eq!(ctx.lines, 3),
            _ => panic!("expected context"),
        }
        match &hunk.hunk_content[1] {
            HunkContent::Change(chg) => {
                assert_eq!(chg.deletions, 2);
                assert_eq!(chg.additions, 2);
            }
            _ => panic!("expected change"),
        }
        match &hunk.hunk_content[2] {
            HunkContent::Context(ctx) => assert_eq!(ctx.lines, 3),
            _ => panic!("expected context"),
        }
    }

    // -- Line index tracking --

    #[test]
    fn line_indices_in_content_groups() {
        let patches = parse_patch_files(SIMPLE_GIT_DIFF);
        let hunk = &patches[0].files[0].hunks[0];

        // Hunk starts at line 1 (1-based), so indices are 0-based: 0
        assert_eq!(hunk.addition_line_index, 0);
        assert_eq!(hunk.deletion_line_index, 0);

        // First context group starts at index 0
        match &hunk.hunk_content[0] {
            HunkContent::Context(ctx) => {
                assert_eq!(ctx.addition_line_index, 0);
                assert_eq!(ctx.deletion_line_index, 0);
            }
            _ => panic!("expected context"),
        }

        // Change group starts after 1 context line
        match &hunk.hunk_content[1] {
            HunkContent::Change(chg) => {
                assert_eq!(chg.addition_line_index, 1);
                assert_eq!(chg.deletion_line_index, 1);
            }
            _ => panic!("expected change"),
        }

        // Last context group: after 1 context + 1 deletion + 2 additions
        match &hunk.hunk_content[2] {
            HunkContent::Context(ctx) => {
                assert_eq!(ctx.deletion_line_index, 2); // 1 ctx + 1 del
                assert_eq!(ctx.addition_line_index, 3); // 1 ctx + 2 add
            }
            _ => panic!("expected context"),
        }
    }

    // -- Hunk context string --

    const DIFF_WITH_CONTEXT: &str = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -10,3 +10,4 @@ fn existing_function()
 fn second() {
+    // comment
     println!(\"second\");
 }
";

    #[test]
    fn hunk_context_string() {
        let patches = parse_patch_files(DIFF_WITH_CONTEXT);
        let hunk = &patches[0].files[0].hunks[0];
        assert_eq!(hunk.hunk_context.as_deref(), Some("fn existing_function()"));
    }

    // -- Multi-commit patch (format-patch) --

    const MULTI_COMMIT_PATCH: &str = "\
From abc123 Mon Sep 17 00:00:00 2001
Subject: First commit

diff --git a/a.rs b/a.rs
index 111..222 100644
--- a/a.rs
+++ b/a.rs
@@ -1 +1 @@
-old
+new
From def456 Mon Sep 17 00:00:00 2001
Subject: Second commit

diff --git a/b.rs b/b.rs
index 333..444 100644
--- a/b.rs
+++ b/b.rs
@@ -1 +1 @@
-old_b
+new_b
";

    #[test]
    fn parse_multi_commit_patch() {
        let patches = parse_patch_files(MULTI_COMMIT_PATCH);
        assert_eq!(patches.len(), 2);

        assert!(patches[0].patch_metadata.is_some());
        assert_eq!(patches[0].files.len(), 1);
        assert_eq!(patches[0].files[0].name, "a.rs");

        assert!(patches[1].patch_metadata.is_some());
        assert_eq!(patches[1].files.len(), 1);
        assert_eq!(patches[1].files[0].name, "b.rs");
    }

    // -- Edge case: empty input --

    #[test]
    fn parse_empty_input() {
        let patches = parse_patch_files("");
        assert!(patches.is_empty() || patches.iter().all(|p| p.files.is_empty()));
    }

    // -- Hunk header with count omitted (defaults to 1) --

    const SINGLE_LINE_HUNK: &str = "\
diff --git a/f.rs b/f.rs
index abc..def 100644
--- a/f.rs
+++ b/f.rs
@@ -5 +5,2 @@
-single
+double
+line
";

    #[test]
    fn hunk_count_defaults_to_one() {
        let patches = parse_patch_files(SINGLE_LINE_HUNK);
        let hunk = &patches[0].files[0].hunks[0];
        assert_eq!(hunk.deletion_count, 1); // no comma → defaults to 1
        assert_eq!(hunk.addition_count, 2);
    }
}
