//! Bounded autofix loop for the cas-code-review orchestrator
//! (Phase 1 Subsystem A, Unit 7 — task cas-56a2).
//!
//! This module owns the *control flow* of the fix-and-rereview loop.
//! The actual fixer sub-agent lives in
//! `cas-cli/src/builtins/skills/cas-code-review/references/fixer.md`
//! and is dispatched via the Task tool by the orchestrator skill body
//! (Unit 4). The rereview step is a fresh invocation of the orchestrator
//! on the patched tree, which eventually returns a new [`MergedFindings`]
//! through the Unit 5 merge pipeline.
//!
//! Because neither of those side effects can happen from pure Rust,
//! [`autofix_loop`] accepts both as injected closures. The production
//! call site wires them to real Task-tool and orchestrator invocations;
//! the tests wire them to deterministic fixtures. The *loop semantics*
//! — filter safe_auto, dispatch, rereview, exit conditions, hard cap —
//! live here and are fully unit-testable.
//!
//! # Semantics
//!
//! Given a starting [`MergedFindings`] and a fixer/rereviewer pair:
//!
//! 1. **Round 1.** Filter `pr_introduced` to [`AutofixClass::SafeAuto`].
//!    If the filtered set is empty, the loop exits immediately with the
//!    initial state as the residual — there is nothing for the fixer to
//!    do, so zero rounds run.
//! 2. Otherwise dispatch the fixer with the safe_auto set.
//! 3. If the fixer crashed (`FixerResult::crashed == true`) OR applied
//!    zero findings, treat the round as zero progress and exit *without*
//!    burning the second round. Zero progress is a signal that a retry
//!    in the same session will not help.
//! 4. If the fixer applied something, invoke the rereviewer to re-run
//!    the orchestrator on the modified tree. The result becomes the new
//!    current state for round 2.
//! 5. **Round 2.** Same as round 1. After round 2 the loop terminates
//!    unconditionally — no round 3, ever, regardless of what round 2
//!    introduced. That hard cap is the invariant this module exists to
//!    guarantee.
//!
//! The caller receives an [`AutofixOutcome`] describing how many rounds
//! ran, which findings were applied, what the residual is, and *why* the
//! loop exited. The residual is always `current.pr_introduced` at exit —
//! anything still on that list at the end is by definition unresolved
//! and belongs on the caller's downstream-routing track (Unit 8).
//!
//! # Scope
//!
//! This module does **not**:
//! - route residual findings to CAS tasks (Unit 8)
//! - integrate with `close_ops` (Unit 9)
//! - escalate fixer permissions beyond whatever the caller already has

use cas_types::{AutofixClass, Finding};

use super::merge::{MergeError, MergedFindings};

/// Hard upper bound on the number of fix-and-rereview rounds. Per R10
/// this is not configurable in Phase 1 — if workers learn they can
/// extend it, they will, and the whole point of the bound is to keep
/// review latency predictable.
pub const MAX_ROUNDS: u8 = 2;

/// Structured result from a single fixer sub-agent invocation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FixerResult {
    /// Findings the fixer reports it successfully applied. The orchestrator
    /// does not verify these independently — the rereview step is the
    /// ground truth.
    pub applied: Vec<Finding>,
    /// Findings the fixer chose to skip (out-of-scope, ambiguous
    /// `suggested_fix`, etc.), paired with the reason the fixer gave.
    pub skipped: Vec<(Finding, String)>,
    /// True if the fixer sub-agent crashed, returned malformed output,
    /// or otherwise failed to produce a usable result for this round.
    pub crashed: bool,
}

impl FixerResult {
    /// Convenience: did this round produce any progress worth a
    /// rereview? Zero progress short-circuits the loop.
    pub fn made_progress(&self) -> bool {
        !self.crashed && !self.applied.is_empty()
    }
}

/// Why the autofix loop terminated. Useful for telemetry + debugging.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitReason {
    /// No safe_auto findings were present at the start of a round, so
    /// there was nothing for the fixer to do.
    NothingToFix,
    /// The fixer returned zero applied findings (or crashed), so the
    /// loop exited early without burning the next round.
    ZeroProgress,
    /// The rereview step returned a merge error. The partial outcome is
    /// still returned so the caller can decide what to do, but no more
    /// rounds run.
    RereviewFailed(String),
    /// The loop hit the [`MAX_ROUNDS`] cap.
    HardCap,
}

/// Result of running [`autofix_loop`].
#[derive(Debug, Clone, PartialEq)]
pub struct AutofixOutcome {
    /// How many fix-and-rereview rounds actually executed. Always
    /// `0..=MAX_ROUNDS`. Zero means the loop exited before the first
    /// fixer dispatch (nothing to fix).
    pub rounds_run: u8,
    /// Cumulative list of findings the fixer reported applied across
    /// all rounds. This is what the fixer *claims*; the ground truth is
    /// `residual` (anything still present after the final rereview).
    pub applied_total: Vec<Finding>,
    /// PR-introduced findings still present after the loop exits. These
    /// are what Unit 8 routes into CAS tasks. Non-safe_auto findings
    /// surface here on the first round; safe_auto findings surface here
    /// only if the fixer skipped them or if round 2 hit the hard cap.
    pub residual: Vec<Finding>,
    /// Pre-existing findings from the *final* merge state. Pass-through
    /// from the rereviewer — autofix never touches pre-existing.
    pub pre_existing: Vec<Finding>,
    /// Why the loop exited. See [`ExitReason`].
    pub exit_reason: ExitReason,
}

/// Run the bounded autofix loop.
///
/// `initial` is the merged output from the first review pass. `fixer`
/// is invoked with the safe_auto subset of `pr_introduced` each round
/// and must return a [`FixerResult`]. `rereviewer` is invoked *after* a
/// productive round to refresh the merged state; it must re-run the
/// orchestrator + merge pipeline on the patched tree.
///
/// The loop is guaranteed to call `fixer` at most [`MAX_ROUNDS`] times
/// and `rereviewer` at most [`MAX_ROUNDS`] times. The hard cap is the
/// load-bearing invariant of this module; tests pin it explicitly.
pub fn autofix_loop<F, R>(
    initial: MergedFindings,
    mut fixer: F,
    mut rereviewer: R,
) -> AutofixOutcome
where
    F: FnMut(&[Finding]) -> FixerResult,
    R: FnMut() -> Result<MergedFindings, MergeError>,
{
    let mut current = initial;
    let mut applied_total: Vec<Finding> = Vec::new();
    let mut rounds_run: u8 = 0;

    loop {
        // Filter to safe_auto — the only class the fixer is allowed to
        // touch per R10 + the fixer prompt mandate. Check this *before*
        // the hard-cap gate so a clean finish after round 2 reports
        // NothingToFix rather than HardCap (HardCap should only fire
        // when there is still productive work left but no round budget).
        let safe_auto: Vec<Finding> = current
            .pr_introduced
            .iter()
            .filter(|f| matches!(f.autofix_class, AutofixClass::SafeAuto))
            .cloned()
            .collect();

        if safe_auto.is_empty() {
            return AutofixOutcome {
                rounds_run,
                applied_total,
                residual: current.pr_introduced,
                pre_existing: current.pre_existing,
                exit_reason: ExitReason::NothingToFix,
            };
        }

        if rounds_run >= MAX_ROUNDS {
            return AutofixOutcome {
                rounds_run,
                applied_total,
                residual: current.pr_introduced,
                pre_existing: current.pre_existing,
                exit_reason: ExitReason::HardCap,
            };
        }

        let result = fixer(&safe_auto);
        rounds_run += 1;

        if !result.made_progress() {
            // Zero progress — fixer crashed or skipped everything. Do
            // not burn the next round; a retry in the same session will
            // not magically succeed.
            return AutofixOutcome {
                rounds_run,
                applied_total,
                residual: current.pr_introduced,
                pre_existing: current.pre_existing,
                exit_reason: ExitReason::ZeroProgress,
            };
        }

        applied_total.extend(result.applied.into_iter());

        // Rereview — this re-runs the orchestrator on the patched tree.
        // A rereview failure is not fatal to the caller; we surface the
        // partial outcome + the reason.
        match rereviewer() {
            Ok(next) => current = next,
            Err(e) => {
                return AutofixOutcome {
                    rounds_run,
                    applied_total,
                    residual: current.pr_introduced,
                    pre_existing: current.pre_existing,
                    exit_reason: ExitReason::RereviewFailed(e.to_string()),
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cas_types::{FindingSeverity, Owner};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn mk(title: &str, line: u32, class: AutofixClass) -> Finding {
        Finding {
            title: title.to_string(),
            severity: FindingSeverity::P2,
            file: "src/lib.rs".to_string(),
            line,
            why_it_matters: "because tests say so".to_string(),
            autofix_class: class,
            owner: Owner::ReviewFixer,
            confidence: 0.9,
            evidence: vec!["let x = foo();".to_string()],
            pre_existing: false,
            suggested_fix: None,
            requires_verification: false,
        }
    }

    fn merged(pr_introduced: Vec<Finding>) -> MergedFindings {
        MergedFindings {
            pr_introduced,
            pre_existing: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    /// A scripted rereviewer: returns the next [`MergedFindings`] from a
    /// FIFO queue. Panics if the loop consumes more rereviews than
    /// scripted, which makes "called too often" bugs loud.
    fn scripted_rereviewer(
        script: Vec<Result<MergedFindings, MergeError>>,
    ) -> (
        impl FnMut() -> Result<MergedFindings, MergeError>,
        Rc<RefCell<usize>>,
    ) {
        let state = Rc::new(RefCell::new(script));
        let count = Rc::new(RefCell::new(0usize));
        let c2 = count.clone();
        let f = move || {
            let mut s = state.borrow_mut();
            *c2.borrow_mut() += 1;
            if s.is_empty() {
                panic!("rereviewer called more times than scripted");
            }
            s.remove(0)
        };
        (f, count)
    }

    // --- Scenario 1 --------------------------------------------------
    //
    // Round 1 fixes all safe_auto findings; round 2 rereview returns
    // empty pr_introduced. Loop exits after round 2's rereview with
    // `NothingToFix` on the *next* iteration's empty-safe_auto check.
    #[test]
    fn round_1_fixes_everything_round_2_sees_empty() {
        let initial = merged(vec![
            mk("needs fix a", 10, AutofixClass::SafeAuto),
            mk("needs fix b", 20, AutofixClass::SafeAuto),
        ]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: safe.to_vec(),
            skipped: Vec::new(),
            crashed: false,
        };
        let (rereviewer, _) = scripted_rereviewer(vec![Ok(merged(Vec::new()))]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 2);
        assert!(out.residual.is_empty());
        assert_eq!(out.exit_reason, ExitReason::NothingToFix);
    }

    // --- Scenario 2 --------------------------------------------------
    //
    // Round 1 fixes 3 of 5 (2 skipped). Rereview returns the 2 skipped
    // ones unchanged + nothing else. Round 2 sees no safe_auto because
    // the leftovers are non-safe_auto, so it exits with NothingToFix,
    // residual = 2.
    #[test]
    fn partial_fix_leaves_non_safe_auto_residual() {
        let initial = merged(vec![
            mk("a", 10, AutofixClass::SafeAuto),
            mk("b", 20, AutofixClass::SafeAuto),
            mk("c", 30, AutofixClass::SafeAuto),
            mk("d", 40, AutofixClass::Manual),
            mk("e", 50, AutofixClass::Manual),
        ]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: safe.to_vec(),
            skipped: Vec::new(),
            crashed: false,
        };
        let (rereviewer, _) = scripted_rereviewer(vec![Ok(merged(vec![
            mk("d", 40, AutofixClass::Manual),
            mk("e", 50, AutofixClass::Manual),
        ]))]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 3);
        assert_eq!(out.residual.len(), 2);
        assert_eq!(out.exit_reason, ExitReason::NothingToFix);
    }

    // --- Scenario 3 --------------------------------------------------
    //
    // Round 1 fixer returns zero applied (skipped everything). Loop
    // must NOT burn round 2 — fast exit with ZeroProgress.
    #[test]
    fn zero_progress_exits_after_one_round() {
        let initial = merged(vec![
            mk("a", 10, AutofixClass::SafeAuto),
            mk("b", 20, AutofixClass::SafeAuto),
        ]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: Vec::new(),
            skipped: safe
                .iter()
                .map(|f: &Finding| (f.clone(), "ambiguous".to_string()))
                .collect(),
            crashed: false,
        };

        // Scripted with zero rereviews — if the loop tries to rereview
        // after a zero-progress round, the helper panics and the test
        // fails loudly.
        let (rereviewer, count) = scripted_rereviewer(Vec::new());

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 0);
        assert_eq!(out.residual.len(), 2);
        assert_eq!(out.exit_reason, ExitReason::ZeroProgress);
        assert_eq!(*count.borrow(), 0, "rereviewer must not be called");
    }

    // --- Scenario 4 --------------------------------------------------
    //
    // Round 1 fix introduces a cascade finding; round 2 fixes the
    // cascade; rereview sees zero. Loop exits cleanly with
    // NothingToFix.
    #[test]
    fn round_2_catches_and_fixes_cascade_finding() {
        let initial = merged(vec![mk("original", 10, AutofixClass::SafeAuto)]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: safe.to_vec(),
            skipped: Vec::new(),
            crashed: false,
        };
        let (rereviewer, count) = scripted_rereviewer(vec![
            // After round 1: a new cascade finding surfaced.
            Ok(merged(vec![mk("cascade", 15, AutofixClass::SafeAuto)])),
            // After round 2: clean.
            Ok(merged(Vec::new())),
        ]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 2);
        assert_eq!(out.applied_total.len(), 2);
        assert!(out.residual.is_empty());
        assert_eq!(out.exit_reason, ExitReason::NothingToFix);
        assert_eq!(*count.borrow(), 2);
    }

    // --- Scenario 5 --------------------------------------------------
    //
    // Round 1 fix introduces a cascade the fixer cannot handle (it's
    // manual). Round 2 sees no safe_auto → NothingToFix → residual =
    // the manual cascade.
    #[test]
    fn round_2_sees_unfixable_cascade() {
        let initial = merged(vec![mk("original", 10, AutofixClass::SafeAuto)]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: safe.to_vec(),
            skipped: Vec::new(),
            crashed: false,
        };
        let (rereviewer, _) = scripted_rereviewer(vec![Ok(merged(vec![mk(
            "cascade-manual",
            15,
            AutofixClass::Manual,
        )]))]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 1);
        assert_eq!(out.residual.len(), 1);
        assert_eq!(out.residual[0].title, "cascade-manual");
        assert_eq!(out.exit_reason, ExitReason::NothingToFix);
    }

    // --- Scenario 6 --------------------------------------------------
    //
    // Hard-cap invariant: round 2 introduces yet more findings that
    // round 3 could fix. There is no round 3 — loop exits with HardCap.
    // This is the test that guards the core invariant of the module.
    #[test]
    fn hard_cap_never_runs_round_3() {
        let initial = merged(vec![mk("a", 10, AutofixClass::SafeAuto)]);

        let call_count = Rc::new(RefCell::new(0u8));
        let c2 = call_count.clone();
        let fixer = move |safe: &[Finding]| {
            *c2.borrow_mut() += 1;
            FixerResult {
                applied: safe.to_vec(),
                skipped: Vec::new(),
                crashed: false,
            }
        };
        let (rereviewer, rrc) = scripted_rereviewer(vec![
            // After round 1: another safe_auto appears.
            Ok(merged(vec![mk("b", 20, AutofixClass::SafeAuto)])),
            // After round 2: yet ANOTHER safe_auto appears. Loop must
            // NOT fix it — it must hit HardCap and exit.
            Ok(merged(vec![mk("c", 30, AutofixClass::SafeAuto)])),
        ]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 2, "must cap at MAX_ROUNDS=2");
        assert_eq!(*call_count.borrow(), 2, "fixer called exactly twice");
        assert_eq!(*rrc.borrow(), 2, "rereviewer called exactly twice");
        assert_eq!(out.applied_total.len(), 2);
        assert_eq!(out.residual.len(), 1);
        assert_eq!(out.residual[0].title, "c");
        assert_eq!(out.exit_reason, ExitReason::HardCap);
    }

    // --- Scenario 7 --------------------------------------------------
    //
    // Fixer crash: loop treats the round as zero progress and exits
    // without running round 2.
    #[test]
    fn fixer_crash_exits_early() {
        let initial = merged(vec![mk("a", 10, AutofixClass::SafeAuto)]);

        let fixer = |_: &[Finding]| FixerResult {
            applied: Vec::new(),
            skipped: Vec::new(),
            crashed: true,
        };
        let (rereviewer, count) = scripted_rereviewer(Vec::new());

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 0);
        assert_eq!(out.residual.len(), 1);
        assert_eq!(out.exit_reason, ExitReason::ZeroProgress);
        assert_eq!(*count.borrow(), 0);
    }

    // --- Additional edge: no safe_auto in initial input -------------
    //
    // The loop must exit at round 0 with NothingToFix, not burn a
    // round calling the fixer with an empty list.
    #[test]
    fn empty_initial_safe_auto_is_zero_rounds() {
        let initial = merged(vec![
            mk("a", 10, AutofixClass::Manual),
            mk("b", 20, AutofixClass::GatedAuto),
        ]);

        let fixer = |_: &[Finding]| panic!("fixer must not be called");
        let (rereviewer, _) = scripted_rereviewer(Vec::new());

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 0);
        assert_eq!(out.applied_total.len(), 0);
        assert_eq!(out.residual.len(), 2);
        assert_eq!(out.exit_reason, ExitReason::NothingToFix);
    }

    // --- Additional edge: rereview failure --------------------------
    //
    // Rereview returns an error after round 1. Loop returns the
    // partial outcome with exit_reason = RereviewFailed.
    #[test]
    fn rereview_failure_surfaces_as_exit_reason() {
        let initial = merged(vec![mk("a", 10, AutofixClass::SafeAuto)]);

        let fixer = |safe: &[Finding]| FixerResult {
            applied: safe.to_vec(),
            skipped: Vec::new(),
            crashed: false,
        };
        let (rereviewer, _) = scripted_rereviewer(vec![Err(
            MergeError::InvalidReviewerOutput {
                reviewer: "security".to_string(),
                reason: "bad json".to_string(),
            },
        )]);

        let out = autofix_loop(initial, fixer, rereviewer);

        assert_eq!(out.rounds_run, 1);
        assert_eq!(out.applied_total.len(), 1);
        match out.exit_reason {
            ExitReason::RereviewFailed(s) => {
                assert!(s.contains("security"), "reason should carry the error: {s}");
            }
            other => panic!("expected RereviewFailed, got {other:?}"),
        }
    }

    #[test]
    fn fixer_result_made_progress_contract() {
        assert!(!FixerResult::default().made_progress());
        assert!(
            !FixerResult {
                crashed: true,
                applied: vec![mk("x", 1, AutofixClass::SafeAuto)],
                ..Default::default()
            }
            .made_progress()
        );
        assert!(
            FixerResult {
                applied: vec![mk("x", 1, AutofixClass::SafeAuto)],
                ..Default::default()
            }
            .made_progress()
        );
    }
}
