//! Integration tests for the push rehome guard (AC6) and truthful push summary
//! display (AC5) introduced by cas-9bc5.
//!
//! AC5 — no fixed boilerplate in push summary:
//!   `cas cloud push` must only print insert/update lines for entity types that
//!   were actually in the push payload; it must always show the Tasks line when
//!   tasks were pushed (even if the server returned 0+0).
//!
//! AC6 — slug re-homing guard:
//!   `cas cloud push` must refuse when the `project_canonical_id` has changed
//!   since the last successful push, unless `--rehome` is explicitly passed.
//!   A changed slug causes the cloud server to re-home ALL existing entities
//!   into the new bucket (defect D from the ozer cloud-sync bug report).
//!
//! These tests exercise `check_canonical_id_rehome` directly (no HTTP traffic)
//! so they are fast and require no wiremock server. The guard is extracted as
//! `pub #[doc(hidden)]` from `cas::cli::cloud` following the same pattern as
//! `execute_team_push` / `execute_team_pull`.

use cas::cli::cloud::check_canonical_id_rehome;
use cas::cloud::SyncQueue;
use tempfile::TempDir;

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Open a fresh, initialized SyncQueue in a temp directory.
fn fresh_queue() -> (TempDir, SyncQueue) {
    let tmp = TempDir::new().unwrap();
    let queue = SyncQueue::open(tmp.path()).unwrap();
    queue.init().unwrap();
    (tmp, queue)
}

// ─── AC6: Rehome guard ───────────────────────────────────────────────────────

/// First push ever: no stored canonical_id → push is always allowed, regardless
/// of the `rehome` flag. The guard has nothing to compare against.
#[test]
fn first_push_no_stored_id_is_allowed_without_rehome() {
    let (_tmp, queue) = fresh_queue();
    // Precondition: no metadata exists
    assert!(
        queue.get_metadata("last_push_canonical_id").unwrap().is_none(),
        "precondition: no canonical_id stored yet"
    );
    assert!(
        check_canonical_id_rehome(&queue, "my-project", false).is_ok(),
        "first push without --rehome must be allowed (nothing to compare against)"
    );
}

/// First push with --rehome set: still allowed (redundant flag, but not an error).
#[test]
fn first_push_no_stored_id_is_allowed_with_rehome() {
    let (_tmp, queue) = fresh_queue();
    assert!(
        check_canonical_id_rehome(&queue, "my-project", true).is_ok(),
        "first push with --rehome must also be allowed"
    );
}

/// Steady state: the stored canonical_id matches the current one → push is safe.
#[test]
fn unchanged_canonical_id_is_allowed() {
    let (_tmp, queue) = fresh_queue();
    queue
        .set_metadata("last_push_canonical_id", "ozer")
        .unwrap();
    assert!(
        check_canonical_id_rehome(&queue, "ozer", false).is_ok(),
        "push with unchanged canonical_id must be allowed"
    );
}

/// Changed canonical_id without --rehome → guard must refuse.
///
/// This is the core protection against defect D: a user who ran
/// `cas cloud project set github.com/org/repo` would otherwise silently
/// re-home ~17k entities on the next push.
#[test]
fn changed_canonical_id_refused_without_rehome() {
    let (_tmp, queue) = fresh_queue();
    queue
        .set_metadata("last_push_canonical_id", "ozer")
        .unwrap();

    let result = check_canonical_id_rehome(&queue, "github.com/Richards-LLC/ozer-health", false);
    assert!(
        result.is_err(),
        "push with changed canonical_id and no --rehome must be refused"
    );
    let msg = result.unwrap_err();
    // The error must explain the re-home risk and how to proceed.
    assert!(
        msg.contains("--rehome"),
        "refusal message must mention --rehome: {msg}"
    );
    assert!(
        msg.contains("ozer"),
        "refusal message must name the stored slug: {msg}"
    );
    assert!(
        msg.contains("github.com/Richards-LLC/ozer-health"),
        "refusal message must name the new slug: {msg}"
    );
}

/// Changed canonical_id WITH --rehome → user explicitly confirmed, push is allowed.
#[test]
fn changed_canonical_id_allowed_with_rehome() {
    let (_tmp, queue) = fresh_queue();
    queue
        .set_metadata("last_push_canonical_id", "ozer")
        .unwrap();
    assert!(
        check_canonical_id_rehome(&queue, "github.com/Richards-LLC/ozer-health", true).is_ok(),
        "push with changed canonical_id AND --rehome must be allowed"
    );
}

/// Guard is symmetric: after a re-home to slug B, a push back to A (re-re-home)
/// is also refused without --rehome. The stored value is the LAST recorded push,
/// not the original.
#[test]
fn rehome_back_to_original_also_refused_without_flag() {
    let (_tmp, queue) = fresh_queue();
    // Simulate: user was at "ozer", re-homed to "github.com/...", and the queue
    // now records the new slug (as `execute_push` would do after success).
    queue
        .set_metadata(
            "last_push_canonical_id",
            "github.com/Richards-LLC/ozer-health",
        )
        .unwrap();

    let result = check_canonical_id_rehome(&queue, "ozer", false);
    assert!(
        result.is_err(),
        "re-home back to previous slug is still a re-home — must be refused without --rehome"
    );
}

/// Guard is stable across multiple pushes with the same slug: repeated calls
/// with the same project_id always return Ok (no false positives after the
/// first push records the canonical_id).
#[test]
fn repeated_pushes_same_slug_never_refused() {
    let (_tmp, queue) = fresh_queue();
    // Simulate the state after a first successful push.
    queue
        .set_metadata("last_push_canonical_id", "ozer")
        .unwrap();

    for _ in 0..5 {
        assert!(
            check_canonical_id_rehome(&queue, "ozer", false).is_ok(),
            "repeated pushes with the same slug must always be allowed"
        );
    }
}
