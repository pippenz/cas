//! Multi-persona code-review merge pipeline (Phase 1 Subsystem A, Unit 5).
//!
//! Pure-Rust function that takes N [`ReviewerOutput`] envelopes (one per
//! persona) and turns them into a single [`MergedFindings`] result that
//! downstream units (autofix, review-to-task, close-gate) consume.
//!
//! The pipeline is the exact sequence required by requirement R4 of
//! `docs/brainstorms/2026-04-09-multi-persona-code-review-requirements.md`:
//!
//! 1. **Schema validation** — every envelope must round-trip the semantic
//!    validators in `cas_types::code_review` (via
//!    [`ReviewerOutput::validate`]). A single bad envelope fails the whole
//!    merge — persona output is a contract and silent drops would hide bugs.
//! 2. **Confidence gate** — findings below `CONFIDENCE_GATE` (0.60) are
//!    dropped, **except** P0 findings at or above `P0_CONFIDENCE_FLOOR`
//!    (0.50) which are always kept. Rationale: critical-but-uncertain
//!    issues must surface.
//! 3. **Fingerprint deduplication** — findings are grouped by
//!    `(normalize(file), normalize(title))`, then within each group
//!    findings whose `line` is within `LINE_BUCKET_RADIUS` (±3) of an
//!    existing cluster are merged into it. Merging keeps the highest
//!    severity, highest confidence, the union of `evidence` entries, and
//!    tracks contributing reviewers.
//! 4. **Cross-reviewer agreement boost** — when a merged finding has 2 or
//!    more distinct contributing reviewers, merged confidence is bumped by
//!    `AGREEMENT_BOOST` (+0.10), capped at 1.0.
//! 5. **Pre-existing separation** — findings are partitioned into a
//!    `pr_introduced` list (`pre_existing == false`) and a `pre_existing`
//!    list. Only the former gates `task.close`.
//! 6. **Conservative routing** — when contributors disagree on `owner` or
//!    `autofix_class`, the most restrictive option wins: for `owner` that
//!    is `human > downstream-resolver > review-fixer`; for `autofix_class`
//!    it is `advisory > manual > gated_auto > safe_auto` (per R4). Each
//!    disagreement is recorded in [`MergedFindings::diagnostics`].
//! 7. **Severity-sorted presentation** — both output lists are sorted by
//!    `(severity DESC, confidence DESC, file ASC, line ASC)`.
//!
//! # Scope
//!
//! This module is deliberately pure: no I/O, no MCP surface, no dispatch,
//! no autofix loop. Unit 4 calls the personas; this unit takes whatever
//! they returned and produces the merged verdict.

use std::collections::BTreeSet;

use cas_types::{AutofixClass, Finding, FindingSeverity, Owner, ReviewerOutput};

/// Findings with confidence below this threshold are dropped, except for
/// P0 at or above [`P0_CONFIDENCE_FLOOR`].
pub const CONFIDENCE_GATE: f32 = 0.60;

/// P0 findings are kept as long as their confidence is at least this value,
/// even if they fall below [`CONFIDENCE_GATE`].
pub const P0_CONFIDENCE_FLOOR: f32 = 0.50;

/// How much to add to merged confidence when two or more reviewers hit the
/// same fingerprint. Capped at 1.0.
pub const AGREEMENT_BOOST: f32 = 0.10;

/// Two findings with the same normalized (file, title) are considered the
/// same issue when their `line` numbers are within this radius of each
/// other (inclusive).
pub const LINE_BUCKET_RADIUS: u32 = 3;

/// Errors returned when the merge pipeline cannot proceed.
#[derive(Debug, Clone, PartialEq)]
pub enum MergeError {
    /// A [`ReviewerOutput`] failed semantic validation. Contains the
    /// reviewer name (or `"<unknown>"`) and the underlying error rendered
    /// as a string so this type doesn't need to know about every validator
    /// variant.
    InvalidReviewerOutput { reviewer: String, reason: String },
}

impl std::fmt::Display for MergeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidReviewerOutput { reviewer, reason } => {
                write!(f, "reviewer '{reviewer}' returned invalid output: {reason}")
            }
        }
    }
}

impl std::error::Error for MergeError {}

/// Diagnostic emitted when the pipeline has to make a judgment call the
/// caller should be aware of (mainly routing disagreements).
#[derive(Debug, Clone, PartialEq)]
pub enum MergeDiagnostic {
    /// Two or more contributing reviewers disagreed on `owner` for the
    /// same merged fingerprint.
    OwnerDisagreement {
        fingerprint: String,
        contributors: Vec<String>,
        chosen: Owner,
        candidates: Vec<Owner>,
    },
    /// Two or more contributing reviewers disagreed on `autofix_class`.
    AutofixClassDisagreement {
        fingerprint: String,
        contributors: Vec<String>,
        chosen: AutofixClass,
        candidates: Vec<AutofixClass>,
    },
}

/// Result of merging N persona outputs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MergedFindings {
    /// Findings introduced by the diff under review. This is the list that
    /// gates `task.close`.
    pub pr_introduced: Vec<Finding>,
    /// Findings that already existed on the base ref. Surfaced for context
    /// but never blocking and never auto-routed.
    pub pre_existing: Vec<Finding>,
    /// Non-fatal notes from the merge — routing disagreements, etc.
    pub diagnostics: Vec<MergeDiagnostic>,
}

/// Run the full merge pipeline on a set of persona envelopes.
///
/// See the module docs for the stage-by-stage behavior.
pub fn merge_findings(envelopes: Vec<ReviewerOutput>) -> Result<MergedFindings, MergeError> {
    // Stage 1: schema validation.
    for env in &envelopes {
        env.validate().map_err(|e| MergeError::InvalidReviewerOutput {
            reviewer: if env.reviewer.trim().is_empty() {
                "<unknown>".to_string()
            } else {
                env.reviewer.clone()
            },
            reason: e.to_string(),
        })?;
    }

    // Stage 2: confidence gate + attach reviewer name to each finding.
    let mut gated: Vec<(String, Finding)> = Vec::new();
    for env in envelopes {
        for f in env.findings {
            if passes_confidence_gate(&f) {
                gated.push((env.reviewer.clone(), f));
            }
        }
    }

    // Stage 3: fingerprint dedup.
    let clusters = cluster_by_fingerprint(gated);

    // Stages 4 + 6: merge each cluster, emit diagnostics.
    let mut diagnostics: Vec<MergeDiagnostic> = Vec::new();
    let mut merged: Vec<Finding> = Vec::with_capacity(clusters.len());
    for cluster in clusters {
        let (finding, mut diag) = merge_cluster(cluster);
        diagnostics.append(&mut diag);
        merged.push(finding);
    }

    // Stage 5: pre_existing partition.
    let (pre_existing, mut pr_introduced): (Vec<_>, Vec<_>) =
        merged.into_iter().partition(|f| f.pre_existing);
    let mut pre_existing = pre_existing;

    // Stage 7: stable ordered presentation.
    sort_findings(&mut pr_introduced);
    sort_findings(&mut pre_existing);

    Ok(MergedFindings {
        pr_introduced,
        pre_existing,
        diagnostics,
    })
}

// ---------- stage helpers (also exposed to unit tests) -------------------

/// Stage 2: does this finding survive the confidence gate?
fn passes_confidence_gate(f: &Finding) -> bool {
    if f.confidence >= CONFIDENCE_GATE {
        return true;
    }
    f.severity == FindingSeverity::P0 && f.confidence >= P0_CONFIDENCE_FLOOR
}

/// A cluster of findings that are all "the same issue" per the fingerprint
/// rule. The `anchor_line` is the line of the first finding in the cluster
/// and is used for the ±3 merge window against later arrivals.
#[derive(Debug)]
struct Cluster {
    fingerprint_key: String,
    anchor_line: u32,
    entries: Vec<(String, Finding)>, // (reviewer, finding)
}

/// Stage 3: group gated findings into clusters whose `(norm_file, norm_title)`
/// match and whose `line` values fall within `LINE_BUCKET_RADIUS` of an
/// existing anchor.
fn cluster_by_fingerprint(gated: Vec<(String, Finding)>) -> Vec<Cluster> {
    let mut clusters: Vec<Cluster> = Vec::new();
    for (reviewer, f) in gated {
        let file_key = normalize_file(&f.file);
        let title_key = normalize_title(&f.title);
        let base_key = format!("{file_key}\u{001f}{title_key}");
        // Find any existing cluster with the same base key whose anchor
        // line is within the bucket radius.
        let hit = clusters.iter_mut().find(|c| {
            c.fingerprint_key == base_key && within_radius(c.anchor_line, f.line)
        });
        match hit {
            Some(c) => c.entries.push((reviewer, f)),
            None => {
                clusters.push(Cluster {
                    fingerprint_key: base_key,
                    anchor_line: f.line,
                    entries: vec![(reviewer, f)],
                });
            }
        }
    }
    clusters
}

fn within_radius(a: u32, b: u32) -> bool {
    a.abs_diff(b) <= LINE_BUCKET_RADIUS
}

/// Normalize a file path for fingerprinting: trim, convert `\` to `/`,
/// lowercase.
fn normalize_file(file: &str) -> String {
    file.trim().replace('\\', "/").to_ascii_lowercase()
}

/// Normalize a title for fingerprinting: lowercase, collapse whitespace.
fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Stages 4 + 6: merge a cluster into one [`Finding`] with appropriate
/// confidence boost and conservative routing. Returns the merged finding
/// plus any diagnostics the merge emitted.
fn merge_cluster(cluster: Cluster) -> (Finding, Vec<MergeDiagnostic>) {
    let fingerprint = cluster.fingerprint_key.clone();
    let Cluster { entries, .. } = cluster;
    debug_assert!(!entries.is_empty());

    // Contributors — distinct reviewer names in first-seen order.
    let mut contributors: Vec<String> = Vec::new();
    for (r, _) in &entries {
        if !contributors.iter().any(|c| c == r) {
            contributors.push(r.clone());
        }
    }

    // Max severity (lowest numeric enum = highest severity).
    let severity = entries
        .iter()
        .map(|(_, f)| f.severity)
        .max_by(|a, b| severity_rank(*a).cmp(&severity_rank(*b)))
        .expect("cluster non-empty");

    // Max confidence.
    let mut confidence = entries
        .iter()
        .map(|(_, f)| f.confidence)
        .fold(0.0_f32, f32::max);

    // Stage 4: agreement boost.
    if contributors.len() >= 2 {
        confidence = (confidence + AGREEMENT_BOOST).min(1.0);
    }

    // Union of evidence — deduped, preserving first-seen order.
    let mut evidence: Vec<String> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for (_, f) in &entries {
        for e in &f.evidence {
            if seen.insert(e.clone()) {
                evidence.push(e.clone());
            }
        }
    }

    // pre_existing — OR across the cluster. If ANY reviewer thinks it's
    // new, treat it as new. Rationale: false-negative on new-ness is
    // worse (fails to block a real regression) than false-positive.
    let pre_existing = entries.iter().all(|(_, f)| f.pre_existing);

    // requires_verification — OR across the cluster.
    let requires_verification = entries.iter().any(|(_, f)| f.requires_verification);

    // Conservative routing for owner.
    let (owner, owner_diag) = resolve_owner(&entries, &contributors, &fingerprint);
    // Conservative routing for autofix_class.
    let (autofix_class, class_diag) =
        resolve_autofix_class(&entries, &contributors, &fingerprint);

    // Use the title / file / line / why / suggested_fix from the
    // highest-confidence entry (deterministic across ties via first-seen).
    let anchor = entries
        .iter()
        .max_by(|a, b| {
            a.1.confidence
                .partial_cmp(&b.1.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("cluster non-empty");

    let anchor_finding = &anchor.1;
    // Prefer the anchor's suggested_fix to stay consistent with how
    // title/file/line/why_it_matters are picked; fall back to the first
    // contributor that offered one.
    let suggested_fix = anchor_finding
        .suggested_fix
        .clone()
        .or_else(|| entries.iter().find_map(|(_, f)| f.suggested_fix.clone()));

    let merged = Finding {
        title: anchor_finding.title.clone(),
        severity,
        file: anchor_finding.file.clone(),
        line: anchor_finding.line,
        why_it_matters: anchor_finding.why_it_matters.clone(),
        autofix_class,
        owner,
        confidence,
        evidence,
        pre_existing,
        suggested_fix,
        requires_verification,
    };

    let mut diags = Vec::new();
    if let Some(d) = owner_diag {
        diags.push(d);
    }
    if let Some(d) = class_diag {
        diags.push(d);
    }

    (merged, diags)
}

/// Severity rank for `max_by`: P0 > P1 > P2 > P3.
fn severity_rank(s: FindingSeverity) -> u8 {
    match s {
        FindingSeverity::P0 => 3,
        FindingSeverity::P1 => 2,
        FindingSeverity::P2 => 1,
        FindingSeverity::P3 => 0,
    }
}

/// Conservative owner selection: `human > downstream-resolver > review-fixer`.
fn owner_rank(o: Owner) -> u8 {
    match o {
        Owner::ReviewFixer => 0,
        Owner::DownstreamResolver => 1,
        Owner::Human => 2,
    }
}

/// Conservative autofix-class selection: `advisory > manual > gated_auto > safe_auto`.
fn autofix_class_rank(c: AutofixClass) -> u8 {
    match c {
        AutofixClass::SafeAuto => 0,
        AutofixClass::GatedAuto => 1,
        AutofixClass::Manual => 2,
        AutofixClass::Advisory => 3,
    }
}

fn resolve_owner(
    entries: &[(String, Finding)],
    contributors: &[String],
    fingerprint: &str,
) -> (Owner, Option<MergeDiagnostic>) {
    let mut candidates: Vec<Owner> = Vec::new();
    for (_, f) in entries {
        if !candidates.contains(&f.owner) {
            candidates.push(f.owner);
        }
    }
    let chosen = candidates
        .iter()
        .copied()
        .max_by_key(|o| owner_rank(*o))
        .expect("entries non-empty");
    let diag = if candidates.len() > 1 {
        Some(MergeDiagnostic::OwnerDisagreement {
            fingerprint: fingerprint.to_string(),
            contributors: contributors.to_vec(),
            chosen,
            candidates,
        })
    } else {
        None
    };
    (chosen, diag)
}

fn resolve_autofix_class(
    entries: &[(String, Finding)],
    contributors: &[String],
    fingerprint: &str,
) -> (AutofixClass, Option<MergeDiagnostic>) {
    let mut candidates: Vec<AutofixClass> = Vec::new();
    for (_, f) in entries {
        if !candidates.contains(&f.autofix_class) {
            candidates.push(f.autofix_class);
        }
    }
    let chosen = candidates
        .iter()
        .copied()
        .max_by_key(|c| autofix_class_rank(*c))
        .expect("entries non-empty");
    let diag = if candidates.len() > 1 {
        Some(MergeDiagnostic::AutofixClassDisagreement {
            fingerprint: fingerprint.to_string(),
            contributors: contributors.to_vec(),
            chosen,
            candidates,
        })
    } else {
        None
    };
    (chosen, diag)
}

/// Stage 7: severity DESC, confidence DESC, file ASC, line ASC.
fn sort_findings(xs: &mut [Finding]) {
    xs.sort_by(|a, b| {
        severity_rank(b.severity)
            .cmp(&severity_rank(a.severity))
            .then_with(|| {
                b.confidence
                    .partial_cmp(&a.confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
}

// --------------------------------------------------------------------------
// Tests
// --------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fnd(
        title: &str,
        sev: FindingSeverity,
        file: &str,
        line: u32,
        conf: f32,
        owner: Owner,
        class: AutofixClass,
        pre_existing: bool,
    ) -> Finding {
        Finding {
            title: title.to_string(),
            severity: sev,
            file: file.to_string(),
            line,
            why_it_matters: "because".to_string(),
            autofix_class: class,
            owner,
            confidence: conf,
            evidence: vec![format!("quoted code at {file}:{line}")],
            pre_existing,
            suggested_fix: None,
            requires_verification: false,
        }
    }

    fn env(reviewer: &str, findings: Vec<Finding>) -> ReviewerOutput {
        ReviewerOutput {
            reviewer: reviewer.to_string(),
            findings,
            residual_risks: vec![],
            testing_gaps: vec![],
        }
    }

    // ---- Stage 1: validation -----------------------------------------

    #[test]
    fn stage1_rejects_invalid_envelope() {
        let bad = ReviewerOutput {
            reviewer: "correctness".to_string(),
            findings: vec![fnd(
                "", // empty title
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.9,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
            residual_risks: vec![],
            testing_gaps: vec![],
        };
        let err = merge_findings(vec![bad]).unwrap_err();
        match err {
            MergeError::InvalidReviewerOutput { reviewer, .. } => {
                assert_eq!(reviewer, "correctness");
            }
        }
    }

    // ---- Stage 2: confidence gate ------------------------------------

    #[test]
    fn stage2_gate_drops_below_060() {
        let out = merge_findings(vec![env(
            "correctness",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                1,
                0.59,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        )])
        .unwrap();
        assert!(out.pr_introduced.is_empty());
    }

    #[test]
    fn stage2_gate_keeps_060_exact() {
        let out = merge_findings(vec![env(
            "correctness",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                1,
                0.60,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        )])
        .unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
    }

    #[test]
    fn stage2_gate_p0_exception_keeps_050() {
        let out = merge_findings(vec![env(
            "correctness",
            vec![fnd(
                "x",
                FindingSeverity::P0,
                "src/a.rs",
                1,
                0.50,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        )])
        .unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
    }

    #[test]
    fn stage2_gate_p0_below_050_dropped() {
        let out = merge_findings(vec![env(
            "correctness",
            vec![fnd(
                "x",
                FindingSeverity::P0,
                "src/a.rs",
                1,
                0.49,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        )])
        .unwrap();
        assert!(out.pr_introduced.is_empty());
    }

    #[test]
    fn stage2_gate_p1_at_059_dropped_even_though_high_sev() {
        // Exception only applies to P0.
        let out = merge_findings(vec![env(
            "correctness",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                1,
                0.55,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        )])
        .unwrap();
        assert!(out.pr_introduced.is_empty());
    }

    // ---- Stage 3: fingerprint dedup ----------------------------------

    #[test]
    fn stage3_same_issue_within_radius_merged() {
        let a = env(
            "correctness",
            vec![fnd(
                "Unwrap on parsed int",
                FindingSeverity::P1,
                "src/a.rs",
                42,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "adversarial",
            vec![fnd(
                "UNWRAP on parsed int",
                FindingSeverity::P1,
                "src/a.rs",
                44, // within ±3
                0.75,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
        let m = &out.pr_introduced[0];
        // Cross-reviewer boost: max(0.80, 0.75) + 0.10 = 0.90
        assert!((m.confidence - 0.90).abs() < 1e-5);
        // Evidence union (deduped)
        assert_eq!(m.evidence.len(), 2);
    }

    #[test]
    fn stage3_same_title_different_files_not_merged() {
        let a = env(
            "c",
            vec![fnd(
                "Unwrap on parsed int",
                FindingSeverity::P1,
                "src/a.rs",
                42,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "Unwrap on parsed int",
                FindingSeverity::P1,
                "src/b.rs",
                42,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced.len(), 2);
    }

    #[test]
    fn stage3_outside_radius_not_merged() {
        let a = env(
            "c",
            vec![fnd(
                "Unwrap on parsed int",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "Unwrap on parsed int",
                FindingSeverity::P1,
                "src/a.rs",
                14, // 4 > radius
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced.len(), 2);
    }

    #[test]
    fn stage3_windows_and_unix_paths_normalized() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src\\a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
    }

    // ---- Stage 4: cross-reviewer boost -------------------------------

    #[test]
    fn stage4_boost_two_reviewers_plus_010() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.70,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.70,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert!((out.pr_introduced[0].confidence - 0.80).abs() < 1e-5);
    }

    #[test]
    fn stage4_boost_capped_at_one() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.95,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.95,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert!((out.pr_introduced[0].confidence - 1.00).abs() < 1e-5);
    }

    #[test]
    fn stage4_no_boost_single_reviewer() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.70,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a]).unwrap();
        assert!((out.pr_introduced[0].confidence - 0.70).abs() < 1e-5);
    }

    // ---- Stage 5: pre_existing separation ----------------------------

    #[test]
    fn stage5_pre_existing_partitioned() {
        let a = env(
            "c",
            vec![
                fnd(
                    "new bug",
                    FindingSeverity::P1,
                    "src/a.rs",
                    10,
                    0.8,
                    Owner::ReviewFixer,
                    AutofixClass::SafeAuto,
                    false,
                ),
                fnd(
                    "old debt",
                    FindingSeverity::P2,
                    "src/b.rs",
                    20,
                    0.9,
                    Owner::Human,
                    AutofixClass::Advisory,
                    true,
                ),
                fnd(
                    "more debt",
                    FindingSeverity::P3,
                    "src/c.rs",
                    5,
                    0.8,
                    Owner::Human,
                    AutofixClass::Advisory,
                    true,
                ),
            ],
        );
        let out = merge_findings(vec![a]).unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
        assert_eq!(out.pr_introduced[0].title, "new bug");
        assert_eq!(out.pre_existing.len(), 2);
    }

    #[test]
    fn stage5_pre_existing_or_semantics_across_cluster() {
        // One reviewer flags as pre-existing, the other as new. We must
        // treat the merged finding as new (false-negative is worse).
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                true,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced.len(), 1);
        assert!(out.pre_existing.is_empty());
        assert!(!out.pr_introduced[0].pre_existing);
    }

    // ---- Stage 6: conservative routing -------------------------------

    #[test]
    fn stage6_owner_more_restrictive_wins_and_logs_diagnostic() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::Human,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced[0].owner, Owner::Human);
        assert!(matches!(
            out.diagnostics[0],
            MergeDiagnostic::OwnerDisagreement { .. }
        ));
    }

    #[test]
    fn stage6_autofix_class_safe_vs_manual_picks_manual() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::Manual,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert_eq!(out.pr_introduced[0].autofix_class, AutofixClass::Manual);
        assert!(matches!(
            out.diagnostics[0],
            MergeDiagnostic::AutofixClassDisagreement { .. }
        ));
    }

    #[test]
    fn stage6_agreement_no_diagnostic() {
        let a = env(
            "c",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let b = env(
            "t",
            vec![fnd(
                "x",
                FindingSeverity::P1,
                "src/a.rs",
                10,
                0.80,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )],
        );
        let out = merge_findings(vec![a, b]).unwrap();
        assert!(out.diagnostics.is_empty());
    }

    // ---- Stage 7: sorting --------------------------------------------

    #[test]
    fn stage7_sort_severity_desc_then_confidence_then_file_then_line() {
        let a = env(
            "c",
            vec![
                fnd(
                    "low",
                    FindingSeverity::P3,
                    "src/z.rs",
                    1,
                    0.9,
                    Owner::ReviewFixer,
                    AutofixClass::SafeAuto,
                    false,
                ),
                fnd(
                    "blocker",
                    FindingSeverity::P0,
                    "src/b.rs",
                    10,
                    0.9,
                    Owner::Human,
                    AutofixClass::Manual,
                    false,
                ),
                fnd(
                    "high1",
                    FindingSeverity::P1,
                    "src/a.rs",
                    5,
                    0.7,
                    Owner::ReviewFixer,
                    AutofixClass::SafeAuto,
                    false,
                ),
                fnd(
                    "high2",
                    FindingSeverity::P1,
                    "src/a.rs",
                    1,
                    0.9,
                    Owner::ReviewFixer,
                    AutofixClass::SafeAuto,
                    false,
                ),
            ],
        );
        let out = merge_findings(vec![a]).unwrap();
        let titles: Vec<_> = out.pr_introduced.iter().map(|f| f.title.clone()).collect();
        assert_eq!(titles, vec!["blocker", "high2", "high1", "low"]);
    }

    // ---- End-to-end 7-persona fixture --------------------------------

    #[test]
    fn end_to_end_seven_personas_realistic_mix() {
        // Shared-hit: correctness + adversarial agree on a P0 unwrap.
        let shared = |reviewer: &str| {
            env(
                reviewer,
                vec![fnd(
                    "Unwrap on parsed int can panic",
                    FindingSeverity::P0,
                    "src/worker.rs",
                    42,
                    0.80,
                    Owner::ReviewFixer,
                    AutofixClass::SafeAuto,
                    false,
                )],
            )
        };

        let correctness = shared("correctness");
        let adversarial = shared("adversarial");

        // Unique finding from testing.
        let testing = env(
            "testing",
            vec![fnd(
                "No test covers retry path",
                FindingSeverity::P2,
                "tests/worker_test.rs",
                1,
                0.85,
                Owner::DownstreamResolver,
                AutofixClass::Manual,
                false,
            )],
        );

        // Maintainability — low confidence, gets dropped.
        let maintainability = env(
            "maintainability",
            vec![fnd(
                "Function too long",
                FindingSeverity::P3,
                "src/worker.rs",
                1,
                0.55,
                Owner::Human,
                AutofixClass::Advisory,
                false,
            )],
        );

        // Project-standards — pre-existing rule drift.
        let project_standards = env(
            "project-standards",
            vec![fnd(
                "Missing doc comment",
                FindingSeverity::P3,
                "src/util.rs",
                7,
                0.9,
                Owner::DownstreamResolver,
                AutofixClass::Manual,
                true,
            )],
        );

        // Security — routing disagreement with correctness' owner via
        // a separate finding on a different line of the same file.
        let security = env(
            "security",
            vec![fnd(
                "Input passed to shell without escape",
                FindingSeverity::P0,
                "src/worker.rs",
                120,
                0.95,
                Owner::Human,
                AutofixClass::Manual,
                false,
            )],
        );

        // Performance — clean pass.
        let performance = env("performance", vec![]);

        let merged = merge_findings(vec![
            correctness,
            testing,
            maintainability,
            project_standards,
            security,
            performance,
            adversarial,
        ])
        .unwrap();

        // pr_introduced should have: shared unwrap (merged), testing gap,
        // security finding — 3 in total. Maintainability dropped by gate.
        let titles: Vec<_> = merged.pr_introduced.iter().map(|f| f.title.clone()).collect();
        assert_eq!(merged.pr_introduced.len(), 3, "titles were: {titles:?}");

        // Pre-existing: project-standards rule drift.
        assert_eq!(merged.pre_existing.len(), 1);
        assert_eq!(merged.pre_existing[0].title, "Missing doc comment");

        // Shared P0 finding got the agreement boost.
        let shared_hit = merged
            .pr_introduced
            .iter()
            .find(|f| f.title == "Unwrap on parsed int can panic")
            .unwrap();
        assert!((shared_hit.confidence - 0.90).abs() < 1e-5);
        assert_eq!(shared_hit.severity, FindingSeverity::P0);
        assert_eq!(shared_hit.evidence.len(), 1); // same evidence string — deduped

        // Sort order: P0 findings first.
        assert_eq!(merged.pr_introduced[0].severity, FindingSeverity::P0);
        assert_eq!(merged.pr_introduced[1].severity, FindingSeverity::P0);
        assert_eq!(merged.pr_introduced[2].severity, FindingSeverity::P2);

        // No routing disagreements (shared finding had identical owner/class).
        assert!(merged.diagnostics.is_empty());
    }

    #[test]
    fn empty_input_returns_empty() {
        let out = merge_findings(vec![]).unwrap();
        assert!(out.pr_introduced.is_empty());
        assert!(out.pre_existing.is_empty());
        assert!(out.diagnostics.is_empty());
    }

    #[test]
    fn passes_confidence_gate_unit() {
        let f = |sev, conf| {
            fnd(
                "x",
                sev,
                "a.rs",
                1,
                conf,
                Owner::ReviewFixer,
                AutofixClass::SafeAuto,
                false,
            )
        };
        assert!(passes_confidence_gate(&f(FindingSeverity::P1, 0.60)));
        assert!(!passes_confidence_gate(&f(FindingSeverity::P1, 0.59)));
        assert!(passes_confidence_gate(&f(FindingSeverity::P0, 0.50)));
        assert!(!passes_confidence_gate(&f(FindingSeverity::P0, 0.49)));
    }

    #[test]
    fn normalize_helpers() {
        assert_eq!(normalize_file("src\\A.RS"), "src/a.rs");
        assert_eq!(normalize_title("  Unwrap  ON   X "), "unwrap on x");
    }
}
