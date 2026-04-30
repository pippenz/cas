//! Multi-persona code review findings schema.
//!
//! This module defines the structured contract that all eight reviewer
//! personas (correctness, testing, maintainability, project-standards,
//! fallow, security, performance, adversarial) emit, and that the
//! orchestrator merges, dedupes, and routes.
//!
//! The shape is fixed by the Phase 1 subsystem A brainstorm
//! (`docs/brainstorms/2026-04-09-multi-persona-code-review-requirements.md`,
//! requirement R3). A corresponding human-readable schema reference lives at
//! `cas-cli/src/builtins/skills/cas-code-review/references/findings-schema.md`
//! and is embedded into the `cas-code-review` skill by Unit 6.
//!
//! # Validation
//!
//! Deserializing a `Finding` or `ReviewerOutput` via serde performs *structural*
//! validation (unknown enum variants are rejected by `#[serde(deny_unknown_fields)]`
//! and the `rename_all` tags). Semantic validation — title length, confidence
//! bounds, non-empty evidence, relative file path — is enforced by calling
//! [`Finding::validate`] / [`ReviewerOutput::validate`] or the convenience
//! [`parse_reviewer_output`] helper which does both in one step.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Maximum title length in characters (per R3).
pub const MAX_TITLE_LEN: usize = 100;

/// Severity level assigned by a reviewer persona.
///
/// Mapped 1:1 to CAS task priorities by the review-to-task flow:
/// `P0→0, P1→1, P2→2, P3→3`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Severity {
    /// Blocker. Hard-blocks `task.close` in autofix mode.
    P0,
    /// High.
    P1,
    /// Medium.
    P2,
    /// Low.
    P3,
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Severity::P0 => "P0",
            Severity::P1 => "P1",
            Severity::P2 => "P2",
            Severity::P3 => "P3",
        };
        f.write_str(s)
    }
}

/// Autofix routing class — how the orchestrator should handle the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutofixClass {
    /// Safe to apply automatically by the fixer sub-agent.
    SafeAuto,
    /// Can be auto-applied, but only behind an explicit gate / human ack.
    GatedAuto,
    /// Requires a human or downstream agent to edit code.
    Manual,
    /// Informational only; never becomes a task, never auto-applied.
    Advisory,
}

impl fmt::Display for AutofixClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            AutofixClass::SafeAuto => "safe_auto",
            AutofixClass::GatedAuto => "gated_auto",
            AutofixClass::Manual => "manual",
            AutofixClass::Advisory => "advisory",
        };
        f.write_str(s)
    }
}

/// Who owns resolving a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Owner {
    /// The fixer sub-agent inside cas-code-review.
    ReviewFixer,
    /// A downstream CAS task (review-to-task flow).
    DownstreamResolver,
    /// A human operator — must not be routed to automation.
    Human,
}

impl fmt::Display for Owner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Owner::ReviewFixer => "review-fixer",
            Owner::DownstreamResolver => "downstream-resolver",
            Owner::Human => "human",
        };
        f.write_str(s)
    }
}

/// A single code-review finding emitted by a persona.
///
/// All fields required by the schema in R3 are present. Use
/// [`Finding::validate`] after deserialization to enforce the semantic rules
/// that serde cannot express.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    /// Short human-readable label (≤ [`MAX_TITLE_LEN`] characters).
    pub title: String,
    /// Severity (P0..P3).
    pub severity: Severity,
    /// File path, **relative** to the repository root.
    pub file: String,
    /// 1-based line number of the finding.
    pub line: u32,
    /// Why this matters — the consequence if unaddressed.
    pub why_it_matters: String,
    /// How the orchestrator should handle the fix.
    pub autofix_class: AutofixClass,
    /// Who owns resolving the finding.
    pub owner: Owner,
    /// Reviewer confidence in the finding, 0.0..=1.0 inclusive.
    pub confidence: f32,
    /// Code-grounded evidence strings. Must contain at least one entry.
    pub evidence: Vec<String>,
    /// True if the finding exists on `main` / pre-diff; false if introduced
    /// by the change under review.
    pub pre_existing: bool,
    /// Optional concrete suggested fix (diff, patch hint, or prose).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_fix: Option<String>,
    /// Whether the finding requires re-verification after a fix.
    #[serde(default)]
    pub requires_verification: bool,
}

/// Errors returned by [`Finding::validate`] / [`ReviewerOutput::validate`].
#[derive(Debug, Clone, PartialEq)]
pub enum FindingValidationError {
    /// Title is empty.
    EmptyTitle,
    /// Title exceeds [`MAX_TITLE_LEN`].
    TitleTooLong { len: usize, max: usize },
    /// `confidence` is NaN or outside 0.0..=1.0.
    ConfidenceOutOfRange { value: f32 },
    /// `evidence` array is empty.
    EmptyEvidence,
    /// A single evidence string is empty / whitespace.
    EmptyEvidenceEntry { index: usize },
    /// `file` is empty.
    EmptyFilePath,
    /// `file` is an absolute path.
    AbsoluteFilePath { path: String },
    /// `why_it_matters` is empty.
    EmptyWhyItMatters,
    /// Reviewer name is empty.
    EmptyReviewerName,
}

impl fmt::Display for FindingValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyTitle => write!(f, "finding title is empty"),
            Self::TitleTooLong { len, max } => {
                write!(f, "finding title is {len} chars, max is {max}")
            }
            Self::ConfidenceOutOfRange { value } => {
                write!(f, "confidence {value} out of range 0.0..=1.0")
            }
            Self::EmptyEvidence => write!(f, "finding evidence array is empty"),
            Self::EmptyEvidenceEntry { index } => {
                write!(f, "finding evidence[{index}] is empty")
            }
            Self::EmptyFilePath => write!(f, "finding file path is empty"),
            Self::AbsoluteFilePath { path } => {
                write!(f, "finding file path must be relative, got '{path}'")
            }
            Self::EmptyWhyItMatters => write!(f, "finding why_it_matters is empty"),
            Self::EmptyReviewerName => write!(f, "reviewer name is empty"),
        }
    }
}

impl std::error::Error for FindingValidationError {}

fn is_absolute_path(path: &str) -> bool {
    // Treat Unix absolute paths and Windows drive/UNC paths as absolute.
    // Relative paths like `src/foo.rs` or `./src/foo.rs` are fine.
    if path.starts_with('/') || path.starts_with('\\') {
        return true;
    }
    // Windows drive: `C:\...` or `C:/...`
    let bytes = path.as_bytes();
    if bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && (bytes[2] == b'/' || bytes[2] == b'\\')
    {
        return true;
    }
    false
}

impl Finding {
    /// Enforce the semantic rules that serde cannot express.
    pub fn validate(&self) -> Result<(), FindingValidationError> {
        if self.title.trim().is_empty() {
            return Err(FindingValidationError::EmptyTitle);
        }
        let title_len = self.title.chars().count();
        if title_len > MAX_TITLE_LEN {
            return Err(FindingValidationError::TitleTooLong {
                len: title_len,
                max: MAX_TITLE_LEN,
            });
        }
        if !(self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence)) {
            return Err(FindingValidationError::ConfidenceOutOfRange {
                value: self.confidence,
            });
        }
        if self.evidence.is_empty() {
            return Err(FindingValidationError::EmptyEvidence);
        }
        for (i, e) in self.evidence.iter().enumerate() {
            if e.trim().is_empty() {
                return Err(FindingValidationError::EmptyEvidenceEntry { index: i });
            }
        }
        if self.file.trim().is_empty() {
            return Err(FindingValidationError::EmptyFilePath);
        }
        if is_absolute_path(&self.file) {
            return Err(FindingValidationError::AbsoluteFilePath {
                path: self.file.clone(),
            });
        }
        if self.why_it_matters.trim().is_empty() {
            return Err(FindingValidationError::EmptyWhyItMatters);
        }
        Ok(())
    }
}

impl fmt::Display for Finding {
    /// Orchestrator-friendly single-finding rendering.
    ///
    /// Format: `[P0] title — file:line (autofix_class, owner, conf=0.87)`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[{sev}] {title} — {file}:{line} ({class}, {owner}, conf={conf:.2})",
            sev = self.severity,
            title = self.title,
            file = self.file,
            line = self.line,
            class = self.autofix_class,
            owner = self.owner,
            conf = self.confidence,
        )
    }
}

/// The full envelope a single persona returns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewerOutput {
    /// Reviewer persona name, e.g. `"correctness"`, `"security"`.
    pub reviewer: String,
    /// Findings emitted by this reviewer (may be empty).
    #[serde(default)]
    pub findings: Vec<Finding>,
    /// Risks the reviewer saw but could not confirm — surfaced to the
    /// orchestrator but never turned into tasks automatically.
    #[serde(default)]
    pub residual_risks: Vec<String>,
    /// Testing gaps the reviewer noticed (e.g., "no test covers the retry path").
    #[serde(default)]
    pub testing_gaps: Vec<String>,
}

impl ReviewerOutput {
    /// Validate every contained finding and the envelope fields themselves.
    pub fn validate(&self) -> Result<(), FindingValidationError> {
        if self.reviewer.trim().is_empty() {
            return Err(FindingValidationError::EmptyReviewerName);
        }
        for f in &self.findings {
            f.validate()?;
        }
        Ok(())
    }
}

impl fmt::Display for ReviewerOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "== reviewer: {} ({} findings) ==",
            self.reviewer,
            self.findings.len()
        )?;
        for finding in &self.findings {
            writeln!(f, "  {finding}")?;
        }
        if !self.residual_risks.is_empty() {
            writeln!(f, "  residual_risks:")?;
            for r in &self.residual_risks {
                writeln!(f, "    - {r}")?;
            }
        }
        if !self.testing_gaps.is_empty() {
            writeln!(f, "  testing_gaps:")?;
            for g in &self.testing_gaps {
                writeln!(f, "    - {g}")?;
            }
        }
        Ok(())
    }
}

/// Parse a JSON string into a [`ReviewerOutput`] and run semantic validation.
///
/// Returns a boxed error so callers can mix serde and validation errors freely.
pub fn parse_reviewer_output(json: &str) -> Result<ReviewerOutput, Box<dyn std::error::Error>> {
    let out: ReviewerOutput = serde_json::from_str(json)?;
    out.validate()?;
    Ok(out)
}

/// End-to-end review outcome that the cas-code-review skill passes back
/// to the `task.close` MCP handler via `TaskCloseRequest.code_review_findings`.
///
/// This is the structured envelope the worker assembles from the
/// orchestrator → merge → autofix → review-to-task pipeline before it
/// retries `task.close`. The close-gate logic in close_ops.rs reads
/// `residual` to decide whether to P0-block the close.
///
/// The shape is intentionally thin — the heavy lifting (dedup, routing,
/// confidence gating, autofix) is already done by the time the worker
/// emits this envelope. Here we only need what the gate and the audit
/// trail depend on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewOutcome {
    /// Findings that survived the autofix loop — i.e., anything the
    /// fixer sub-agent did not resolve within the bounded 2-round
    /// budget. The close gate blocks on any `severity == P0` entry
    /// that is not `pre_existing`.
    #[serde(default)]
    pub residual: Vec<Finding>,
    /// Findings that existed on the base ref, surfaced for context but
    /// never blocking. Separated from `residual` per R4 / R9.
    #[serde(default)]
    pub pre_existing: Vec<Finding>,
    /// Invocation mode the review ran in: `"autofix"`, `"interactive"`,
    /// `"report-only"`, or `"headless"`. Required so the audit trail
    /// and downstream consumers can tell whether the envelope came
    /// from the primary close-gate path or an out-of-band invocation.
    pub mode: String,
}

impl ReviewOutcome {
    /// Validate every contained finding + the mode string. Called by
    /// the close gate before it trusts the envelope — a malformed
    /// envelope from the worker is treated like a reviewer error, not
    /// a free pass.
    pub fn validate(&self) -> Result<(), FindingValidationError> {
        if self.mode.trim().is_empty() {
            return Err(FindingValidationError::EmptyReviewerName);
        }
        for f in &self.residual {
            f.validate()?;
        }
        for f in &self.pre_existing {
            f.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_finding() -> Finding {
        Finding {
            title: "Unwrap on parsed int can panic".to_string(),
            severity: Severity::P1,
            file: "src/foo.rs".to_string(),
            line: 42,
            why_it_matters: "Panics on malformed user input, crashing the worker.".to_string(),
            autofix_class: AutofixClass::SafeAuto,
            owner: Owner::ReviewFixer,
            confidence: 0.85,
            evidence: vec!["let n: u32 = s.parse().unwrap();".to_string()],
            pre_existing: false,
            suggested_fix: Some("Use `?` or `.map_err(...)`".to_string()),
            requires_verification: false,
        }
    }

    #[test]
    fn finding_roundtrip() {
        let f = valid_finding();
        let json = serde_json::to_string(&f).unwrap();
        let back: Finding = serde_json::from_str(&json).unwrap();
        assert_eq!(f, back);
    }

    #[test]
    fn finding_validates_happy_path() {
        valid_finding().validate().unwrap();
    }

    #[test]
    fn title_too_long_rejected() {
        let mut f = valid_finding();
        f.title = "x".repeat(MAX_TITLE_LEN + 1);
        assert_eq!(
            f.validate().unwrap_err(),
            FindingValidationError::TitleTooLong {
                len: MAX_TITLE_LEN + 1,
                max: MAX_TITLE_LEN,
            }
        );
    }

    #[test]
    fn title_exactly_max_ok() {
        let mut f = valid_finding();
        f.title = "x".repeat(MAX_TITLE_LEN);
        f.validate().unwrap();
    }

    #[test]
    fn empty_title_rejected() {
        let mut f = valid_finding();
        f.title = "   ".to_string();
        assert_eq!(f.validate().unwrap_err(), FindingValidationError::EmptyTitle);
    }

    #[test]
    fn confidence_below_zero_rejected() {
        let mut f = valid_finding();
        f.confidence = -0.01;
        assert!(matches!(
            f.validate().unwrap_err(),
            FindingValidationError::ConfidenceOutOfRange { .. }
        ));
    }

    #[test]
    fn confidence_above_one_rejected() {
        let mut f = valid_finding();
        f.confidence = 1.01;
        assert!(matches!(
            f.validate().unwrap_err(),
            FindingValidationError::ConfidenceOutOfRange { .. }
        ));
    }

    #[test]
    fn confidence_nan_rejected() {
        let mut f = valid_finding();
        f.confidence = f32::NAN;
        assert!(matches!(
            f.validate().unwrap_err(),
            FindingValidationError::ConfidenceOutOfRange { .. }
        ));
    }

    #[test]
    fn confidence_bounds_inclusive() {
        let mut f = valid_finding();
        f.confidence = 0.0;
        f.validate().unwrap();
        f.confidence = 1.0;
        f.validate().unwrap();
    }

    #[test]
    fn empty_evidence_rejected() {
        let mut f = valid_finding();
        f.evidence.clear();
        assert_eq!(
            f.validate().unwrap_err(),
            FindingValidationError::EmptyEvidence
        );
    }

    #[test]
    fn empty_evidence_entry_rejected() {
        let mut f = valid_finding();
        f.evidence = vec!["ok".to_string(), "   ".to_string()];
        assert_eq!(
            f.validate().unwrap_err(),
            FindingValidationError::EmptyEvidenceEntry { index: 1 }
        );
    }

    #[test]
    fn absolute_file_path_rejected_unix() {
        let mut f = valid_finding();
        f.file = "/etc/passwd".to_string();
        assert!(matches!(
            f.validate().unwrap_err(),
            FindingValidationError::AbsoluteFilePath { .. }
        ));
    }

    #[test]
    fn absolute_file_path_rejected_windows() {
        let mut f = valid_finding();
        f.file = "C:\\Users\\foo.rs".to_string();
        assert!(matches!(
            f.validate().unwrap_err(),
            FindingValidationError::AbsoluteFilePath { .. }
        ));
    }

    #[test]
    fn relative_dotted_path_ok() {
        let mut f = valid_finding();
        f.file = "./src/foo.rs".to_string();
        f.validate().unwrap();
    }

    #[test]
    fn empty_file_rejected() {
        let mut f = valid_finding();
        f.file = "".to_string();
        assert_eq!(
            f.validate().unwrap_err(),
            FindingValidationError::EmptyFilePath
        );
    }

    #[test]
    fn empty_why_rejected() {
        let mut f = valid_finding();
        f.why_it_matters = "".to_string();
        assert_eq!(
            f.validate().unwrap_err(),
            FindingValidationError::EmptyWhyItMatters
        );
    }

    #[test]
    fn invalid_severity_enum_rejected() {
        let json = r#"{
            "title":"t","severity":"P5","file":"a.rs","line":1,
            "why_it_matters":"w","autofix_class":"safe_auto","owner":"human",
            "confidence":0.5,"evidence":["e"],"pre_existing":false
        }"#;
        assert!(serde_json::from_str::<Finding>(json).is_err());
    }

    #[test]
    fn invalid_autofix_class_rejected() {
        let json = r#"{
            "title":"t","severity":"P1","file":"a.rs","line":1,
            "why_it_matters":"w","autofix_class":"yolo","owner":"human",
            "confidence":0.5,"evidence":["e"],"pre_existing":false
        }"#;
        assert!(serde_json::from_str::<Finding>(json).is_err());
    }

    #[test]
    fn unknown_field_rejected() {
        let json = r#"{
            "title":"t","severity":"P1","file":"a.rs","line":1,
            "why_it_matters":"w","autofix_class":"safe_auto","owner":"human",
            "confidence":0.5,"evidence":["e"],"pre_existing":false,
            "bogus_field":"x"
        }"#;
        assert!(serde_json::from_str::<Finding>(json).is_err());
    }

    #[test]
    fn severity_display() {
        assert_eq!(Severity::P0.to_string(), "P0");
        assert_eq!(Severity::P3.to_string(), "P3");
    }

    #[test]
    fn autofix_class_display() {
        assert_eq!(AutofixClass::SafeAuto.to_string(), "safe_auto");
        assert_eq!(AutofixClass::GatedAuto.to_string(), "gated_auto");
        assert_eq!(AutofixClass::Advisory.to_string(), "advisory");
    }

    #[test]
    fn owner_display() {
        assert_eq!(Owner::ReviewFixer.to_string(), "review-fixer");
        assert_eq!(Owner::DownstreamResolver.to_string(), "downstream-resolver");
        assert_eq!(Owner::Human.to_string(), "human");
    }

    #[test]
    fn reviewer_output_roundtrip() {
        let out = ReviewerOutput {
            reviewer: "correctness".to_string(),
            findings: vec![valid_finding()],
            residual_risks: vec!["retry path not verified".to_string()],
            testing_gaps: vec!["no integration test".to_string()],
        };
        let json = serde_json::to_string(&out).unwrap();
        let back: ReviewerOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(out, back);
        back.validate().unwrap();
    }

    #[test]
    fn reviewer_output_empty_reviewer_name_rejected() {
        let out = ReviewerOutput {
            reviewer: " ".to_string(),
            findings: vec![],
            residual_risks: vec![],
            testing_gaps: vec![],
        };
        assert_eq!(
            out.validate().unwrap_err(),
            FindingValidationError::EmptyReviewerName
        );
    }

    #[test]
    fn reviewer_output_propagates_finding_error() {
        let mut bad = valid_finding();
        bad.confidence = 2.0;
        let out = ReviewerOutput {
            reviewer: "correctness".to_string(),
            findings: vec![bad],
            residual_risks: vec![],
            testing_gaps: vec![],
        };
        assert!(matches!(
            out.validate().unwrap_err(),
            FindingValidationError::ConfidenceOutOfRange { .. }
        ));
    }

    #[test]
    fn reviewer_output_defaults_for_optional_arrays() {
        let json = r#"{"reviewer":"security","findings":[]}"#;
        let out: ReviewerOutput = serde_json::from_str(json).unwrap();
        assert!(out.residual_risks.is_empty());
        assert!(out.testing_gaps.is_empty());
        out.validate().unwrap();
    }

    #[test]
    fn parse_reviewer_output_helper_happy_path() {
        let out = ReviewerOutput {
            reviewer: "testing".to_string(),
            findings: vec![valid_finding()],
            residual_risks: vec![],
            testing_gaps: vec![],
        };
        let json = serde_json::to_string(&out).unwrap();
        let parsed = parse_reviewer_output(&json).unwrap();
        assert_eq!(parsed, out);
    }

    #[test]
    fn parse_reviewer_output_helper_rejects_bad_path() {
        let json = r#"{
            "reviewer":"testing",
            "findings":[{
                "title":"t","severity":"P1","file":"/abs/path.rs","line":1,
                "why_it_matters":"w","autofix_class":"safe_auto","owner":"human",
                "confidence":0.5,"evidence":["e"],"pre_existing":false
            }]
        }"#;
        assert!(parse_reviewer_output(json).is_err());
    }

    #[test]
    fn finding_display_format() {
        let f = valid_finding();
        let s = f.to_string();
        assert!(s.starts_with("[P1] Unwrap on parsed int can panic — src/foo.rs:42"));
        assert!(s.contains("safe_auto"));
        assert!(s.contains("review-fixer"));
        assert!(s.contains("conf=0.85"));
    }

    #[test]
    fn reviewer_output_display_format() {
        let out = ReviewerOutput {
            reviewer: "correctness".to_string(),
            findings: vec![valid_finding()],
            residual_risks: vec!["r1".to_string()],
            testing_gaps: vec!["g1".to_string()],
        };
        let s = out.to_string();
        assert!(s.contains("== reviewer: correctness (1 findings) =="));
        assert!(s.contains("residual_risks:"));
        assert!(s.contains("- r1"));
        assert!(s.contains("testing_gaps:"));
        assert!(s.contains("- g1"));
    }
}
