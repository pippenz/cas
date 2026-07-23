use crate::hooks::handlers::*;

// =========================================================================
// detect_and_mark_skill_drift tests (cas-f9ad)
// =========================================================================

/// Drift detected when sentinel exists but session marker is absent → returns true
/// and writes the marker so the next call for the same session returns false.
#[test]
fn skill_drift_no_marker_returns_true() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:111").unwrap();

    let result = detect_and_mark_skill_drift(cas_root, "sess-drift-a");
    assert!(result, "should detect drift when no marker exists");

    // Marker should now exist with the sentinel's content
    let marker = cas_root.join("session_skills_seen_sess-drift-a");
    assert!(marker.exists(), "marker must be created after drift detected");
    assert_eq!(
        std::fs::read_to_string(&marker).unwrap(),
        "ts:111",
        "marker must echo sentinel content"
    );
}

/// No drift when marker content matches sentinel content → returns false.
#[test]
fn skill_drift_marker_matches_returns_false() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:222").unwrap();
    std::fs::write(
        cas_root.join("session_skills_seen_sess-drift-b"),
        b"ts:222",
    )
    .unwrap();

    let result = detect_and_mark_skill_drift(cas_root, "sess-drift-b");
    assert!(!result, "no drift when marker matches sentinel");
}

/// No sentinel file → always false (sync has never run).
#[test]
fn skill_drift_no_sentinel_returns_false() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    let result = detect_and_mark_skill_drift(cas_root, "sess-drift-c");
    assert!(!result, "no drift when sentinel absent");
}

#[test]
fn skill_drift_empty_session_id_does_not_create_bare_marker() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();
    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:empty").unwrap();

    assert!(!detect_and_mark_skill_drift(cas_root, ""));
    assert!(
        !cas_root.join("session_skills_seen_").exists(),
        "empty session ids must never create a bare marker"
    );
}

/// After drift is detected and marker written, a second call returns false
/// (idempotent within a session lifecycle).
#[test]
fn skill_drift_second_call_returns_false() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:333").unwrap();

    let first = detect_and_mark_skill_drift(cas_root, "sess-drift-d");
    assert!(first, "first call must detect drift");

    let second = detect_and_mark_skill_drift(cas_root, "sess-drift-d");
    assert!(!second, "second call must NOT detect drift (marker updated)");
}

/// Different sessions track independently: one session acking drift does NOT
/// suppress it for another session that hasn't seen the sentinel yet.
#[test]
fn skill_drift_independent_per_session() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:444").unwrap();

    let _ = detect_and_mark_skill_drift(cas_root, "sess-drift-e1");
    // Session e2 has its own marker → should still see drift
    let e2 = detect_and_mark_skill_drift(cas_root, "sess-drift-e2");
    assert!(e2, "session e2 must detect drift independently of e1");
}

/// After new sync (sentinel changes), drift is re-detected even for a session
/// that previously acked.
#[test]
fn skill_drift_re_detected_after_new_sync() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let cas_root = tmp.path();

    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:555").unwrap();
    let _ = detect_and_mark_skill_drift(cas_root, "sess-drift-f");

    // Simulate new sync
    std::fs::write(cas_root.join("skill_sync_sentinel"), b"ts:666").unwrap();

    let again = detect_and_mark_skill_drift(cas_root, "sess-drift-f");
    assert!(again, "drift must be re-detected after sentinel updated by new sync");
}

// =========================================================================
// reloadSkills JSON serialization tests (cas-f9ad)
// =========================================================================

/// `with_reload_skills(true)` on an existing SessionStart output adds
/// `"reloadSkills":true` to the JSON wire shape.
#[test]
fn reload_skills_true_serializes() {
    let output = HookOutput::with_session_start_context("ctx".into()).with_reload_skills(true);
    let json = serde_json::to_string(&output).unwrap();
    assert!(
        json.contains("\"reloadSkills\":true"),
        "Expected reloadSkills:true in: {json}"
    );
    assert!(
        json.contains("\"additionalContext\":\"ctx\""),
        "additionalContext must still be present: {json}"
    );
}

/// `with_reload_skills(true)` on an empty output creates a minimal SessionStart
/// output with an empty additionalContext.
#[test]
fn reload_skills_on_empty_output_creates_session_start() {
    let output = HookOutput::empty().with_reload_skills(true);
    let json = serde_json::to_string(&output).unwrap();
    assert!(
        json.contains("\"reloadSkills\":true"),
        "Expected reloadSkills:true in: {json}"
    );
    assert!(
        json.contains("SessionStart"),
        "Expected SessionStart hookEventName: {json}"
    );
}

/// When `reload_skills` is `None` (default), the field is absent from JSON.
#[test]
fn reload_skills_absent_by_default() {
    let output = HookOutput::with_session_start_context("ctx".into());
    let json = serde_json::to_string(&output).unwrap();
    assert!(
        !json.contains("reloadSkills"),
        "reloadSkills must be absent when not set: {json}"
    );
}
