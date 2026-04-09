//! P0 close-gate decision logic for cas-code-review
//! (Phase 1 Subsystem A, Unit 9 — task cas-b39f).
//!
//! This module owns the *policy* of the close gate: given the residual
//! findings from an autofix round (Unit 7), decide whether `task.close`
//! should proceed or be blocked. The *integration* with `close_ops.rs`
//! and the *dispatch* of the cas-code-review skill itself live at the
//! call site. Keeping the decision here makes it pure, unit-testable,
//! and free of MCP surface dependencies.
//!
//! # Semantics
//!
//! Per R9, the gate is strict on P0 and permissive on everything else:
//!
//! - Any residual `pr_introduced` finding with `severity == P0` →
//!   [`GateDecision::BlockOnP0`], carrying the full list of P0 findings
//!   so the caller can format a useful error.
//! - Residual findings of P1/P2/P3 are **not** blocking. They are
//!   surfaced through Unit 8 (review-to-task) as follow-up tasks
//!   instead, and the close is allowed to proceed. This is intentional:
//!   the gate's job is to catch "this change must not land as-is", not
//!   to serialize all review work before every close.
//! - Advisory findings never block (already filtered upstream, but we
//!   defend in depth).
//! - An empty residual (or a residual where every P0 has already been
//!   marked `pre_existing`) → [`GateDecision::Allow`].
//!
//! The caller combines this with a few higher-level skip conditions
//! (additive-only tasks, no-code-files diffs, supervisor override,
//! skill dispatch unavailable) before calling `evaluate_gate` — those
//! live in `close_ops.rs` because they need to see the `Task` record
//! and the harness env, neither of which belong in `cas-store`.

use cas_types::{Finding, FindingSeverity};

/// Decision returned by [`evaluate_gate`].
#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    /// Close may proceed. Either the residual was empty, or every
    /// blocking finding was filtered (pre-existing / advisory) or
    /// non-blocking (P1..P3).
    Allow,
    /// Close is blocked by one or more P0 findings. The caller should
    /// format a `tool_error` listing these and return to the worker so
    /// the worker can either fix them or request a supervisor override.
    BlockOnP0(Vec<Finding>),
}

/// Evaluate the close gate against a residual finding set.
///
/// `residual` is the PR-introduced residual list coming out of the
/// autofix loop (Unit 7) — that is, anything the fixer sub-agent did
/// not resolve within the bounded 2-round budget. Pre-existing findings
/// must already be partitioned out by the caller (via the Unit 5 merge
/// pipeline), but we still defensively skip any `pre_existing: true`
/// entries that slip through, since "block the close on pre-existing
/// debt the author did not introduce" would be the worst possible user
/// experience.
pub fn evaluate_gate(residual: &[Finding]) -> GateDecision {
    let p0: Vec<Finding> = residual
        .iter()
        .filter(|f| !f.pre_existing && f.severity == FindingSeverity::P0)
        .cloned()
        .collect();

    if p0.is_empty() {
        GateDecision::Allow
    } else {
        GateDecision::BlockOnP0(p0)
    }
}

/// Format a P0 block message for `close_ops.rs` to return as a
/// `tool_error`. Kept here so the wording lives next to the policy it
/// explains.
pub fn format_block_message(task_id: &str, blocking: &[Finding]) -> String {
    let mut out = format!(
        "⚠️ CODE REVIEW P0 BLOCK\n\n\
        Task {task_id} close rejected — cas-code-review found {count} \
        P0-severity finding(s) the worker must resolve before closing.\n\n\
        P0 findings:\n",
        count = blocking.len()
    );
    for (i, f) in blocking.iter().enumerate() {
        out.push_str(&format!(
            "\n  {n}. [{file}:{line}] {title}\n     Why: {why}\n",
            n = i + 1,
            file = f.file,
            line = f.line,
            title = f.title,
            why = f.why_it_matters,
        ));
        if !f.evidence.is_empty() {
            out.push_str(&format!("     Evidence: {}\n", f.evidence[0]));
        }
    }
    out.push_str(
        "\nTo resolve: either fix the findings and retry close, or \
         request a supervisor override by calling close with \
         bypass_code_review=true (supervisors only — see R9).",
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_types::{AutofixClass, Owner};

    fn mk(severity: FindingSeverity, pre_existing: bool) -> Finding {
        Finding {
            title: "something".to_string(),
            severity,
            file: "src/lib.rs".to_string(),
            line: 10,
            why_it_matters: "matters a lot".to_string(),
            autofix_class: AutofixClass::Manual,
            owner: Owner::Human,
            confidence: 0.9,
            evidence: vec!["let x = unwrap();".to_string()],
            pre_existing,
            suggested_fix: None,
            requires_verification: false,
        }
    }

    #[test]
    fn empty_residual_allows() {
        assert_eq!(evaluate_gate(&[]), GateDecision::Allow);
    }

    #[test]
    fn p1_residual_does_not_block() {
        let r = vec![mk(FindingSeverity::P1, false), mk(FindingSeverity::P2, false)];
        assert_eq!(evaluate_gate(&r), GateDecision::Allow);
    }

    #[test]
    fn single_p0_blocks() {
        let r = vec![mk(FindingSeverity::P0, false)];
        match evaluate_gate(&r) {
            GateDecision::BlockOnP0(blocking) => assert_eq!(blocking.len(), 1),
            other => panic!("expected BlockOnP0, got {other:?}"),
        }
    }

    #[test]
    fn multiple_p0_all_reported() {
        let r = vec![
            mk(FindingSeverity::P0, false),
            mk(FindingSeverity::P1, false),
            mk(FindingSeverity::P0, false),
            mk(FindingSeverity::P2, false),
        ];
        match evaluate_gate(&r) {
            GateDecision::BlockOnP0(blocking) => assert_eq!(blocking.len(), 2),
            other => panic!("expected BlockOnP0, got {other:?}"),
        }
    }

    #[test]
    fn pre_existing_p0_does_not_block() {
        // Defense in depth: if a pre_existing=true P0 sneaks through
        // the upstream partition, the gate still ignores it. Blocking
        // a close on debt the author did not introduce would be
        // maximally frustrating.
        let r = vec![mk(FindingSeverity::P0, true)];
        assert_eq!(evaluate_gate(&r), GateDecision::Allow);
    }

    #[test]
    fn mixed_pre_existing_and_new_p0_blocks_only_on_new() {
        let r = vec![
            mk(FindingSeverity::P0, true), // pre-existing — ignored
            mk(FindingSeverity::P0, false),
        ];
        match evaluate_gate(&r) {
            GateDecision::BlockOnP0(blocking) => {
                assert_eq!(blocking.len(), 1);
                assert!(!blocking[0].pre_existing);
            }
            other => panic!("expected BlockOnP0, got {other:?}"),
        }
    }

    #[test]
    fn format_block_message_mentions_task_and_findings() {
        let blocking = vec![{
            let mut f = mk(FindingSeverity::P0, false);
            f.title = "SQL injection in login()".to_string();
            f.file = "src/auth.rs".to_string();
            f.line = 42;
            f.why_it_matters = "Attacker can bypass auth".to_string();
            f
        }];
        let msg = format_block_message("cas-abcd", &blocking);
        assert!(msg.contains("cas-abcd"));
        assert!(msg.contains("SQL injection"));
        assert!(msg.contains("src/auth.rs:42"));
        assert!(msg.contains("Attacker can bypass auth"));
        assert!(msg.contains("bypass_code_review=true"));
    }
}
