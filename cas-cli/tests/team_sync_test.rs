//! Integration tests for `cas cloud sync`'s team-queue drain path.
//!
//! Exercises `cas::cli::cloud::execute_team_push` — the helper wired into
//! `execute_sync` by cas-1f44 (T4) that drains the team sync queue into
//! `POST /api/teams/{uuid}/sync/push` when a team is configured.
//!
//! Coverage:
//! - Happy path: team configured + queued items → POST fires, queue drained.
//! - No team configured → early return, zero HTTP requests.
//! - `team_auto_promote=Some(false)` kill-switch → suppresses push even
//!   when team_id is set.
//! - Empty queue with team configured → silent early return.
//! - HTTP 500 failure → `push_team` re-enqueues items, helper still
//!   returns `Ok(())` (isolation contract — personal push and pull must
//!   not be blocked by team push errors).
//!
//! Lives in `cas-cli/tests/` (integration-test tree) rather than
//! co-located with the impl because the verifier flagged the inline
//! `#[cfg(test)] mod` as a test-first posture concern — tests are easier
//! to find in the integration tree than buried in a 2400-line impl file.

mod common;
use common::{TEST_TEAM, make_cli_json, make_cloud_config};

use cas::cli::cloud::execute_team_push;
use cas::cloud::{CloudConfig, EntityType, SyncOperation, SyncQueue};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Create a `.cas`-style directory and seed the sync queue with one
/// team-tagged entry upsert, returning the TempDir owning the files.
fn make_cas_root_with_team_item() -> TempDir {
    let tmp = TempDir::new().unwrap();
    let queue = SyncQueue::open(tmp.path()).unwrap();
    queue.init().unwrap();
    queue
        .enqueue_for_team(
            EntityType::Entry,
            "p-test-001",
            SyncOperation::Upsert,
            Some(r#"{"id":"p-test-001","scope":"project","content":"hi"}"#),
            TEST_TEAM,
        )
        .unwrap();
    tmp
}

/// Happy path: team configured + queued items → POST fires against
/// `/api/teams/{uuid}/sync/push`, queue is drained.
///
/// NOTE: server contract includes `project_canonical_id` in the payload
/// so the server can auto-register the project. The payload is
/// gzip-compressed before send so wiremock body matchers can't cheaply
/// verify the field; that contract is covered by the push_team unit
/// tests in `cas-cli/src/cloud/syncer/team_push.rs`.
#[tokio::test]
async fn team_push_drains_queue_when_team_configured() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/teams/{TEST_TEAM}/sync/push")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "synced": {
                    "entries": 1,
                    "tasks": 0, "rules": 0, "skills": 0,
                    "sessions": 0, "verifications": 0, "events": 0,
                    "prompts": 0, "file_changes": 0, "commit_links": 0,
                    "agents": 0, "worktrees": 0,
                }
            })),
        )
        .expect(1)
        .mount(&server)
        .await;

    let cfg = make_cloud_config(server.uri());
    let tmp = make_cas_root_with_team_item();
    let cas_root = tmp.path().to_path_buf();
    let cli = make_cli_json();

    // `execute_team_push` is sync and uses `ureq`; run on the blocking
    // pool so the wiremock tokio runtime can serve the request.
    let result =
        tokio::task::spawn_blocking(move || execute_team_push(&cfg, &cas_root, &cli))
            .await
            .unwrap();

    assert!(result.is_ok(), "execute_team_push returned Err: {result:?}");

    let queue = SyncQueue::open(tmp.path()).unwrap();
    let remaining = queue.pending_for_team(TEST_TEAM, 100, 5).unwrap();
    assert_eq!(remaining.len(), 0, "team queue should be drained");
}

/// No team configured → early return, zero HTTP traffic, `Ok(())`.
#[tokio::test]
async fn team_push_no_op_when_no_team_configured() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let mut cfg = CloudConfig::default();
    cfg.endpoint = server.uri();
    cfg.token = Some("test-token".to_string());
    // Deliberately no set_team — active_team_id() returns None.

    let tmp = TempDir::new().unwrap();
    let cas_root = tmp.path().to_path_buf();
    let cli = make_cli_json();

    let result =
        tokio::task::spawn_blocking(move || execute_team_push(&cfg, &cas_root, &cli))
            .await
            .unwrap();
    assert!(result.is_ok());
}

/// Kill-switch: `team_auto_promote=Some(false)` must suppress the push
/// exactly like no-team-configured does — even when team_id is set.
#[tokio::test]
async fn team_push_suppressed_by_auto_promote_kill_switch() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let mut cfg = make_cloud_config(server.uri());
    cfg.team_auto_promote = Some(false);

    let tmp = make_cas_root_with_team_item();
    let cas_root = tmp.path().to_path_buf();
    let cli = make_cli_json();

    let result =
        tokio::task::spawn_blocking(move || execute_team_push(&cfg, &cas_root, &cli))
            .await
            .unwrap();
    assert!(result.is_ok());
}

/// Empty team queue + team configured (steady state after a full sync)
/// → no output noise, no error, zero HTTP.
#[tokio::test]
async fn team_push_silent_when_queue_empty() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let cfg = make_cloud_config(server.uri());
    let tmp = TempDir::new().unwrap();
    let queue = SyncQueue::open(tmp.path()).unwrap();
    queue.init().unwrap();
    // Deliberately no enqueue_for_team — queue is empty.
    let cas_root = tmp.path().to_path_buf();
    let cli = make_cli_json();

    let result =
        tokio::task::spawn_blocking(move || execute_team_push(&cfg, &cas_root, &cli))
            .await
            .unwrap();
    assert!(result.is_ok());
}

/// Isolation contract: team push HTTP failure must not block the caller's
/// pull step. `execute_team_push` returns `Ok(())` even on 5xx; push_team
/// re-enqueues the drained items internally so they survive the failed
/// attempt for the next sync cycle.
#[tokio::test]
async fn team_push_http_failure_is_isolated() {
    let server = MockServer::start().await;
    // push_team retries 3 times internally on 5xx.
    Mock::given(method("POST"))
        .and(path(format!("/api/teams/{TEST_TEAM}/sync/push")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let cfg = make_cloud_config(server.uri());
    let tmp = make_cas_root_with_team_item();
    let cas_root = tmp.path().to_path_buf();
    let cli = make_cli_json();

    let result =
        tokio::task::spawn_blocking(move || execute_team_push(&cfg, &cas_root, &cli))
            .await
            .unwrap();
    assert!(
        result.is_ok(),
        "helper must return Ok even when team push fails (partial-failure isolation): {result:?}"
    );

    // Item re-enqueued by push_team on error — still in queue for next sync.
    let queue = SyncQueue::open(tmp.path()).unwrap();
    let remaining = queue.pending_for_team(TEST_TEAM, 100, 5).unwrap();
    assert_eq!(
        remaining.len(),
        1,
        "team items re-enqueued on http failure (preserves data for next sync)"
    );
}
