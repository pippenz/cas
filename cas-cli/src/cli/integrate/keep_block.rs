//! Round-trip and merge support for `<!-- keep -->` … `<!-- /keep -->` blocks
//! in generated SKILL.md / docs files.
//!
//! Spec source: `cas-cli/src/builtins/skills/codemap/SKILL.md` and
//! `cas-cli/src/builtins/skills/project-overview/SKILL.md`
//! ("Preserving hand-edited sections" sections). Used by Vercel/Neon/GitHub
//! integration handlers (cas-8e37/1ece/f425) to update their generated
//! SKILL files without clobbering user hand-edits or MCP-recorded IDs.
//!
//! ## Marker grammar
//!
//! - Open: `<!-- keep -->` (unnamed) or `<!-- keep <ident> -->` (named).
//! - Close: `<!-- /keep -->` (unnamed) or `<!-- /keep <ident> -->` (named).
//! - Markers must be on their own line (leading/trailing whitespace ignored).
//! - Nested keep blocks are not supported.
//! - When a block is opened with a name, its close marker must use the same name.
//!
//! ## Merge semantics
//!
//! See [`MergeMode`] for the two flows (regenerate vs refresh).

use std::collections::{HashMap, VecDeque};

use thiserror::Error;

/// A single keep block extracted from a source document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeepBlock {
    /// Optional name parsed from the marker (e.g. `<!-- keep vercel-ids -->`).
    pub name: Option<String>,
    /// Body bytes between the open and close markers, exclusive of both
    /// marker lines, joined by `\n` (trailing newline omitted).
    pub body: String,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum KeepBlockError {
    #[error("unmatched opening keep marker at line {line}")]
    UnmatchedOpen { line: usize },

    #[error("unmatched closing keep marker at line {line}")]
    UnmatchedClose { line: usize },

    #[error(
        "nested keep markers are not supported (outer open at line {outer}, inner open at line {inner})"
    )]
    Nested { outer: usize, inner: usize },

    #[error(
        "keep marker name mismatch at line {line}: opened with {open:?} but closed with {close:?}"
    )]
    NameMismatch {
        open: Option<String>,
        close: Option<String>,
        line: usize,
    },
}

/// Strategy for [`merge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMode {
    /// **Regenerate flow.** Outer content (everything outside keep blocks) is
    /// taken from `template`; keep-block bodies are spliced in from
    /// `existing`. Used when the caller is re-rendering boilerplate prose
    /// and must not destroy hand-edited content the user owns.
    PreserveExisting,

    /// **Refresh flow.** Outer content and keep-block bodies are both taken
    /// from `template`. Used when the caller has freshly canonical content
    /// (e.g. MCP-fetched IDs already templated in) it wants to overwrite the
    /// previous keep-block values with.
    PreferTemplate,
}

const OPEN_PREFIX: &str = "<!-- keep";
const CLOSE_PREFIX: &str = "<!-- /keep";
const MARKER_SUFFIX: &str = "-->";

fn is_close_marker_line(trim: &str) -> bool {
    trim.starts_with(CLOSE_PREFIX) && trim.ends_with(MARKER_SUFFIX)
}

fn is_open_marker_line(trim: &str) -> bool {
    trim.starts_with(OPEN_PREFIX)
        && !trim.starts_with(CLOSE_PREFIX)
        && trim.ends_with(MARKER_SUFFIX)
}

/// Returns `Some(name)` if `trim` is a valid open marker. The inner option
/// distinguishes named (`Some(Some(name))`) from unnamed (`Some(None)`).
fn parse_open(trim: &str) -> Option<Option<String>> {
    if !is_open_marker_line(trim) {
        return None;
    }
    let inner = trim[OPEN_PREFIX.len()..trim.len() - MARKER_SUFFIX.len()].trim();
    Some(if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    })
}

/// Returns `Some(name)` if `trim` is a valid close marker.
fn parse_close(trim: &str) -> Option<Option<String>> {
    if !is_close_marker_line(trim) {
        return None;
    }
    let inner = trim[CLOSE_PREFIX.len()..trim.len() - MARKER_SUFFIX.len()].trim();
    Some(if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    })
}

/// Extract every keep block from `source`, preserving order.
///
/// Returns an empty vec if no markers are present.
pub fn extract(source: &str) -> Result<Vec<KeepBlock>, KeepBlockError> {
    let lines: Vec<&str> = source.split('\n').collect();
    let mut out: Vec<KeepBlock> = Vec::new();
    // (1-based open line, name, accumulated body lines)
    let mut state: Option<(usize, Option<String>, Vec<&str>)> = None;

    for (idx, line) in lines.iter().enumerate() {
        let line_no = idx + 1;
        let trim = line.trim();

        if let Some(name) = parse_open(trim) {
            if let Some((outer, _, _)) = state {
                return Err(KeepBlockError::Nested {
                    outer,
                    inner: line_no,
                });
            }
            state = Some((line_no, name, Vec::new()));
            continue;
        }

        if let Some(close_name) = parse_close(trim) {
            match state.take() {
                None => return Err(KeepBlockError::UnmatchedClose { line: line_no }),
                Some((_open_line, open_name, body_lines)) => {
                    if open_name != close_name {
                        return Err(KeepBlockError::NameMismatch {
                            open: open_name,
                            close: close_name,
                            line: line_no,
                        });
                    }
                    out.push(KeepBlock {
                        name: open_name,
                        body: body_lines.join("\n"),
                    });
                }
            }
            continue;
        }

        if let Some((_, _, body)) = state.as_mut() {
            body.push(line);
        }
    }

    if let Some((open_line, _, _)) = state {
        return Err(KeepBlockError::UnmatchedOpen { line: open_line });
    }
    Ok(out)
}

/// Merge `template` with an optional `existing` document under [`MergeMode`].
///
/// - In [`MergeMode::PreferTemplate`], `existing` is ignored; `template` is
///   validated and returned verbatim.
/// - In [`MergeMode::PreserveExisting`]:
///     - If `existing` is `None`, `template` is validated and returned.
///     - Otherwise, the output structure follows `template`. For each keep
///       block in `template`, a body is selected from `existing`:
///         - Named blocks match by name.
///         - Unnamed blocks match by ordinal position among the unnamed
///           blocks in `existing`.
///         - If no match exists in `existing`, the template's own keep-block
///           body is kept.
pub fn merge(
    template: &str,
    existing: Option<&str>,
    mode: MergeMode,
) -> Result<String, KeepBlockError> {
    // Always validate template, regardless of mode.
    let _ = extract(template)?;

    if mode == MergeMode::PreferTemplate {
        return Ok(template.to_string());
    }

    let Some(existing) = existing else {
        return Ok(template.to_string());
    };

    let existing_blocks = extract(existing)?;
    let mut named: HashMap<String, String> = HashMap::new();
    let mut unnamed: VecDeque<String> = VecDeque::new();
    for b in existing_blocks {
        match b.name {
            Some(n) => {
                named.insert(n, b.body);
            }
            None => unnamed.push_back(b.body),
        }
    }

    let lines: Vec<&str> = template.split('\n').collect();
    let mut out_lines: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trim = line.trim();
        if let Some(name) = parse_open(trim) {
            // Find matching close marker (extract() above already validated).
            let mut j = i + 1;
            while j < lines.len() && parse_close(lines[j].trim()).is_none() {
                j += 1;
            }

            // Emit open marker line verbatim.
            out_lines.push(line.to_string());

            // Choose body from existing if available, else fall back to
            // the template's own body for this block.
            let chosen = match &name {
                Some(n) => named.remove(n),
                None => unnamed.pop_front(),
            };
            let body = chosen.unwrap_or_else(|| {
                if j > i + 1 {
                    lines[i + 1..j].join("\n")
                } else {
                    String::new()
                }
            });

            if !body.is_empty() {
                for bl in body.split('\n') {
                    out_lines.push(bl.to_string());
                }
            }

            // Emit close marker line verbatim (j is the close line, guaranteed
            // to exist because extract() validated the template).
            out_lines.push(lines[j].to_string());
            i = j + 1;
        } else {
            out_lines.push(line.to_string());
            i += 1;
        }
    }
    Ok(out_lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract --------------------------------------------------------

    #[test]
    fn extract_empty_source_yields_no_blocks() {
        assert_eq!(extract("").unwrap(), Vec::new());
    }

    #[test]
    fn extract_no_markers_yields_no_blocks() {
        let src = "# Heading\n\nSome prose.\n- bullet\n";
        assert_eq!(extract(src).unwrap(), Vec::new());
    }

    #[test]
    fn extract_single_unnamed_block() {
        let src = "before\n<!-- keep -->\nfoo\nbar\n<!-- /keep -->\nafter\n";
        let blocks = extract(src).unwrap();
        assert_eq!(
            blocks,
            vec![KeepBlock {
                name: None,
                body: "foo\nbar".to_string()
            }]
        );
    }

    #[test]
    fn extract_named_block() {
        let src = "<!-- keep vercel-ids -->\nprojectId=abc\n<!-- /keep vercel-ids -->\n";
        let blocks = extract(src).unwrap();
        assert_eq!(
            blocks,
            vec![KeepBlock {
                name: Some("vercel-ids".to_string()),
                body: "projectId=abc".to_string()
            }]
        );
    }

    #[test]
    fn extract_multiple_blocks_independent() {
        let src = concat!(
            "## A\n",
            "<!-- keep -->\nalpha\n<!-- /keep -->\n",
            "middle\n",
            "<!-- keep two -->\nbeta\n<!-- /keep two -->\n",
            "## B\n",
            "<!-- keep -->\ngamma\n<!-- /keep -->\n",
        );
        let blocks = extract(src).unwrap();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].name, None);
        assert_eq!(blocks[0].body, "alpha");
        assert_eq!(blocks[1].name, Some("two".to_string()));
        assert_eq!(blocks[1].body, "beta");
        assert_eq!(blocks[2].name, None);
        assert_eq!(blocks[2].body, "gamma");
    }

    #[test]
    fn extract_empty_block_body() {
        let src = "<!-- keep -->\n<!-- /keep -->\n";
        let blocks = extract(src).unwrap();
        assert_eq!(
            blocks,
            vec![KeepBlock {
                name: None,
                body: String::new()
            }]
        );
    }

    #[test]
    fn extract_unmatched_open_errors() {
        let src = "ok\n<!-- keep -->\nfoo\n";
        let err = extract(src).unwrap_err();
        assert_eq!(err, KeepBlockError::UnmatchedOpen { line: 2 });
    }

    #[test]
    fn extract_unmatched_close_errors() {
        let src = "ok\n<!-- /keep -->\n";
        let err = extract(src).unwrap_err();
        assert_eq!(err, KeepBlockError::UnmatchedClose { line: 2 });
    }

    #[test]
    fn extract_nested_errors() {
        let src = "<!-- keep -->\n<!-- keep inner -->\nx\n<!-- /keep inner -->\n<!-- /keep -->\n";
        let err = extract(src).unwrap_err();
        assert_eq!(err, KeepBlockError::Nested { outer: 1, inner: 2 });
    }

    #[test]
    fn extract_name_mismatch_errors() {
        let src = "<!-- keep a -->\nx\n<!-- /keep b -->\n";
        let err = extract(src).unwrap_err();
        assert_eq!(
            err,
            KeepBlockError::NameMismatch {
                open: Some("a".to_string()),
                close: Some("b".to_string()),
                line: 3
            }
        );
    }

    #[test]
    fn extract_unnamed_open_named_close_errors() {
        let src = "<!-- keep -->\nx\n<!-- /keep mine -->\n";
        let err = extract(src).unwrap_err();
        assert_eq!(
            err,
            KeepBlockError::NameMismatch {
                open: None,
                close: Some("mine".to_string()),
                line: 3
            }
        );
    }

    // --- merge: round-trip & no-existing --------------------------------

    #[test]
    fn merge_round_trip_no_existing_preserves_template_byte_for_byte() {
        let template = "intro\n<!-- keep -->\nfoo\nbar\n<!-- /keep -->\noutro\n";
        let merged = merge(template, None, MergeMode::PreserveExisting).unwrap();
        assert_eq!(merged, template);
    }

    #[test]
    fn merge_prefer_template_returns_template_unchanged() {
        let template = "intro\n<!-- keep -->\nfresh-ids\n<!-- /keep -->\noutro\n";
        let existing = "intro-old\n<!-- keep -->\nold-ids\n<!-- /keep -->\noutro-old\n";
        let merged = merge(template, Some(existing), MergeMode::PreferTemplate).unwrap();
        assert_eq!(merged, template);
    }

    // --- merge: regen-merge (preserve existing keep) --------------------

    #[test]
    fn merge_preserve_existing_swaps_keep_body() {
        let template = "new outer\n<!-- keep -->\nplaceholder\n<!-- /keep -->\nmore new\n";
        let existing = "old outer\n<!-- keep -->\nuser-edited content\nline 2\n<!-- /keep -->\nold more\n";
        let merged = merge(template, Some(existing), MergeMode::PreserveExisting).unwrap();
        assert_eq!(
            merged,
            "new outer\n<!-- keep -->\nuser-edited content\nline 2\n<!-- /keep -->\nmore new\n"
        );
    }

    #[test]
    fn merge_preserve_existing_named_blocks_match_by_name_not_position() {
        let template = concat!(
            "<!-- keep b -->\nplaceholder-b\n<!-- /keep b -->\n",
            "<!-- keep a -->\nplaceholder-a\n<!-- /keep a -->\n",
        );
        let existing = concat!(
            "<!-- keep a -->\nA-content\n<!-- /keep a -->\n",
            "<!-- keep b -->\nB-content\n<!-- /keep b -->\n",
        );
        let merged = merge(template, Some(existing), MergeMode::PreserveExisting).unwrap();
        assert_eq!(
            merged,
            concat!(
                "<!-- keep b -->\nB-content\n<!-- /keep b -->\n",
                "<!-- keep a -->\nA-content\n<!-- /keep a -->\n",
            )
        );
    }

    #[test]
    fn merge_preserve_existing_unnamed_blocks_match_by_ordinal() {
        let template = concat!(
            "<!-- keep -->\nph1\n<!-- /keep -->\n",
            "<!-- keep -->\nph2\n<!-- /keep -->\n",
        );
        let existing = concat!(
            "<!-- keep -->\nfirst\n<!-- /keep -->\n",
            "<!-- keep -->\nsecond\n<!-- /keep -->\n",
        );
        let merged = merge(template, Some(existing), MergeMode::PreserveExisting).unwrap();
        assert_eq!(
            merged,
            concat!(
                "<!-- keep -->\nfirst\n<!-- /keep -->\n",
                "<!-- keep -->\nsecond\n<!-- /keep -->\n",
            )
        );
    }

    #[test]
    fn merge_preserve_existing_falls_back_to_template_when_no_match() {
        // Template has 2 unnamed; existing has 1 unnamed. Second unnamed
        // template block keeps its own body.
        let template = concat!(
            "<!-- keep -->\ntpl1\n<!-- /keep -->\n",
            "<!-- keep -->\ntpl2\n<!-- /keep -->\n",
        );
        let existing = "<!-- keep -->\nexist1\n<!-- /keep -->\n";
        let merged = merge(template, Some(existing), MergeMode::PreserveExisting).unwrap();
        assert_eq!(
            merged,
            concat!(
                "<!-- keep -->\nexist1\n<!-- /keep -->\n",
                "<!-- keep -->\ntpl2\n<!-- /keep -->\n",
            )
        );
    }

    // --- merge: missing/malformed --------------------------------------

    #[test]
    fn merge_no_markers_in_template_returns_template_unchanged() {
        let template = "# All regenerable\nno markers here\n";
        let existing = "# Old\n<!-- keep -->\nold-keep\n<!-- /keep -->\n";
        let merged = merge(template, Some(existing), MergeMode::PreserveExisting).unwrap();
        assert_eq!(merged, template);
    }

    #[test]
    fn merge_malformed_template_errors_does_not_destroy_content() {
        let template = "<!-- keep -->\nno close\n";
        let err = merge(template, None, MergeMode::PreserveExisting).unwrap_err();
        assert!(matches!(err, KeepBlockError::UnmatchedOpen { .. }));
    }

    #[test]
    fn merge_malformed_existing_errors() {
        let template = "<!-- keep -->\nx\n<!-- /keep -->\n";
        let existing = "<!-- /keep -->\n";
        let err =
            merge(template, Some(existing), MergeMode::PreserveExisting).unwrap_err();
        assert!(matches!(err, KeepBlockError::UnmatchedClose { .. }));
    }

    // --- whitespace tolerance -------------------------------------------

    #[test]
    fn extract_tolerates_indented_markers() {
        let src = "  <!-- keep -->\n  body\n  <!-- /keep -->\n";
        let blocks = extract(src).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].body, "  body");
    }
}
