//! Review-to-task flow for the cas-code-review orchestrator
//! (Phase 1 Subsystem A, Unit 8 — task cas-c654).
//!
//! This module takes the residual PR-introduced findings that survive
//! the Unit 7 autofix loop and routes them into CAS tasks via the
//! existing task-store surface. It owns three responsibilities:
//!
//! 1. **Field mapping.** Finding → [`TaskDraft`] with the title,
//!    description, priority, task type, labels, and `external_ref`
//!    fields the caller will pass to a task store.
//! 2. **Advisory exclusion.** Findings with
//!    [`AutofixClass::Advisory`] never become tasks, full stop.
//! 3. **Idempotency.** Re-running the flow on the same unchanged
//!    findings updates existing tasks instead of creating duplicates,
//!    keyed by a stable `external_ref` hash.
//!
//! Following the autofix.rs convention, persistence is injected as
//! closures rather than going through the [`crate::TaskStore`] trait
//! directly. This keeps the module pure-Rust, unit-testable, and
//! independent of SQLite or MCP plumbing. The production call site
//! (Unit 9, `close_ops` integration) wires the closures to a concrete
//! task store.
//!
//! # Scope
//!
//! This module does **not**:
//! - call `mcp__cas__task` directly (the caller in Unit 9 does)
//! - touch `close_ops` (Unit 9)
//! - add new fields to the task schema (uses existing columns)
//! - create tasks for `advisory` findings, ever

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use cas_types::{AutofixClass, Finding, FindingSeverity, Priority, TaskType};

/// Label attached to every task created by this flow. Lets downstream
/// tooling (search, filters, cleanup jobs) identify review-routed
/// tasks without parsing descriptions.
pub const REVIEW_LABEL: &str = "code-review";

/// Description footer appended to every routed task so a human
/// reader can trace the task back to its source.
pub const SOURCE_FOOTER: &str = "Source: cas-code-review";

/// Builder for the subset of task fields this flow controls. The
/// caller is responsible for mapping this into whatever concrete
/// task type its store requires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskDraft {
    /// Already ≤100 chars (enforced by [`Finding`] validation).
    pub title: String,
    pub description: String,
    pub priority: Priority,
    pub task_type: TaskType,
    pub labels: Vec<String>,
    /// Stable identifier used to dedup across reruns. See
    /// [`external_ref_for`] for the exact input.
    pub external_ref: String,
}

/// What happened to a single residual finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteAction {
    /// A new task was created. String is the new task ID.
    Created(String),
    /// An existing task was found via `external_ref` and updated.
    Updated(String),
    /// The finding was not routed.
    Skipped { reason: SkipReason },
}

/// Why a finding was not routed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SkipReason {
    /// `autofix_class == advisory` — advisories never become tasks.
    Advisory,
}

/// Errors the routing closures can return.
#[derive(Debug, thiserror::Error)]
pub enum RouteError {
    #[error("create failed for external_ref {external_ref}: {source}")]
    CreateFailed {
        external_ref: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
    #[error("update failed for task {task_id}: {source}")]
    UpdateFailed {
        task_id: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

/// Aggregated outcome of a routing run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteOutcome {
    /// One entry per input finding, in input order.
    pub actions: Vec<RouteAction>,
    /// The drafts built for each *non-advisory* finding, in input
    /// order. Advisories do not appear here.
    pub drafts: Vec<TaskDraft>,
}

impl RouteOutcome {
    /// IDs of tasks created by this run.
    pub fn created_ids(&self) -> Vec<&str> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                RouteAction::Created(id) => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// IDs of tasks updated by this run.
    pub fn updated_ids(&self) -> Vec<&str> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                RouteAction::Updated(id) => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Combined created + updated IDs, in the order they appear.
    pub fn routed_ids(&self) -> Vec<&str> {
        self.actions
            .iter()
            .filter_map(|a| match a {
                RouteAction::Created(id) | RouteAction::Updated(id) => Some(id.as_str()),
                _ => None,
            })
            .collect()
    }
}

/// Map a finding severity onto a CAS task priority.
///
/// The mapping is a 1:1 identity on the numeric level: P0→0 (Critical),
/// P1→1 (High), P2→2 (Medium), P3→3 (Low). CAS also defines a
/// `BACKLOG`/4 tier which this flow never produces, because persona
/// calibration caps low-confidence findings at P3.
pub fn map_severity(sev: FindingSeverity) -> Priority {
    match sev {
        FindingSeverity::P0 => Priority::CRITICAL,
        FindingSeverity::P1 => Priority::HIGH,
        FindingSeverity::P2 => Priority::MEDIUM,
        FindingSeverity::P3 => Priority::LOW,
    }
}

/// Heuristic mapping from a finding's autofix class to the CAS task
/// type it should be filed as.
///
/// - `manual` → `Task` (human-handled, not necessarily a bug)
/// - `gated_auto` → `Bug` (auto-fixable under a gate implies a
///   concrete defect rather than a design discussion)
/// - `safe_auto` → `Task` (should not reach routing in practice — the
///   autofix loop applies these; if it reaches here the loop bailed on
///   it, treat as a generic follow-up)
/// - `advisory` → **not routed** (this function never returns a value
///   for advisories; callers must filter them out first)
pub fn map_task_type(class: AutofixClass) -> Option<TaskType> {
    match class {
        AutofixClass::Advisory => None,
        AutofixClass::GatedAuto => Some(TaskType::Bug),
        AutofixClass::Manual | AutofixClass::SafeAuto => Some(TaskType::Task),
    }
}

/// Return just the final path component of a `Finding.file`, using
/// both `/` and `\` as separators so Windows-style paths work too.
/// Falls back to the full string if no separator is found.
fn file_basename(file: &str) -> &str {
    file.rsplit(|c| c == '/' || c == '\\')
        .next()
        .unwrap_or(file)
}

/// Normalize a finding title for hashing — lowercased, collapsed
/// whitespace, stripped of trailing punctuation. Keeps the hash
/// stable across cosmetic rewording.
fn normalize_title(title: &str) -> String {
    let collapsed: String = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();
    collapsed
        .trim_end_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

/// Stable identifier for a routed finding.
///
/// The hash input is deliberately `(file, normalized_title,
/// why_it_matters_prefix)` — it **excludes** `line` because line
/// numbers drift across edits and we want the same defect to land on
/// the same task across reruns. The `why_it_matters` prefix (first 80
/// chars) pins the semantic content so two different findings on the
/// same file with similar titles do not collide.
///
/// The output format is `code-review:<16-hex>` so it is recognizable
/// at a glance in `tasks.external_ref` queries and cannot collide
/// with external-ref values from other subsystems.
pub fn external_ref_for(finding: &Finding) -> String {
    let mut hasher = DefaultHasher::new();
    finding.file.hash(&mut hasher);
    normalize_title(&finding.title).hash(&mut hasher);
    let why_prefix: String = finding.why_it_matters.chars().take(80).collect();
    why_prefix.hash(&mut hasher);
    format!("code-review:{:016x}", hasher.finish())
}

fn build_labels(finding: &Finding) -> Vec<String> {
    vec![
        REVIEW_LABEL.to_string(),
        format!("severity:{}", finding.severity),
        format!("file:{}", file_basename(&finding.file)),
    ]
}

fn build_description(finding: &Finding) -> String {
    let mut out = String::new();
    out.push_str("## Why it matters\n\n");
    out.push_str(finding.why_it_matters.trim());
    out.push_str("\n\n## Evidence\n");
    for e in &finding.evidence {
        out.push_str("\n- ");
        out.push_str(e.trim());
    }
    if let Some(fix) = finding.suggested_fix.as_deref() {
        let fix = fix.trim();
        if !fix.is_empty() {
            out.push_str("\n\n## Suggested fix\n\n");
            out.push_str(fix);
        }
    }
    out.push_str("\n\n## Location\n\n`");
    out.push_str(&finding.file);
    out.push_str(":");
    out.push_str(&finding.line.to_string());
    out.push_str("`\n\n---\n");
    out.push_str(SOURCE_FOOTER);
    out
}

/// Build a [`TaskDraft`] from a single finding.
///
/// Returns `None` for advisory findings, which are never routed.
pub fn build_draft(finding: &Finding) -> Option<TaskDraft> {
    let task_type = map_task_type(finding.autofix_class)?;
    Some(TaskDraft {
        title: finding.title.clone(),
        description: build_description(finding),
        priority: map_severity(finding.severity),
        task_type,
        labels: build_labels(finding),
        external_ref: external_ref_for(finding),
    })
}

/// Route the residual findings from an autofix run into CAS tasks.
///
/// For each non-advisory finding:
///
/// 1. Build a [`TaskDraft`].
/// 2. Call `find_by_ref(&draft.external_ref)`. If it returns
///    `Some(task_id)`, dispatch to `update(task_id, &draft)` and
///    record [`RouteAction::Updated`].
/// 3. Otherwise dispatch to `create(&draft)` and record
///    [`RouteAction::Created`] with the new task ID.
///
/// Advisory findings are recorded as
/// [`RouteAction::Skipped`]`{ reason: Advisory }` and their drafts
/// are **not** added to [`RouteOutcome::drafts`].
///
/// The closure-based signature mirrors [`super::autofix_loop`] so
/// unit tests can pin behavior with deterministic stubs and the
/// production integration (Unit 9) can wire real task-store calls
/// without this module depending on SQLite.
pub fn route_residual_to_tasks<F, C, U>(
    residual: &[Finding],
    mut find_by_ref: F,
    mut create: C,
    mut update: U,
) -> Result<RouteOutcome, RouteError>
where
    F: FnMut(&str) -> Option<String>,
    C: FnMut(&TaskDraft) -> Result<String, Box<dyn std::error::Error + Send + Sync>>,
    U: FnMut(&str, &TaskDraft) -> Result<(), Box<dyn std::error::Error + Send + Sync>>,
{
    let mut outcome = RouteOutcome::default();

    for finding in residual {
        if matches!(finding.autofix_class, AutofixClass::Advisory) {
            outcome.actions.push(RouteAction::Skipped {
                reason: SkipReason::Advisory,
            });
            continue;
        }

        // Safe to unwrap: we already filtered out advisories above.
        let draft = build_draft(finding).expect("non-advisory findings always build a draft");

        let action = if let Some(task_id) = find_by_ref(&draft.external_ref) {
            update(&task_id, &draft).map_err(|source| RouteError::UpdateFailed {
                task_id: task_id.clone(),
                source,
            })?;
            RouteAction::Updated(task_id)
        } else {
            let new_id = create(&draft).map_err(|source| RouteError::CreateFailed {
                external_ref: draft.external_ref.clone(),
                source,
            })?;
            RouteAction::Created(new_id)
        };

        outcome.actions.push(action);
        outcome.drafts.push(draft);
    }

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_types::Owner;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn mk(title: &str, sev: FindingSeverity, class: AutofixClass) -> Finding {
        Finding {
            title: title.to_string(),
            severity: sev,
            file: "cas-cli/src/foo/bar.rs".to_string(),
            line: 42,
            why_it_matters: "this breaks the thing because reasons".to_string(),
            autofix_class: class,
            owner: Owner::ReviewFixer,
            confidence: 0.9,
            evidence: vec!["let x = foo.unwrap();".to_string()],
            pre_existing: false,
            suggested_fix: Some("match foo { Some(v) => v, None => return }".to_string()),
            requires_verification: false,
        }
    }

    // --- mapping matrices ---

    #[test]
    fn severity_to_priority_matrix() {
        assert_eq!(map_severity(FindingSeverity::P0), Priority::CRITICAL);
        assert_eq!(map_severity(FindingSeverity::P1), Priority::HIGH);
        assert_eq!(map_severity(FindingSeverity::P2), Priority::MEDIUM);
        assert_eq!(map_severity(FindingSeverity::P3), Priority::LOW);
        // Spec: P0→0, P1→1, P2→2, P3→3.
        assert_eq!(map_severity(FindingSeverity::P0).0, 0);
        assert_eq!(map_severity(FindingSeverity::P1).0, 1);
        assert_eq!(map_severity(FindingSeverity::P2).0, 2);
        assert_eq!(map_severity(FindingSeverity::P3).0, 3);
    }

    #[test]
    fn autofix_class_to_task_type_matrix() {
        assert_eq!(map_task_type(AutofixClass::Manual), Some(TaskType::Task));
        assert_eq!(map_task_type(AutofixClass::GatedAuto), Some(TaskType::Bug));
        assert_eq!(map_task_type(AutofixClass::SafeAuto), Some(TaskType::Task));
        assert_eq!(map_task_type(AutofixClass::Advisory), None);
    }

    // --- draft building ---

    #[test]
    fn build_draft_emits_expected_fields() {
        let f = mk("Dropped Result", FindingSeverity::P1, AutofixClass::Manual);
        let draft = build_draft(&f).expect("manual finding routes");

        assert_eq!(draft.title, "Dropped Result");
        assert_eq!(draft.priority, Priority::HIGH);
        assert_eq!(draft.task_type, TaskType::Task);
        assert!(draft.labels.contains(&REVIEW_LABEL.to_string()));
        assert!(draft.labels.contains(&"severity:P1".to_string()));
        assert!(draft.labels.contains(&"file:bar.rs".to_string()));
        // description contains the prose + evidence + suggested_fix + footer
        assert!(draft.description.contains("this breaks the thing"));
        assert!(draft.description.contains("let x = foo.unwrap();"));
        assert!(draft.description.contains("match foo"));
        assert!(draft.description.contains(SOURCE_FOOTER));
        assert!(draft.description.contains("cas-cli/src/foo/bar.rs:42"));
        assert!(draft.external_ref.starts_with("code-review:"));
    }

    #[test]
    fn build_draft_returns_none_for_advisory() {
        let f = mk("FYI note", FindingSeverity::P3, AutofixClass::Advisory);
        assert!(build_draft(&f).is_none());
    }

    #[test]
    fn labels_use_basename_not_full_path() {
        let mut f = mk("X", FindingSeverity::P2, AutofixClass::Manual);
        f.file = "cas-cli/src/deep/nested/handler.rs".to_string();
        let draft = build_draft(&f).unwrap();
        assert!(draft.labels.iter().any(|l| l == "file:handler.rs"));
        assert!(!draft.labels.iter().any(|l| l.contains("nested")));
    }

    #[test]
    fn labels_handle_windows_path_separators() {
        let mut f = mk("X", FindingSeverity::P2, AutofixClass::Manual);
        f.file = "cas-cli\\src\\handler.rs".to_string();
        let draft = build_draft(&f).unwrap();
        assert!(draft.labels.iter().any(|l| l == "file:handler.rs"));
    }

    // --- external_ref stability ---

    #[test]
    fn external_ref_stable_across_line_drift() {
        let mut a = mk("SQL injection", FindingSeverity::P0, AutofixClass::Manual);
        let mut b = a.clone();
        a.line = 42;
        b.line = 107;
        assert_eq!(
            external_ref_for(&a),
            external_ref_for(&b),
            "line number must NOT contribute to the hash"
        );
    }

    #[test]
    fn external_ref_stable_across_title_whitespace_and_case() {
        let mut a = mk("SQL injection in handler", FindingSeverity::P0, AutofixClass::Manual);
        let mut b = a.clone();
        b.title = "  sql   INJECTION  in  handler.  ".to_string();
        assert_eq!(external_ref_for(&a), external_ref_for(&b));
        let _ = (&mut a, &mut b);
    }

    #[test]
    fn external_ref_differs_across_files() {
        let mut a = mk("X", FindingSeverity::P2, AutofixClass::Manual);
        let mut b = a.clone();
        a.file = "src/a.rs".to_string();
        b.file = "src/b.rs".to_string();
        assert_ne!(external_ref_for(&a), external_ref_for(&b));
    }

    #[test]
    fn external_ref_differs_across_semantically_distinct_findings() {
        let a = mk("Null deref", FindingSeverity::P0, AutofixClass::Manual);
        let mut b = a.clone();
        b.why_it_matters = "a completely different root cause explanation".to_string();
        assert_ne!(external_ref_for(&a), external_ref_for(&b));
    }

    // --- routing behavior ---

    #[test]
    fn advisory_findings_are_skipped_and_absent_from_drafts() {
        let residual = vec![
            mk("real bug", FindingSeverity::P1, AutofixClass::Manual),
            mk("fyi", FindingSeverity::P3, AutofixClass::Advisory),
        ];

        let created: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let created_sink = created.clone();
        let outcome = route_residual_to_tasks(
            &residual,
            |_ref| None,
            move |d| {
                created_sink.borrow_mut().push(d.external_ref.clone());
                Ok(format!("cas-new{}", created_sink.borrow().len()))
            },
            |_id, _d| unreachable!("no updates expected"),
        )
        .unwrap();

        assert_eq!(outcome.actions.len(), 2);
        assert!(matches!(outcome.actions[0], RouteAction::Created(_)));
        assert!(matches!(
            outcome.actions[1],
            RouteAction::Skipped {
                reason: SkipReason::Advisory
            }
        ));
        assert_eq!(outcome.drafts.len(), 1, "advisory drafts must be excluded");
        assert_eq!(outcome.created_ids().len(), 1);
        assert_eq!(created.borrow().len(), 1);
    }

    #[test]
    fn idempotent_rerun_updates_existing_tasks() {
        let residual = vec![mk("a bug", FindingSeverity::P1, AutofixClass::Manual)];

        // First run — finds nothing, creates task cas-aaaa.
        let outcome1 = route_residual_to_tasks(
            &residual,
            |_ref| None,
            |_d| Ok("cas-aaaa".to_string()),
            |_id, _d| unreachable!(),
        )
        .unwrap();
        assert_eq!(outcome1.created_ids(), vec!["cas-aaaa"]);
        assert!(outcome1.updated_ids().is_empty());

        // Second run — find_by_ref returns the existing task ID.
        let update_count = Rc::new(RefCell::new(0usize));
        let uc = update_count.clone();
        let outcome2 = route_residual_to_tasks(
            &residual,
            |_ref| Some("cas-aaaa".to_string()),
            |_d| unreachable!("must not create when existing task is present"),
            move |id, _d| {
                assert_eq!(id, "cas-aaaa");
                *uc.borrow_mut() += 1;
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(outcome2.updated_ids(), vec!["cas-aaaa"]);
        assert!(outcome2.created_ids().is_empty());
        assert_eq!(*update_count.borrow(), 1);
    }

    #[test]
    fn create_error_surfaces_as_route_error() {
        let residual = vec![mk("a bug", FindingSeverity::P1, AutofixClass::Manual)];
        let err = route_residual_to_tasks(
            &residual,
            |_ref| None,
            |_d| Err("boom".into()),
            |_id, _d| unreachable!(),
        )
        .unwrap_err();
        match err {
            RouteError::CreateFailed { external_ref, .. } => {
                assert!(external_ref.starts_with("code-review:"));
            }
            other => panic!("expected CreateFailed, got {other:?}"),
        }
    }

    #[test]
    fn update_error_surfaces_as_route_error() {
        let residual = vec![mk("a bug", FindingSeverity::P1, AutofixClass::Manual)];
        let err = route_residual_to_tasks(
            &residual,
            |_ref| Some("cas-aaaa".to_string()),
            |_d| unreachable!(),
            |_id, _d| Err("kaboom".into()),
        )
        .unwrap_err();
        match err {
            RouteError::UpdateFailed { task_id, .. } => assert_eq!(task_id, "cas-aaaa"),
            other => panic!("expected UpdateFailed, got {other:?}"),
        }
    }

    #[test]
    fn mixed_residual_with_all_classes_routes_correctly() {
        let residual = vec![
            mk("safe", FindingSeverity::P2, AutofixClass::SafeAuto),   // Task
            mk("gated", FindingSeverity::P1, AutofixClass::GatedAuto), // Bug
            mk("manual", FindingSeverity::P0, AutofixClass::Manual),   // Task
            mk("advisory", FindingSeverity::P3, AutofixClass::Advisory), // Skipped
        ];

        let counter = Rc::new(RefCell::new(0usize));
        let c = counter.clone();
        let outcome = route_residual_to_tasks(
            &residual,
            |_ref| None,
            move |_d| {
                *c.borrow_mut() += 1;
                Ok(format!("cas-{}", c.borrow()))
            },
            |_id, _d| unreachable!(),
        )
        .unwrap();

        assert_eq!(outcome.drafts.len(), 3);
        assert_eq!(outcome.drafts[0].task_type, TaskType::Task); // safe_auto → Task
        assert_eq!(outcome.drafts[0].priority, Priority::MEDIUM);
        assert_eq!(outcome.drafts[1].task_type, TaskType::Bug); // gated_auto → Bug
        assert_eq!(outcome.drafts[1].priority, Priority::HIGH);
        assert_eq!(outcome.drafts[2].task_type, TaskType::Task); // manual → Task
        assert_eq!(outcome.drafts[2].priority, Priority::CRITICAL);
        assert_eq!(outcome.created_ids().len(), 3);
        assert!(matches!(
            outcome.actions[3],
            RouteAction::Skipped {
                reason: SkipReason::Advisory
            }
        ));
    }

    #[test]
    fn empty_residual_is_ok_and_idle() {
        let outcome = route_residual_to_tasks(
            &[],
            |_r| None,
            |_d| unreachable!(),
            |_id, _d| unreachable!(),
        )
        .unwrap();
        assert!(outcome.actions.is_empty());
        assert!(outcome.drafts.is_empty());
    }

    #[test]
    fn labels_always_include_review_marker_and_severity() {
        for sev in [
            FindingSeverity::P0,
            FindingSeverity::P1,
            FindingSeverity::P2,
            FindingSeverity::P3,
        ] {
            let f = mk("x", sev, AutofixClass::Manual);
            let draft = build_draft(&f).unwrap();
            assert!(draft.labels.contains(&REVIEW_LABEL.to_string()));
            assert!(draft.labels.iter().any(|l| l == &format!("severity:{sev}")));
        }
    }
}
