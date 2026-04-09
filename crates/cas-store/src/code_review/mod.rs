//! Code review support utilities (Phase 1 Subsystem A).
//!
//! This module hosts helpers used by the multi-persona code-review pipeline
//! that is being built out in the Phase 1 roadmap. The first component is
//! [`base_sha::resolve`], a fork-safe helper for picking the base revision
//! against which a review diff should be computed.

pub mod base_sha;

pub use base_sha::{BaseShaError, resolve as resolve_base_sha};
