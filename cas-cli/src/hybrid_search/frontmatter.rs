//! Memory frontmatter parsing for the search index.
//!
//! Extracts the structured fields defined by the memory schema
//! (`track`, `module`, `problem_type`, `severity`, `root_cause`, `date`)
//! from YAML frontmatter at the top of a memory body. Legacy memories
//! without frontmatter — or with only the legacy fields
//! (`name`/`description`/`type`) — yield an empty result and are still
//! searchable by keyword.

use serde::Deserialize;

/// Structured frontmatter fields extracted from a memory body.
///
/// All fields are optional so that legacy memories (and partial frontmatter)
/// parse cleanly. Missing fields are simply not indexed as filter terms.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct FrontmatterFields {
    #[serde(default)]
    pub track: Option<String>,
    #[serde(default)]
    pub module: Option<String>,
    #[serde(default)]
    pub problem_type: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub root_cause: Option<String>,
    #[serde(default)]
    pub date: Option<String>,
}

impl FrontmatterFields {
    /// True when no structured field was parsed. Used to short-circuit
    /// indexing for legacy memories.
    pub fn is_empty(&self) -> bool {
        self.track.is_none()
            && self.module.is_none()
            && self.problem_type.is_none()
            && self.severity.is_none()
            && self.root_cause.is_none()
            && self.date.is_none()
    }
}

/// Parse YAML frontmatter from a memory body, if present.
///
/// Returns an empty `FrontmatterFields` when:
/// - the body does not start with `---`
/// - the frontmatter block is malformed YAML
/// - the frontmatter has only legacy fields (no structured fields)
///
/// This function never errors — it is best-effort by design. Callers
/// always index the body text regardless of frontmatter parseability.
pub fn extract_frontmatter_fields(body: &str) -> FrontmatterFields {
    let trimmed = body.trim_start();
    if !trimmed.starts_with("---") {
        return FrontmatterFields::default();
    }

    // Split ---\nYAML\n---\n<rest>
    // Use splitn(3, "---") so the first element is empty, second is YAML,
    // third is the body. Matches cas-cli/src/store/markdown.rs:37.
    let parts: Vec<&str> = trimmed.splitn(3, "---").collect();
    if parts.len() < 3 {
        return FrontmatterFields::default();
    }
    let yaml = parts[1].trim();
    if yaml.is_empty() {
        return FrontmatterFields::default();
    }

    serde_yaml::from_str::<FrontmatterFields>(yaml).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_structured_frontmatter() {
        let body = "---\n\
                    name: x\n\
                    type: bugfix\n\
                    track: bug\n\
                    module: cas-mcp\n\
                    problem_type: runtime_error\n\
                    severity: critical\n\
                    root_cause: environment\n\
                    date: 2026-03-30\n\
                    ---\nbody";
        let f = extract_frontmatter_fields(body);
        assert_eq!(f.track.as_deref(), Some("bug"));
        assert_eq!(f.module.as_deref(), Some("cas-mcp"));
        assert_eq!(f.problem_type.as_deref(), Some("runtime_error"));
        assert_eq!(f.severity.as_deref(), Some("critical"));
        assert_eq!(f.root_cause.as_deref(), Some("environment"));
        assert_eq!(f.date.as_deref(), Some("2026-03-30"));
        assert!(!f.is_empty());
    }

    #[test]
    fn legacy_only_fields_yield_empty() {
        let body = "---\n\
                    name: legacy\n\
                    description: thing\n\
                    type: learning\n\
                    ---\nbody";
        let f = extract_frontmatter_fields(body);
        assert!(f.is_empty());
    }

    #[test]
    fn no_frontmatter_yields_empty() {
        let f = extract_frontmatter_fields("just some text about stuff");
        assert!(f.is_empty());
    }

    #[test]
    fn malformed_yaml_yields_empty() {
        let body = "---\nnot: [valid yaml\n---\nbody";
        let f = extract_frontmatter_fields(body);
        assert!(f.is_empty());
    }

    #[test]
    fn partial_structured_fields() {
        let body = "---\nmodule: cas-search\nseverity: high\n---\nbody";
        let f = extract_frontmatter_fields(body);
        assert_eq!(f.module.as_deref(), Some("cas-search"));
        assert_eq!(f.severity.as_deref(), Some("high"));
        assert!(f.track.is_none());
        assert!(!f.is_empty());
    }
}
