//! Code review support utilities (Phase 1 Subsystem A).
//!
//! This module hosts helpers used by the multi-persona code-review pipeline
//! that is being built out in the Phase 1 roadmap. The first component is
//! [`base_sha::resolve`], a fork-safe helper for picking the base revision
//! against which a review diff should be computed.

pub mod autofix;
pub mod base_sha;
pub mod close_gate;
pub mod merge;
pub mod review_to_task;

pub use autofix::{AutofixOutcome, ExitReason, FixerResult, MAX_ROUNDS, autofix_loop};
pub use close_gate::{GateDecision, evaluate_gate, format_block_message};
pub use base_sha::{BaseShaError, resolve as resolve_base_sha};
pub use merge::{
    AGREEMENT_BOOST, CONFIDENCE_GATE, LINE_BUCKET_RADIUS, MergeDiagnostic, MergeError,
    MergedFindings, P0_CONFIDENCE_FLOOR, merge_findings,
};
pub use review_to_task::{
    REVIEW_LABEL, RouteAction, RouteError, RouteOutcome, SkipReason, SOURCE_FOOTER, TaskDraft,
    build_draft, external_ref_for, map_severity, map_task_type, route_residual_to_tasks,
};
