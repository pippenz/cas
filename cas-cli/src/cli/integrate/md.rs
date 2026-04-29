//! Shared markdown helpers for the `cas integrate <platform>` handlers.
//!
//! Two concerns live here:
//!
//! 1. **Cell escaping** ([`escape_md_cell`], [`escape_md_cell_code`]) — any
//!    platform-supplied string that we splice into a markdown table cell
//!    must be sanitized so that an exotic project name (`foo|bar`,
//!    `<!-- gotcha -->`, `` `evil` ``) cannot break out of the table or
//!    corrupt the surrounding `<!-- keep <name> -->` markers.
//!
//! 2. **Identity tag** ([`emit_cas_full_name_tag`], [`parse_cas_full_name_tag`])
//!    — a single, machine-readable tag for the canonical identity of a
//!    handler's keep block. This replaces ad-hoc parsing of human-friendly
//!    rows (the github handler used to grep for `**Full name**`); now every
//!    handler emits and parses the same `<!-- cas:full_name=... -->`
//!    convention. Future template revisions that rename rows or restyle
//!    tables won't break verify.
//!
//! Owner: cas-fc38 (cross-cutting hardening).

// ---------------------------------------------------------------------------
// Cell escaping
// ---------------------------------------------------------------------------

/// Escape a value for safe inclusion in a markdown table cell. Strips:
///
/// - `|` (would break the cell layout — escaped to `\|`).
/// - HTML comment open/close (`<!--`, `-->`) — would corrupt the surrounding
///   `<!-- keep <name> -->` markers and on subsequent refresh cause
///   `keep_block::extract` to mis-parse.
/// - Newlines and CR (would break the table row entirely).
pub fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|")
        .replace("<!--", "&lt;!--")
        .replace("-->", "--&gt;")
        .replace('\n', " ")
        .replace('\r', " ")
}

/// Escape a value rendered inside backticks (`code` cell). Same as
/// [`escape_md_cell`] but additionally strips backticks so a malicious id
/// cannot break out of the inline-code span and hijack the table.
pub fn escape_md_cell_code(s: &str) -> String {
    escape_md_cell(s).replace('`', "")
}

// ---------------------------------------------------------------------------
// `cas:full_name` identity tag
// ---------------------------------------------------------------------------

const TAG_PREFIX: &str = "<!-- cas:full_name=";
const TAG_SUFFIX: &str = " -->";

/// Emit a single line of the form `<!-- cas:full_name=<value> -->`. Embedded
/// in a keep block so refresh / verify can recover the canonical identity
/// without parsing human-friendly markdown around it.
///
/// The value is sanitized: `-->` is rewritten to `--&gt;` so the tag itself
/// stays well-formed, and CR/LF are stripped so the tag fits on one line.
pub fn emit_cas_full_name_tag(value: &str) -> String {
    let safe = value
        .replace("-->", "--&gt;")
        .replace('\n', " ")
        .replace('\r', " ");
    format!("{TAG_PREFIX}{safe}{TAG_SUFFIX}")
}

/// Extract the value from the first `<!-- cas:full_name=<value> -->` tag in
/// `body`. Returns `None` if no tag is present or the tag is malformed.
///
/// Whitespace immediately around the value is trimmed for tolerance against
/// hand-edits like `<!-- cas:full_name= acme/widget -->`.
pub fn parse_cas_full_name_tag(body: &str) -> Option<String> {
    for line in body.lines() {
        let trim = line.trim();
        let Some(rest) = trim.strip_prefix(TAG_PREFIX) else {
            continue;
        };
        let Some(value) = rest.strip_suffix(TAG_SUFFIX) else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        return Some(value.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- escape_md_cell ------------------------------------------------------

    #[test]
    fn escape_md_cell_passes_through_plain() {
        assert_eq!(escape_md_cell("hello"), "hello");
    }

    #[test]
    fn escape_md_cell_escapes_pipes() {
        assert_eq!(escape_md_cell("a|b|c"), "a\\|b\\|c");
    }

    #[test]
    fn escape_md_cell_escapes_keep_marker_open_and_close() {
        let raw = "name <!-- keep evil --> end";
        let esc = escape_md_cell(raw);
        // Both <!-- and --> must be neutralized so a malicious project name
        // cannot inject a fake keep marker into the table.
        assert!(!esc.contains("<!--"));
        assert!(!esc.contains("-->"));
        assert!(esc.contains("&lt;!--"));
        assert!(esc.contains("--&gt;"));
    }

    #[test]
    fn escape_md_cell_strips_newlines_and_cr() {
        assert_eq!(escape_md_cell("a\nb\r\nc"), "a b  c");
    }

    #[test]
    fn escape_md_cell_code_strips_backticks() {
        assert_eq!(escape_md_cell_code("abc`evil`def"), "abcevildef");
    }

    #[test]
    fn escape_md_cell_code_combines_all_rules() {
        let raw = "x|y\n<!--z`q-->";
        let esc = escape_md_cell_code(raw);
        assert!(!esc.contains('|') || esc.contains("\\|"));
        assert!(!esc.contains('`'));
        assert!(!esc.contains("<!--"));
        assert!(!esc.contains("-->"));
    }

    // --- cas:full_name tag ---------------------------------------------------

    #[test]
    fn emit_then_parse_round_trips_plain_value() {
        let v = "acme/widget";
        let body = emit_cas_full_name_tag(v);
        assert_eq!(parse_cas_full_name_tag(&body).as_deref(), Some(v));
    }

    #[test]
    fn parse_returns_none_when_tag_missing() {
        assert_eq!(parse_cas_full_name_tag("# heading\nno tag here\n"), None);
    }

    #[test]
    fn parse_returns_none_for_malformed_tag() {
        // Missing trailing space before -->.
        assert_eq!(
            parse_cas_full_name_tag("<!-- cas:full_name=foo-->"),
            None
        );
        // Wrong prefix.
        assert_eq!(
            parse_cas_full_name_tag("<!-- cas:fullname=foo -->"),
            None
        );
    }

    #[test]
    fn parse_first_tag_when_multiple() {
        let body = "<!-- cas:full_name=first -->\n<!-- cas:full_name=second -->\n";
        assert_eq!(
            parse_cas_full_name_tag(body).as_deref(),
            Some("first")
        );
    }

    #[test]
    fn parse_trims_whitespace_around_value() {
        let body = "<!-- cas:full_name=   acme/widget   -->";
        assert_eq!(
            parse_cas_full_name_tag(body).as_deref(),
            Some("acme/widget")
        );
    }

    #[test]
    fn parse_ignores_empty_value() {
        assert_eq!(
            parse_cas_full_name_tag("<!-- cas:full_name= -->"),
            None
        );
    }

    #[test]
    fn emit_neutralizes_close_marker_in_value() {
        // A value containing literal `-->` cannot be allowed to terminate the
        // tag early; emit must rewrite it.
        let v = "evil-->payload";
        let tag = emit_cas_full_name_tag(v);
        // Tag must end exactly once with " -->" (the proper terminator).
        assert!(tag.ends_with(" -->"));
        // And the body must NOT round-trip the literal `-->`.
        let inner = tag
            .strip_prefix(TAG_PREFIX)
            .unwrap()
            .strip_suffix(TAG_SUFFIX)
            .unwrap();
        assert!(!inner.contains("-->"));
        // Round-tripping through parse returns the rewritten form (lossy but
        // safe). Callers that need exact preservation should validate the
        // value before emit.
        let parsed = parse_cas_full_name_tag(&tag).unwrap();
        assert_eq!(parsed, "evil--&gt;payload");
    }

    #[test]
    fn emit_strips_newlines_in_value() {
        let v = "line1\nline2";
        let tag = emit_cas_full_name_tag(v);
        // Tag must remain a single line.
        assert_eq!(tag.lines().count(), 1);
    }
}
